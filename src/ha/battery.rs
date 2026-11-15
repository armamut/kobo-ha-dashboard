//! The Kobo's battery level as a Home Assistant sensor (issue #9).
//!
//! Written with `POST /api/states/<entity>`, which needs no integration on
//! the HA side but an admin user's token. Such a state isn't in the entity
//! registry and is gone after an HA restart until the next report.

use super::client::HaClient;
use crate::config::default_battery_entity;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

/// Report at least this often while awake, even when nothing changed, so
/// the sensor comes back soon after an HA restart.
const RESEND_AFTER: Duration = Duration::from_secs(600);

/// What was sent last, and when.
#[derive(Debug, Clone, Copy)]
pub struct Sent {
    percent: u32,
    charging: bool,
    at: Instant,
}

/// The configured entity if it's a valid sensor id, otherwise the default.
pub fn entity_id(configured: &str) -> String {
    let id = configured.trim();
    let valid = id
        .strip_prefix("sensor.")
        .map(|obj| !obj.is_empty() && obj.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'))
        .unwrap_or(false);
    if valid {
        id.to_string()
    } else {
        default_battery_entity()
    }
}

/// Request body for `POST /api/states/<entity>`; `percent` None writes
/// "unknown" (the setup tool creating the sensor before the Kobo reports).
pub fn payload(percent: Option<u32>, charging: Option<bool>) -> Value {
    let mut attributes = json!({
        "unit_of_measurement": "%",
        "device_class": "battery",
        "state_class": "measurement",
        "friendly_name": "Kobo battery",
    });
    if let Some(c) = charging {
        attributes["charging"] = json!(c);
    }
    json!({
        "state": percent.map(|p| p.to_string()).unwrap_or_else(|| "unknown".into()),
        "attributes": attributes,
    })
}

fn due(last: Option<Sent>, percent: u32, charging: bool, force: bool, now: Instant) -> bool {
    match last {
        None => true,
        Some(s) => force || s.percent != percent || s.charging != charging || now.duration_since(s.at) >= RESEND_AFTER,
    }
}

impl HaClient {
    /// Send the battery level if reporting is on and it changed, the last
    /// report is old, or `force` (after a wake-up or a cable change: the
    /// monotonic clock stops while suspended). `reading` is (percent,
    /// plugged in); None when the level can't be read.
    pub async fn report_battery(&self, reading: Option<(u32, bool)>, force: bool) {
        let config = self.config();
        if !config.report_battery || config.token.trim().is_empty() {
            return;
        }
        let Some((percent, charging)) = reading else {
            return;
        };
        let now = Instant::now();
        let last = *self.battery_sent.lock().unwrap_or_else(|e| e.into_inner());
        if !due(last, percent, charging, force, now) {
            return;
        }
        let entity = entity_id(&config.battery_entity);
        let result = self
            .client
            .post(self.url(&format!("/api/states/{}", entity)))
            .json(&payload(Some(percent), Some(charging)))
            .send()
            .await;
        match result {
            Ok(resp) if resp.status().is_success() => {
                tracing::info!("Battery {}%{} sent to {}", percent, if charging { " (charging)" } else { "" }, entity);
                *self.battery_sent.lock().unwrap_or_else(|e| e.into_inner()) = Some(Sent { percent, charging, at: now });
            }
            Ok(resp) if matches!(resp.status().as_u16(), 401 | 403) => {
                tracing::warn!("Battery report to {} refused (HTTP {}): the token must belong to an admin user", entity, resp.status())
            }
            Ok(resp) => tracing::warn!("Battery report to {} failed: HTTP {}", entity, resp.status()),
            Err(e) => tracing::warn!("Battery report to {} failed: {}", entity, e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entity_id_falls_back_to_the_default() {
        assert_eq!(entity_id("sensor.kobo_battery"), "sensor.kobo_battery");
        assert_eq!(entity_id(" sensor.kobo_2_battery "), "sensor.kobo_2_battery");
        assert_eq!(entity_id(""), "sensor.kobo_battery");
        assert_eq!(entity_id("kobo_battery"), "sensor.kobo_battery");
        assert_eq!(entity_id("sensor."), "sensor.kobo_battery");
        assert_eq!(entity_id("sensor.Kobo Battery"), "sensor.kobo_battery");
        assert_eq!(entity_id("binary_sensor.kobo"), "sensor.kobo_battery");
    }

    #[test]
    fn payload_is_a_battery_sensor() {
        let p = payload(Some(42), Some(true));
        assert_eq!(p["state"], "42");
        assert_eq!(p["attributes"]["device_class"], "battery");
        assert_eq!(p["attributes"]["unit_of_measurement"], "%");
        assert_eq!(p["attributes"]["charging"], true);
        let p = payload(None, None);
        assert_eq!(p["state"], "unknown");
        assert!(p["attributes"].get("charging").is_none());
    }

    #[test]
    fn reports_on_change_force_or_age() {
        let t = Instant::now();
        let last = Some(Sent { percent: 50, charging: false, at: t });
        assert!(due(None, 50, false, false, t));
        assert!(!due(last, 50, false, false, t + Duration::from_secs(60)));
        assert!(due(last, 49, false, false, t));
        assert!(due(last, 50, true, false, t));
        assert!(due(last, 50, false, true, t));
        assert!(due(last, 50, false, false, t + RESEND_AFTER));
    }

    /// Answers every request with `status` and hands each request's text over.
    async fn recording_ha(status: u16) -> (String, tokio::sync::mpsc::UnboundedReceiver<String>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                let mut buf = vec![0u8; 8192];
                let mut len = 0;
                // Headers, then the body by Content-Length.
                loop {
                    let n = sock.read(&mut buf[len..]).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    len += n;
                    let text = String::from_utf8_lossy(&buf[..len]).to_string();
                    if let Some(end) = text.find("\r\n\r\n") {
                        let body_len = text
                            .lines()
                            .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                            .unwrap_or(0);
                        if len >= end + 4 + body_len {
                            break;
                        }
                    }
                }
                let _ = tx.send(String::from_utf8_lossy(&buf[..len]).to_string());
                let resp = format!("HTTP/1.1 {} X\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}", status);
                let _ = sock.write_all(resp.as_bytes()).await;
            }
        });
        (url, rx)
    }

    fn client(url: String, report: bool) -> HaClient {
        HaClient::new(crate::config::HomeAssistantConfig {
            url,
            token: "t".into(),
            demo_mode_fallback: false,
            report_battery: report,
            battery_entity: "sensor.kobo_2_battery".into(),
            ..Default::default()
        })
    }

    #[tokio::test]
    async fn sends_the_level_once_until_it_changes() {
        let (url, mut rx) = recording_ha(200).await;
        let ha = client(url, true);
        ha.report_battery(Some((42, false)), false).await;
        let req = rx.recv().await.unwrap();
        assert!(req.starts_with("POST /api/states/sensor.kobo_2_battery "), "{}", req);
        assert!(req.contains(r#""state":"42""#), "{}", req);
        assert!(req.contains(r#""charging":false"#), "{}", req);

        ha.report_battery(Some((42, false)), false).await;
        ha.report_battery(Some((41, false)), false).await;
        let req = rx.recv().await.unwrap();
        assert!(req.contains(r#""state":"41""#), "the unchanged report should have been skipped: {}", req);
    }

    #[tokio::test]
    async fn nothing_is_sent_when_reporting_is_off_or_refused_reports_retry() {
        let (url, mut rx) = recording_ha(200).await;
        client(url, false).report_battery(Some((42, false)), true).await;
        assert!(rx.try_recv().is_err());

        let (url, mut rx) = recording_ha(401).await;
        let ha = client(url, true);
        ha.report_battery(Some((42, false)), false).await;
        ha.report_battery(Some((42, false)), false).await;
        assert!(rx.recv().await.is_some());
        assert!(rx.recv().await.is_some(), "a refused report doesn't count as sent");
    }
}
