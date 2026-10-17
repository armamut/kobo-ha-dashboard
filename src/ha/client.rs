use crate::config::{AppConfig, CardConfig, HomeAssistantConfig};
use crate::ha::mock::generate_mock_states;
use crate::ha::models::EntityState;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// Attribute key under which recent numeric history is stored for sparklines
pub const HISTORY_ATTR: &str = "__history";

const HISTORY_REFRESH: Duration = Duration::from_secs(300);
const HISTORY_SPAN_HOURS: i64 = 24;
const HISTORY_POINTS: usize = 48;

#[derive(Clone)]
pub struct HaClient {
    config: HomeAssistantConfig,
    client: reqwest::Client,
    states: Arc<RwLock<HashMap<String, EntityState>>>,
    is_live: Arc<RwLock<bool>>,
    /// Weather entities whose forecast must be fetched via weather.get_forecasts
    weather_entities: Arc<Vec<String>>,
    /// Sensors shown with a graph
    history_entities: Arc<Vec<String>>,
    extras: Arc<RwLock<Extras>>,
    last_update: Arc<RwLock<Option<chrono::DateTime<chrono::Utc>>>>,
}

#[derive(Default)]
struct Extras {
    forecasts: HashMap<String, Value>,
    history: HashMap<String, Vec<f32>>,
    last_history_fetch: Option<Instant>,
    last_forecast_fetch: Option<Instant>,
}

impl HaClient {
    pub fn new(config: HomeAssistantConfig) -> Self {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        if !config.token.is_empty() {
            if let Ok(val) = HeaderValue::from_str(&format!("Bearer {}", config.token.trim())) {
                headers.insert(AUTHORIZATION, val);
            }
        }

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .connect_timeout(Duration::from_secs(config.timeout_secs.min(5)))
            .default_headers(headers)
            .build()
            .unwrap_or_default();

        let initial_states = if config.token.is_empty() && config.demo_mode_fallback {
            generate_mock_states()
        } else {
            HashMap::new()
        };

        Self {
            config,
            client,
            states: Arc::new(RwLock::new(initial_states)),
            is_live: Arc::new(RwLock::new(false)),
            weather_entities: Arc::new(Vec::new()),
            history_entities: Arc::new(Vec::new()),
            extras: Arc::new(RwLock::new(Extras::default())),
            last_update: Arc::new(RwLock::new(None)),
        }
    }

    /// Collect entities that need extra API calls (forecasts, history) from the pages.
    pub fn for_app(app: &AppConfig) -> Self {
        let mut client = Self::new(app.home_assistant.clone());
        let mut weather = Vec::new();
        let mut history = Vec::new();
        for card in app.pages.iter().flat_map(|p| p.cards.iter()) {
            match card {
                CardConfig::Weather { entity_id, .. } => weather.push(entity_id.clone()),
                CardConfig::Sensor { entity_id, show_graph: true, .. } => history.push(entity_id.clone()),
                _ => {}
            }
        }
        weather.dedup();
        history.dedup();
        client.weather_entities = Arc::new(weather);
        client.history_entities = Arc::new(history);
        client
    }

    fn is_demo(&self) -> bool {
        self.config.token.is_empty()
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.config.url.trim_end_matches('/'), path)
    }

    pub async fn is_connected(&self) -> bool {
        *self.is_live.read().await
    }

    pub async fn last_update(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        *self.last_update.read().await
    }

    pub async fn get_state(&self, entity_id: &str) -> Option<EntityState> {
        self.states.read().await.get(entity_id).cloned()
    }

    pub async fn get_all_states(&self) -> HashMap<String, EntityState> {
        self.states.read().await.clone()
    }

    pub async fn refresh_states(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if self.is_demo() {
            if self.config.demo_mode_fallback {
                let mut states = self.states.write().await;
                if states.is_empty() {
                    *states = generate_mock_states();
                }
            }
            return Ok(());
        }

        let result = self.fetch_states().await;
        *self.is_live.write().await = result.is_ok();

        match result {
            Ok(raw_states) => {
                self.refresh_extras().await;
                let extras = self.extras.read().await;
                let mut map = self.states.write().await;
                map.clear();
                for mut state in raw_states {
                    if let Some(fc) = extras.forecasts.get(&state.entity_id) {
                        state.attributes.insert("forecast".to_string(), fc.clone());
                    }
                    if let Some(h) = extras.history.get(&state.entity_id) {
                        state.attributes.insert(HISTORY_ATTR.to_string(), serde_json::json!(h));
                    }
                    map.insert(state.entity_id.clone(), state);
                }
                *self.last_update.write().await = Some(chrono::Utc::now());
                tracing::info!("Updated {} entities from Home Assistant", map.len());
                Ok(())
            }
            Err(e) => {
                tracing::warn!("Home Assistant refresh failed: {}", e);
                if self.config.demo_mode_fallback {
                    let mut states = self.states.write().await;
                    if states.is_empty() {
                        *states = generate_mock_states();
                    }
                }
                Err(e)
            }
        }
    }

    async fn fetch_states(&self) -> Result<Vec<EntityState>, Box<dyn std::error::Error + Send + Sync>> {
        let resp = self.client.get(self.url("/api/states")).send().await?;
        if !resp.status().is_success() {
            return Err(format!("HTTP {}", resp.status()).into());
        }
        Ok(resp.json().await?)
    }

    /// Forecasts (HA 2024.3+ removed the `forecast` attribute) and sensor history.
    async fn refresh_extras(&self) {
        let (need_forecast, need_history) = {
            let ex = self.extras.read().await;
            let stale = |t: Option<Instant>| t.map(|t| t.elapsed() > HISTORY_REFRESH).unwrap_or(true);
            (stale(ex.last_forecast_fetch), stale(ex.last_history_fetch))
        };

        if need_forecast && !self.weather_entities.is_empty() {
            let mut forecasts = HashMap::new();
            for entity in self.weather_entities.iter() {
                match self.fetch_forecast(entity).await {
                    Ok(Some(fc)) => {
                        forecasts.insert(entity.clone(), fc);
                    }
                    Ok(None) => {}
                    Err(e) => tracing::warn!("Forecast fetch for {} failed: {}", entity, e),
                }
            }
            let mut ex = self.extras.write().await;
            ex.forecasts = forecasts;
            ex.last_forecast_fetch = Some(Instant::now());
        }

        if need_history && !self.history_entities.is_empty() {
            match self.fetch_history().await {
                Ok(history) => {
                    let mut ex = self.extras.write().await;
                    ex.history = history;
                    ex.last_history_fetch = Some(Instant::now());
                }
                Err(e) => tracing::warn!("History fetch failed: {}", e),
            }
        }
    }

    async fn fetch_forecast(&self, entity_id: &str) -> Result<Option<Value>, Box<dyn std::error::Error + Send + Sync>> {
        let resp = self
            .client
            .post(self.url("/api/services/weather/get_forecasts?return_response"))
            .json(&serde_json::json!({ "entity_id": entity_id, "type": "daily" }))
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(format!("HTTP {}", resp.status()).into());
        }
        let body: Value = resp.json().await?;
        Ok(body
            .pointer(&format!("/service_response/{}/forecast", entity_id.replace('/', "~1")))
            .cloned())
    }

    async fn fetch_history(&self) -> Result<HashMap<String, Vec<f32>>, Box<dyn std::error::Error + Send + Sync>> {
        let start = chrono::Utc::now() - chrono::Duration::hours(HISTORY_SPAN_HOURS);
        let resp = self
            .client
            .get(self.url(&format!("/api/history/period/{}", start.format("%Y-%m-%dT%H:%M:%SZ"))))
            .query(&[
                ("filter_entity_id", self.history_entities.join(",")),
                ("minimal_response", String::new()),
                ("no_attributes", String::new()),
            ])
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(format!("HTTP {}", resp.status()).into());
        }
        let series: Vec<Vec<Value>> = resp.json().await?;
        let mut out = HashMap::new();
        for (idx, points) in series.into_iter().enumerate() {
            // With minimal_response only the first item carries entity_id.
            let entity = points
                .first()
                .and_then(|p| p.get("entity_id"))
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .or_else(|| self.history_entities.get(idx).cloned());
            let values: Vec<f32> = points
                .iter()
                .filter_map(|p| p.get("state")?.as_str()?.parse::<f32>().ok())
                .collect();
            if let Some(entity) = entity {
                out.insert(entity, downsample(&values, HISTORY_POINTS));
            }
        }
        Ok(out)
    }

    /// Update the local cache right away so the screen reflects a tap before HA answers.
    pub async fn apply_optimistic(&self, domain: &str, service: &str, data: &Value) {
        self.apply_mock_service_call(domain, service, data).await;
    }

    pub async fn call_service(
        &self,
        domain: &str,
        service: &str,
        data: Value,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if self.is_demo() {
            self.apply_mock_service_call(domain, service, &data).await;
            return Ok(());
        }

        let resp = self
            .client
            .post(self.url(&format!("/api/services/{}/{}", domain, service)))
            .json(&data)
            .send()
            .await?;
        if resp.status().is_success() {
            tracing::info!("Service call {}.{} succeeded", domain, service);
            // Give HA a moment to settle state before re-reading it
            tokio::time::sleep(Duration::from_millis(300)).await;
            let _ = self.refresh_states().await;
            Ok(())
        } else {
            let status = resp.status();
            tracing::error!("Service call {}.{} failed with status {}", domain, service, status);
            let _ = self.refresh_states().await;
            Err(format!("Service call failed: {}", status).into())
        }
    }

    pub fn toggle_request(&self, entity_id: &str) -> (String, String, Value) {
        let domain = entity_id.split('.').next().unwrap_or("homeassistant");
        let domain = match domain {
            "light" | "switch" | "fan" | "input_boolean" | "automation" | "media_player" | "climate" | "cover" => domain,
            _ => "homeassistant",
        };
        (domain.to_string(), "toggle".to_string(), serde_json::json!({ "entity_id": entity_id }))
    }

    /// Service for a "scene" button: run scenes/scripts, press buttons, toggle the rest.
    pub fn activate_request(&self, entity_id: &str) -> (String, String, Value) {
        let domain = entity_id.split('.').next().unwrap_or("");
        let data = serde_json::json!({ "entity_id": entity_id });
        match domain {
            "scene" | "script" => (domain.to_string(), "turn_on".to_string(), data),
            "button" | "input_button" => (domain.to_string(), "press".to_string(), data),
            _ => self.toggle_request(entity_id),
        }
    }

    pub async fn climate_request(&self, entity_id: &str, delta: f32) -> (String, String, Value) {
        let (current_target, min, max) = {
            let states = self.states.read().await;
            let s = states.get(entity_id);
            (
                s.and_then(|s| s.get_attr_f32("temperature")).unwrap_or(21.0),
                s.and_then(|s| s.get_attr_f32("min_temp")).unwrap_or(5.0),
                s.and_then(|s| s.get_attr_f32("max_temp")).unwrap_or(35.0),
            )
        };
        let new_temp = (((current_target + delta) * 2.0).round() / 2.0).clamp(min, max);
        (
            "climate".to_string(),
            "set_temperature".to_string(),
            serde_json::json!({ "entity_id": entity_id, "temperature": new_temp }),
        )
    }

    pub async fn toggle_entity(&self, entity_id: &str) {
        let (domain, service, payload) = self.toggle_request(entity_id);
        let _ = self.call_service(&domain, &service, payload).await;
    }

    pub async fn set_climate_temp(&self, entity_id: &str, delta: f32) {
        let (domain, service, payload) = self.climate_request(entity_id, delta).await;
        let _ = self.call_service(&domain, &service, payload).await;
    }

    async fn apply_mock_service_call(&self, _domain: &str, service: &str, data: &Value) {
        let mut states = self.states.write().await;
        let entity_id = data.get("entity_id").and_then(|v| v.as_str()).unwrap_or("");
        if let Some(state) = states.get_mut(entity_id) {
            match service {
                "toggle" => {
                    state.state = if state.is_on() { "off" } else { "on" }.to_string();
                }
                "turn_on" => {
                    state.state = "on".to_string();
                }
                "turn_off" => {
                    state.state = "off".to_string();
                }
                "set_temperature" => {
                    if let Some(target) = data.get("temperature").and_then(|v| v.as_f64()) {
                        state.attributes.insert("temperature".to_string(), serde_json::json!(target));
                    }
                }
                "media_play_pause" => {
                    state.state = if state.state == "playing" { "paused" } else { "playing" }.to_string();
                }
                _ => {}
            }
        }
    }
}

fn downsample(values: &[f32], max_points: usize) -> Vec<f32> {
    if values.len() <= max_points {
        return values.to_vec();
    }
    let bucket = values.len() as f32 / max_points as f32;
    (0..max_points)
        .map(|i| {
            let a = (i as f32 * bucket) as usize;
            let b = (((i + 1) as f32 * bucket) as usize).clamp(a + 1, values.len());
            values[a..b].iter().sum::<f32>() / (b - a) as f32
        })
        .collect()
}
