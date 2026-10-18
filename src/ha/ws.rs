//! Live updates over Home Assistant's WebSocket API.
//!
//! `subscribe_entities` sends the full state of the requested entities once,
//! then only what changes, in a compressed form:
//! `{"a": {id: {"s", "a", ...}}}` added, `{"c": {id: {"+": {...}, "-": {"a": [...]}}}}`
//! changed and `{"r": [id]}` removed. This replaces polling `/api/states`
//! (hundreds of entities) every interval, so the Kobo only wakes its radio and
//! redraws when something on screen actually changes.

use crate::ha::client::HaClient;
use crate::ha::models::EntityState;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

const SUBSCRIPTION_ID: u64 = 1;
/// Ping rarely: every ping wakes the Wi-Fi radio.
const PING_EVERY: Duration = Duration::from_secs(240);
const PONG_TIMEOUT: Duration = Duration::from_secs(30);

type BoxError = Box<dyn std::error::Error + Send + Sync>;

fn ws_url(base: &str) -> Option<String> {
    let base = base.trim().trim_end_matches('/');
    let rest = base.strip_prefix("https://").map(|r| format!("wss://{}", r)).or_else(|| base.strip_prefix("http://").map(|r| format!("ws://{}", r)))?;
    Some(format!("{}/api/websocket", rest))
}

impl HaClient {
    /// Keep a live subscription to the dashboard's entities, reconnecting with
    /// backoff. `notify` gets a message after every change.
    pub fn spawn_live_updates(&self, notify: mpsc::Sender<()>) {
        if self.config().token.trim().is_empty() || self.entity_ids.is_empty() {
            return;
        }
        let me = self.clone();
        tokio::spawn(async move {
            let mut backoff = Duration::from_secs(5);
            loop {
                let started = Instant::now();
                match me.run_subscription(&notify).await {
                    Ok(()) => tracing::info!("Home Assistant WebSocket closed"),
                    Err(e) => tracing::warn!("Home Assistant WebSocket: {}", e),
                }
                me.ws_live.store(false, Ordering::Relaxed);
                if started.elapsed() > Duration::from_secs(120) {
                    backoff = Duration::from_secs(5);
                }
                tokio::select! {
                    _ = tokio::time::sleep(backoff) => {}
                    _ = me.ws_nudge.notified() => backoff = Duration::from_secs(5),
                }
                backoff = (backoff * 2).min(Duration::from_secs(120));
            }
        });
    }

    async fn run_subscription(&self, notify: &mpsc::Sender<()>) -> Result<(), BoxError> {
        let url = ws_url(&self.config().url).ok_or("Home Assistant URL must start with http:// or https://")?;
        let (mut ws, _) = tokio::time::timeout(Duration::from_secs(15), tokio_tungstenite::connect_async(url.as_str()))
            .await
            .map_err(|_| "connection timed out")??;

        // Handshake: auth_required -> auth -> auth_ok
        loop {
            let msg = next_json(&mut ws).await?;
            match msg.get("type").and_then(|t| t.as_str()) {
                Some("auth_required") => {
                    let auth = json!({ "type": "auth", "access_token": self.config().token.trim() });
                    ws.send(Message::Text(auth.to_string())).await?;
                }
                Some("auth_ok") => break,
                Some("auth_invalid") => return Err("the access token was rejected".into()),
                _ => {}
            }
        }

        let sub = json!({ "id": SUBSCRIPTION_ID, "type": "subscribe_entities", "entity_ids": *self.entity_ids });
        ws.send(Message::Text(sub.to_string())).await?;
        tracing::info!("Subscribed to {} entities over WebSocket", self.entity_ids.len());

        let mut next_id = SUBSCRIPTION_ID + 1;
        let mut ping = tokio::time::interval(PING_EVERY);
        ping.tick().await;
        let mut waiting_since: Option<Instant> = None;

        loop {
            tokio::select! {
                msg = ws.next() => {
                    let text = match msg {
                        Some(Ok(Message::Text(t))) => t,
                        Some(Ok(Message::Ping(p))) => { ws.send(Message::Pong(p)).await?; continue; }
                        Some(Ok(Message::Close(_))) | None => return Ok(()),
                        Some(Ok(_)) => continue,
                        Some(Err(e)) => return Err(e.into()),
                    };
                    let msg: Value = serde_json::from_str(&text)?;
                    match msg.get("type").and_then(|t| t.as_str()) {
                        Some("event") if msg.get("id").and_then(|i| i.as_u64()) == Some(SUBSCRIPTION_ID) => {
                            if let Some(event) = msg.get("event") {
                                self.apply_compressed(event).await;
                                self.ws_live.store(true, Ordering::Relaxed);
                                self.set_live(true).await;
                                let _ = notify.try_send(());
                            }
                        }
                        Some("result") if msg.get("id").and_then(|i| i.as_u64()) == Some(SUBSCRIPTION_ID) => {
                            if msg.get("success").and_then(|s| s.as_bool()) != Some(true) {
                                return Err(format!("subscribe_entities failed: {}", msg.get("error").unwrap_or(&Value::Null)).into());
                            }
                        }
                        Some("pong") => waiting_since = None,
                        _ => {}
                    }
                }
                _ = ping.tick() => {
                    if waiting_since.map(|t| t.elapsed() > PONG_TIMEOUT).unwrap_or(false) {
                        return Err("no answer to ping".into());
                    }
                    ws.send(Message::Text(json!({ "id": next_id, "type": "ping" }).to_string())).await?;
                    next_id += 1;
                    waiting_since = Some(Instant::now());
                }
            }
        }
    }

    /// Apply one `subscribe_entities` event to the cached states.
    pub(crate) async fn apply_compressed(&self, event: &Value) {
        if let Some(added) = event.get("a").and_then(|a| a.as_object()) {
            for (id, compressed) in added {
                let mut state = EntityState {
                    entity_id: id.clone(),
                    state: compressed.get("s").map(value_to_state).unwrap_or_default(),
                    attributes: compressed
                        .get("a")
                        .and_then(|a| a.as_object())
                        .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                        .unwrap_or_default(),
                    last_changed: None,
                    last_updated: None,
                };
                self.decorate(&mut state).await;
                self.states_handle().write().await.insert(id.clone(), state);
            }
        }

        if let Some(changed) = event.get("c").and_then(|c| c.as_object()) {
            let mut states = self.states_handle().write().await;
            for (id, diff) in changed {
                let Some(state) = states.get_mut(id) else { continue };
                if let Some(plus) = diff.get("+") {
                    if let Some(s) = plus.get("s") {
                        state.state = value_to_state(s);
                    }
                    if let Some(attrs) = plus.get("a").and_then(|a| a.as_object()) {
                        for (k, v) in attrs {
                            state.attributes.insert(k.clone(), v.clone());
                        }
                    }
                }
                if let Some(removed) = diff.get("-").and_then(|m| m.get("a")).and_then(|a| a.as_array()) {
                    for key in removed.iter().filter_map(|k| k.as_str()) {
                        state.attributes.remove(key);
                    }
                }
            }
        }

        if let Some(removed) = event.get("r").and_then(|r| r.as_array()) {
            let mut states = self.states_handle().write().await;
            for id in removed.iter().filter_map(|i| i.as_str()) {
                states.remove(id);
            }
        }
    }
}

fn value_to_state(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

async fn next_json<S>(ws: &mut S) -> Result<Value, BoxError>
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    loop {
        match tokio::time::timeout(Duration::from_secs(15), ws.next()).await.map_err(|_| "no answer during login")? {
            Some(Ok(Message::Text(t))) => return Ok(serde_json::from_str(&t)?),
            Some(Ok(Message::Close(_))) | None => return Err("connection closed during login".into()),
            Some(Ok(_)) => continue,
            Some(Err(e)) => return Err(e.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::HomeAssistantConfig;

    #[test]
    fn websocket_url() {
        assert_eq!(ws_url("http://192.168.1.10:8123/").as_deref(), Some("ws://192.168.1.10:8123/api/websocket"));
        assert_eq!(ws_url("https://ha.example.com").as_deref(), Some("wss://ha.example.com/api/websocket"));
        assert_eq!(ws_url("ha.local"), None);
    }

    #[tokio::test]
    async fn applies_added_changed_and_removed() {
        let client = HaClient::new(HomeAssistantConfig { token: "t".into(), demo_mode_fallback: false, ..Default::default() });
        client
            .apply_compressed(&json!({"a": {
                "light.desk": {"s": "off", "a": {"friendly_name": "Desk", "brightness": 10}, "lc": 1.0},
                "sensor.t": {"s": 21.5, "a": {}}
            }}))
            .await;
        client
            .apply_compressed(&json!({"c": {"light.desk": {"+": {"s": "on", "a": {"brightness": 200}}, "-": {"a": ["friendly_name"]}}}}))
            .await;
        client.apply_compressed(&json!({"r": ["sensor.t"]})).await;

        let states = client.get_all_states().await;
        let desk = &states["light.desk"];
        assert_eq!(desk.state, "on");
        assert_eq!(desk.attributes["brightness"], json!(200));
        assert!(!desk.attributes.contains_key("friendly_name"));
        assert!(!states.contains_key("sensor.t"));
    }

    /// Manual check against a real Home Assistant:
    /// `HA_URL=http://… HA_TOKEN=… HA_ENTITIES=light.a,sensor.b cargo test live_subscription -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn live_subscription() {
        let (Ok(url), Ok(token), Ok(entities)) = (std::env::var("HA_URL"), std::env::var("HA_TOKEN"), std::env::var("HA_ENTITIES")) else {
            panic!("set HA_URL, HA_TOKEN and HA_ENTITIES");
        };
        let mut client = HaClient::new(HomeAssistantConfig { url, token, demo_mode_fallback: false, ..Default::default() });
        client.entity_ids = std::sync::Arc::new(entities.split(',').map(str::to_string).collect());
        let (tx, mut rx) = mpsc::channel(8);
        client.spawn_live_updates(tx);
        tokio::time::timeout(Duration::from_secs(20), rx.recv()).await.expect("no event within 20 s");
        let states = client.get_all_states().await;
        for (id, s) in &states {
            println!("{} = {} ({} attributes)", id, s.state, s.attributes.len());
        }
        assert_eq!(states.len(), client.entity_ids.len(), "every requested entity arrives");
        assert!(client.is_live_push());
    }
}
