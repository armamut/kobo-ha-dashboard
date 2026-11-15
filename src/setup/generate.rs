//! Home Assistant discovery and starter-dashboard generation for the setup tool.

use crate::config::{CardConfig, PageConfig, SceneItemConfig, SensorItemConfig};
use crate::setup::kobo::Msg;
use crate::i18n;
use crate::ui::screen::hidden_card_count;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityInfo {
    pub entity_id: String,
    pub domain: String,
    pub name: String,
    pub area: Option<String>,
    pub state: String,
    pub device_class: Option<String>,
    pub unit: Option<String>,
    pub hidden: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConnectionReport {
    pub ok: bool,
    /// Translatable message (key under `report.`)
    pub message: Msg,
    pub version: Option<String>,
    pub location: Option<String>,
    pub warnings: Vec<Msg>,
    /// A better URL to use from the Kobo, if we found one
    pub suggested_url: Option<String>,
}

fn client(token: &str) -> reqwest::Client {
    let mut headers = reqwest::header::HeaderMap::new();
    if let Ok(v) = reqwest::header::HeaderValue::from_str(&format!("Bearer {}", token.trim())) {
        headers.insert(reqwest::header::AUTHORIZATION, v);
    }
    reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .connect_timeout(Duration::from_secs(5))
        .default_headers(headers)
        .build()
        .unwrap_or_default()
}

fn base(url: &str) -> String {
    crate::ha::base_url(url).0
}

/// Check URL + token and point out setups that work on a PC but not on a Kobo.
pub async fn test_connection(url: &str, token: &str) -> ConnectionReport {
    let mut warnings = Vec::new();
    let mut suggested_url = None;
    let (url, path_dropped) = crate::ha::base_url(url);

    if url.is_empty() || token.trim().is_empty() {
        return ConnectionReport {
            ok: false,
            message: Msg::new("missing"),
            version: None,
            location: None,
            warnings,
            suggested_url,
        };
    }

    let parsed = reqwest::Url::parse(&url);
    if let Ok(u) = &parsed {
        if u.host_str().map(|h| h.ends_with(".local")).unwrap_or(false) {
            warnings.push(Msg::new("mdns"));
            // Resolve it here so we can suggest the IP.
            let port = u.port_or_known_default().unwrap_or(8123);
            if let Ok(mut addrs) = tokio::net::lookup_host((u.host_str().unwrap_or(""), port)).await {
                if let Some(ip) = addrs.find(|a| a.is_ipv4()) {
                    suggested_url = Some(format!("{}://{}:{}", u.scheme(), ip.ip(), port));
                }
            }
        }
        if path_dropped {
            // Page paths like /lovelace/0 are not part of the API address.
            warnings.push(Msg::new("path_removed").arg("url", url.as_str()));
            suggested_url.get_or_insert_with(|| url.clone());
        }
    } else {
        return ConnectionReport {
            ok: false,
            message: Msg::new("invalid_url").arg("url", url.as_str()),
            version: None,
            location: None,
            warnings,
            suggested_url,
        };
    }

    let http = client(token);
    let result = http.get(format!("{}/api/config", url)).send().await;
    match result {
        Ok(resp) if resp.status().is_success() => {
            // Anything that isn't HA's config JSON (its web app, a proxy page)
            // would only fail later, when the entities are read.
            let body: serde_json::Value = crate::ha::read_json(resp).await.unwrap_or_default();
            let version = body.get("version").and_then(|v| v.as_str()).map(str::to_string);
            if version.is_none() {
                return ConnectionReport {
                    ok: false,
                    message: Msg::new("not_api"),
                    version: None,
                    location: None,
                    warnings,
                    suggested_url,
                };
            }
            ConnectionReport {
                ok: true,
                message: Msg::new("connected"),
                version,
                location: body.get("location_name").and_then(|v| v.as_str()).map(str::to_string),
                warnings,
                suggested_url,
            }
        }
        Ok(resp) if resp.status().as_u16() == 401 => ConnectionReport {
            ok: false,
            message: Msg::new("unauthorized"),
            version: None,
            location: None,
            warnings,
            suggested_url,
        },
        Ok(resp) => ConnectionReport {
            ok: false,
            message: Msg::new("http").arg("status", resp.status().to_string()),
            version: None,
            location: None,
            warnings,
            suggested_url,
        },
        Err(e) => {
            let mut warnings = warnings;
            if let Ok(u) = &parsed {
                if u.scheme() == "https" && u.port() == Some(8123) {
                    warnings.push(Msg::new("https_port"));
                }
            }
            let message = Msg::new("unreachable").arg("error", e.to_string());
            ConnectionReport { ok: false, message, version: None, location: None, warnings, suggested_url }
        }
    }
}

/// Battery reporting writes a state, which needs an admin user's token.
/// Write the sensor now (keeping a value the Kobo already sent, else
/// "unknown") so it can be used in automations right away. A warning if
/// that's not possible.
pub async fn prepare_battery_sensor(url: &str, token: &str, entity: &str) -> Option<Msg> {
    let url = base(url);
    let entity = crate::ha::battery::entity_id(entity);
    let http = client(token);
    let existing: Option<serde_json::Value> = match http.get(format!("{}/api/states/{}", url, entity)).send().await {
        Ok(r) if r.status().is_success() => r.json().await.ok(),
        _ => None,
    };
    let body = match existing {
        Some(state) if state.get("state").and_then(|v| v.as_str()).is_some() => serde_json::json!({
            "state": state["state"],
            "attributes": state.get("attributes").cloned().unwrap_or_default(),
        }),
        _ => crate::ha::battery::payload(None, None),
    };
    match http.post(format!("{}/api/states/{}", url, entity)).json(&body).send().await {
        Ok(r) if r.status().is_success() => None,
        Ok(r) if matches!(r.status().as_u16(), 401 | 403) => Some(Msg::new("battery_not_admin")),
        Ok(r) => Some(Msg::new("battery_failed").arg("error", format!("HTTP {}", r.status()))),
        Err(e) => Some(Msg::new("battery_failed").arg("error", e.to_string())),
    }
}

/// All entities with their area and hidden flag.
pub async fn fetch_entities(url: &str, token: &str) -> Result<Vec<EntityInfo>, Box<dyn std::error::Error + Send + Sync>> {
    let url = base(url);
    let http = client(token);
    let resp = http.get(format!("{}/api/states", url)).send().await?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()).into());
    }
    let states: Vec<serde_json::Value> = crate::ha::read_json(resp).await?;

    // Areas and hidden flags are only exposed to templates, not /api/states.
    let template = r#"{% set ns = namespace(o={}) %}{% for s in states %}{% set ns.o = dict(ns.o, **{s.entity_id: [area_name(s.entity_id) or '', is_hidden_entity(s.entity_id)]}) %}{% endfor %}{{ ns.o | tojson }}"#;
    let meta: BTreeMap<String, (String, bool)> = match http
        .post(format!("{}/api/template", url))
        .json(&serde_json::json!({ "template": template }))
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => r
            .text()
            .await
            .ok()
            .and_then(|t| serde_json::from_str::<BTreeMap<String, (String, bool)>>(&t).ok())
            .unwrap_or_default(),
        _ => {
            tracing::warn!("Area lookup failed; entities will not be grouped by area");
            BTreeMap::new()
        }
    };

    let mut out: Vec<EntityInfo> = states
        .iter()
        .filter_map(|s| {
            let entity_id = s.get("entity_id")?.as_str()?.to_string();
            let attrs = s.get("attributes");
            let attr = |k: &str| attrs.and_then(|a| a.get(k)).and_then(|v| v.as_str()).map(str::to_string);
            let (area, hidden) = meta.get(&entity_id).cloned().unwrap_or_default();
            Some(EntityInfo {
                domain: entity_id.split('.').next().unwrap_or("").to_string(),
                name: attr("friendly_name").unwrap_or_else(|| entity_id.clone()),
                area: (!area.is_empty()).then_some(area),
                state: s.get("state").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                device_class: attr("device_class"),
                unit: attr("unit_of_measurement"),
                hidden,
                entity_id,
            })
        })
        .collect();
    out.sort_by(|a, b| a.entity_id.cmp(&b.entity_id));
    Ok(out)
}

/// Switch entities that are device settings rather than things to control.
fn is_technical(e: &EntityInfo) -> bool {
    const PATTERNS: &[&str] = &[
        "child_lock", "network_led", "_led", "led_", "indicator", "turbo", "detach", "power_outage", "outage_memory",
        "adaptive", "open_window", "window_detection", "external_temperature", "inverted", "detection", "buzzer",
        "auto_off", "restart", "reboot", "firmware", "calibration", "do_not_disturb", "privacy", "lock_",
        // Enable switches of thermostatic radiator valves
        "trv", "thermostat", "termostat", "valve", "vana",
    ];
    let id = e.entity_id.to_lowercase();
    PATTERNS.iter().any(|p| id.contains(p))
}

/// "Living Room - Ceiling Light" in area "Living Room" -> "Ceiling Light"
fn short_name(name: &str, area: Option<&str>) -> String {
    let mut n = name.trim().to_string();
    if let Some(area) = area {
        let lower = n.to_lowercase();
        if lower.starts_with(&area.to_lowercase()) && n.len() > area.len() {
            let rest = n[area.len()..].trim_start_matches(|c: char| c.is_whitespace() || "-–—:·|/".contains(c));
            if !rest.is_empty() {
                n = rest.to_string();
            }
        }
    }
    n
}

fn sensor_icon(device_class: Option<&str>) -> &'static str {
    match device_class {
        Some("temperature") => "thermometer",
        Some("humidity") | Some("moisture") => "droplet",
        Some("carbon_dioxide") | Some("pm25") | Some("pm10") | Some("volatile_organic_compounds") | Some("aqi") => "wind",
        Some("battery") => "battery",
        _ => "zap",
    }
}

fn card_for(e: &EntityInfo) -> Option<CardConfig> {
    let title = Some(short_name(&e.name, e.area.as_deref()));
    let subtitle = e.area.clone();
    Some(match e.domain.as_str() {
        "light" => CardConfig::Light {
            entity_id: e.entity_id.clone(),
            title,
            subtitle,
            icon: None,
            show_brightness: false,
        },
        "switch" | "fan" | "input_boolean" => CardConfig::Switch {
            entity_id: e.entity_id.clone(),
            title,
            subtitle,
            icon: if e.domain == "fan" { Some("fan".into()) } else { None },
        },
        "climate" => CardConfig::Climate { entity_id: e.entity_id.clone(), title, step: 0.5 },
        "media_player" => CardConfig::MediaPlayer { entity_id: e.entity_id.clone(), title },
        _ => return None,
    })
}

/// Build a starter dashboard: an overview page plus one page per area,
/// splitting pages that would not fit the screen.
pub fn generate(entities: &[EntityInfo], screen: (u32, u32)) -> Vec<PageConfig> {
    let usable: Vec<&EntityInfo> = entities
        .iter()
        .filter(|e| !e.hidden && e.state != "unavailable")
        .filter(|e| !(e.domain == "switch" && is_technical(e)))
        .collect();

    // A "switch as light" keeps a hidden or same-named switch around; drop it.
    let light_names: HashSet<String> =
        usable.iter().filter(|e| e.domain == "light").map(|e| e.name.to_lowercase()).collect();
    let mut seen_media = HashSet::new();
    let usable: Vec<&EntityInfo> = usable
        .into_iter()
        .filter(|e| !(e.domain == "switch" && light_names.contains(&e.name.to_lowercase())))
        .filter(|e| e.domain != "media_player" || seen_media.insert(e.name.to_lowercase()))
        .collect();

    // Rank areas by size: the busiest one is usually the living room, so the
    // overview picks its lights and thermostat first.
    let mut area_size: BTreeMap<&str, usize> = BTreeMap::new();
    for e in usable.iter().filter(|e| matches!(e.domain.as_str(), "light" | "switch" | "climate" | "media_player")) {
        if let Some(a) = e.area.as_deref() {
            *area_size.entry(a).or_default() += 1;
        }
    }
    let rank = |e: &&EntityInfo| std::cmp::Reverse(e.area.as_deref().and_then(|a| area_size.get(a)).copied().unwrap_or(0));
    let mut by_area: Vec<&EntityInfo> = usable.clone();
    by_area.sort_by_key(rank);

    let mut pages = Vec::new();

    // Overview
    let mut overview = Vec::new();
    if let Some(w) = usable.iter().find(|e| e.domain == "weather") {
        overview.push(CardConfig::Weather { entity_id: w.entity_id.clone(), title: None, forecast_days: 5 });
    }
    let scenes: Vec<SceneItemConfig> = usable
        .iter()
        .filter(|e| e.domain == "scene" || e.domain == "script")
        .take(4)
        .map(|e| SceneItemConfig {
            entity_id: e.entity_id.clone(),
            title: short_name(&e.name, None),
            icon: if e.domain == "scene" { "play".into() } else { "zap".into() },
        })
        .collect();
    if !scenes.is_empty() {
        overview.push(CardConfig::ScenesRow { title: Some(i18n::t("generated.scenes")), scenes });
    }
    for e in by_area.iter().filter(|e| e.domain == "light").take(4) {
        overview.extend(card_for(e));
    }
    if let Some(c) = by_area.iter().find(|e| e.domain == "climate") {
        overview.extend(card_for(c));
    }
    if let Some(m) = by_area.iter().find(|e| e.domain == "media_player" && e.state != "off") {
        overview.extend(card_for(m));
    }
    if !overview.is_empty() {
        pages.push(PageConfig { id: "home".into(), title: i18n::t("generated.overview"), icon: "home".into(), cards: overview });
    }

    // Per area, biggest areas first; entities without an area go to "Other".
    let mut areas: BTreeMap<String, Vec<&EntityInfo>> = BTreeMap::new();
    for e in &usable {
        if matches!(e.domain.as_str(), "light" | "switch" | "fan" | "input_boolean" | "climate" | "media_player" | "sensor") {
            let area = e.area.clone().unwrap_or_else(|| i18n::t("generated.other"));
            areas.entry(area).or_default().push(e);
        }
    }

    let mut area_pages: Vec<(String, Vec<CardConfig>)> = areas
        .into_iter()
        .filter_map(|(area, ents)| {
            let order = |d: &str| match d {
                "climate" => 0,
                "light" => 1,
                "switch" | "fan" | "input_boolean" => 2,
                "media_player" => 3,
                _ => 4,
            };
            let mut ents = ents;
            ents.sort_by_key(|e| order(&e.domain));
            let mut cards: Vec<CardConfig> = ents.iter().filter_map(|e| card_for(e)).collect();
            let sensors: Vec<SensorItemConfig> = ents
                .iter()
                .filter(|e| e.domain == "sensor")
                .filter(|e| {
                    matches!(
                        e.device_class.as_deref(),
                        Some("temperature" | "humidity" | "carbon_dioxide" | "pm25" | "power")
                    ) && e.state.parse::<f64>().is_ok()
                })
                .take(4)
                .map(|e| SensorItemConfig {
                    entity_id: e.entity_id.clone(),
                    title: short_name(&e.name, e.area.as_deref()),
                    icon: Some(sensor_icon(e.device_class.as_deref()).into()),
                    unit: e.unit.clone(),
                })
                .collect();
            if !sensors.is_empty() && !cards.is_empty() {
                cards.push(CardConfig::SensorGrid { title: None, sensors, large_value: false });
            }
            (!cards.is_empty()).then_some((area, cards))
        })
        .collect();
    area_pages.sort_by(|a, b| b.1.len().cmp(&a.1.len()));

    // Fold tiny areas into "Other" so the tab bar stays readable. Cards keep
    // their area as subtitle, so nothing becomes ambiguous.
    const MIN_CARDS: usize = 3;
    let other = i18n::t("generated.other");
    let (mut area_pages, small): (Vec<_>, Vec<_>) = area_pages.into_iter().partition(|(area, cards)| cards.len() >= MIN_CARDS || *area == other);
    let leftovers: Vec<CardConfig> = small.into_iter().flat_map(|(_, cards)| cards).collect();
    if !leftovers.is_empty() {
        match area_pages.iter_mut().find(|(area, _)| *area == other) {
            Some((_, cards)) => cards.extend(leftovers),
            None => area_pages.push((other.clone(), leftovers)),
        }
    }

    let (w, h) = screen;
    for (area, cards) in area_pages {
        let icon = if cards.iter().any(|c| matches!(c, CardConfig::Light { .. })) { "lightbulb" } else { "home" };
        let mut chunk: Vec<CardConfig> = Vec::new();
        let mut part = 1;
        for card in cards {
            chunk.push(card);
            if hidden_card_count(&chunk, w, h, true) > 0 && chunk.len() > 1 {
                let overflow = chunk.pop().expect("chunk has the card we just pushed");
                pages.push(page(&area, part, icon, std::mem::take(&mut chunk)));
                part += 1;
                chunk.push(overflow);
            }
        }
        if !chunk.is_empty() {
            pages.push(page(&area, part, icon, chunk));
        }
    }

    pages
}

fn page(area: &str, part: usize, icon: &str, cards: Vec<CardConfig>) -> PageConfig {
    let title = if part == 1 { area.to_string() } else { format!("{} {}", area, part) };
    let id: String = title
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();
    PageConfig { id, title, icon: icon.to_string(), cards }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ent(id: &str, name: &str, area: Option<&str>, state: &str) -> EntityInfo {
        EntityInfo {
            entity_id: id.into(),
            domain: id.split('.').next().unwrap().into(),
            name: name.into(),
            area: area.map(str::to_string),
            state: state.into(),
            device_class: None,
            unit: None,
            hidden: false,
        }
    }

    #[test]
    fn filters_noise_and_groups_by_area() {
        let _guard = i18n::TEST_LANGUAGE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        i18n::set_language("en");
        let entities = vec![
            ent("weather.home", "Home", None, "sunny"),
            ent("light.salon_main", "Salon - Main Light", Some("Salon"), "on"),
            ent("light.salon_lamp", "Salon - Floor Lamp", Some("Salon"), "off"),
            ent("switch.salon_main", "Salon - Main Light", Some("Salon"), "on"),
            ent("switch.trv_child_lock", "TRV Child lock", Some("Salon"), "off"),
            ent("switch.kettle", "Kettle", Some("Kitchen"), "off"),
            ent("light.broken", "Broken", Some("Kitchen"), "unavailable"),
            ent("climate.salon", "Salon Thermostat", Some("Salon"), "heat"),
        ];
        let pages = generate(&entities, (1072, 1448));
        let ids: Vec<String> = pages
            .iter()
            .flat_map(|p| p.cards.iter())
            .filter_map(|c| match c {
                CardConfig::Light { entity_id, .. }
                | CardConfig::Switch { entity_id, .. }
                | CardConfig::Climate { entity_id, .. } => Some(entity_id.clone()),
                _ => None,
            })
            .collect();
        assert!(ids.contains(&"light.salon_main".to_string()));
        assert!(ids.contains(&"switch.kettle".to_string()));
        assert!(!ids.contains(&"switch.salon_main".to_string()), "switch-as-light duplicate kept");
        assert!(!ids.contains(&"switch.trv_child_lock".to_string()), "technical switch kept");
        assert!(!ids.contains(&"light.broken".to_string()), "unavailable entity kept");
        assert!(pages.iter().any(|p| p.title == "Salon"));
        // Kitchen has a single card, so it is folded into "Other"
        assert!(pages.iter().any(|p| p.title == "Other"));
        assert!(!pages.iter().any(|p| p.title == "Kitchen"));
        let salon_light = pages.iter().find(|p| p.title == "Salon").unwrap().cards.iter().find_map(|c| match c {
            CardConfig::Light { title, .. } => title.clone(),
            _ => None,
        });
        assert_eq!(salon_light.as_deref(), Some("Main Light"));
    }

    #[test]
    fn long_areas_are_split_into_pages_that_fit() {
        let _guard = i18n::TEST_LANGUAGE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        i18n::set_language("en");
        let entities: Vec<EntityInfo> =
            (0..30).map(|i| ent(&format!("light.l{}", i), &format!("Light {}", i), Some("Big"), "off")).collect();
        let pages = generate(&entities, (1072, 1448));
        let big: Vec<&PageConfig> = pages.iter().filter(|p| p.title.starts_with("Big")).collect();
        assert!(big.len() >= 2);
        for p in big {
            assert_eq!(hidden_card_count(&p.cards, 1072, 1448, true), 0, "{} overflows", p.title);
        }
    }

    /// Minimal stand-in for HA: answers every request with `body`.
    async fn fake_ha(content_type: &'static str, body: &'static str) -> String {
        fake_ha_status(200, content_type, body).await
    }

    async fn fake_ha_status(status: u16, content_type: &'static str, body: &'static str) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            while let Ok((mut sock, _)) = listener.accept().await {
                let mut buf = [0u8; 4096];
                let _ = sock.read(&mut buf).await;
                let resp = format!(
                    "HTTP/1.1 {} X\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    status,
                    content_type,
                    body.len(),
                    body
                );
                let _ = sock.write_all(resp.as_bytes()).await;
            }
        });
        format!("http://127.0.0.1:{}", port)
    }

    #[tokio::test]
    async fn web_app_page_is_not_a_connection() {
        let url = fake_ha("text/html", "<!DOCTYPE html><html></html>").await;
        let report = test_connection(&format!("{}/home/overview", url), "token").await;
        assert!(!report.ok);
        assert_eq!(report.message.key, "not_api");
        assert!(report.warnings.iter().any(|w| w.key == "path_removed"));
        assert_eq!(report.suggested_url.as_deref(), Some(url.as_str()));

        let err = fetch_entities(&url, "token").await.unwrap_err().to_string();
        assert!(err.contains("text/html"), "{}", err);
    }

    #[tokio::test]
    async fn page_path_is_dropped_for_api_calls() {
        let url = fake_ha("application/json", r#"{"version":"2026.11.1","location_name":"Home"}"#).await;
        let report = test_connection(&format!("{}/lovelace/0", url), "token").await;
        assert!(report.ok);
        assert_eq!(report.version.as_deref(), Some("2026.11.1"));
        assert!(report.warnings.iter().any(|w| w.key == "path_removed"));
    }

    #[tokio::test]
    async fn battery_sensor_needs_an_admin_token() {
        let url = fake_ha_status(401, "application/json", r#"{"message":"Unauthorized"}"#).await;
        let warning = prepare_battery_sensor(&url, "token", "sensor.kobo_battery").await;
        assert_eq!(warning.map(|w| w.key), Some("battery_not_admin".to_string()));

        let url = fake_ha("application/json", r#"{"state":"57","attributes":{"unit_of_measurement":"%"}}"#).await;
        assert!(prepare_battery_sensor(&url, "token", "sensor.kobo_battery").await.is_none());
    }
}
