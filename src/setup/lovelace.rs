//! Import a Home Assistant Lovelace dashboard as native pages.
//!
//! The Kobo draws its own widgets, so a Lovelace card is translated to the
//! closest native card. Cards without a native equivalent are counted and
//! reported instead of silently disappearing, and a view that does not fit the
//! screen is split into several pages.

use crate::config::{CardConfig, PageConfig};
use crate::ui::screen::flow_hidden_count;
#[cfg(test)]
use crate::ui::screen::hidden_card_count;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;

/// A Lovelace card (or entity domain) the Kobo has no widget for, with how
/// often it was left out.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Skipped {
    /// Card type ("gauge", "custom:mushroom-light-card") or entity domain ("cover")
    pub card: String,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportResult {
    pub pages: Vec<PageConfig>,
    pub skipped: Vec<Skipped>,
}

/// Why a dashboard cannot be imported at all.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ImportError {
    /// Built by a dashboard strategy; the cards exist only in the frontend.
    Strategy,
    /// No views, or nothing in them the Kobo can show.
    Empty,
}

/// Dashboards of this Home Assistant as (url_path, title), the default
/// dashboard first. Listing them needs an admin token; a plain user still gets
/// the default dashboard.
pub async fn dashboards(url: &str, token: &str) -> Result<Vec<(String, String)>, String> {
    let mut out = vec![(String::new(), "Overview".to_string())];
    let payload = serde_json::json!({ "id": 1, "type": "lovelace/dashboards/list" });
    // Listing dashboards needs an admin token. A plain user still gets the
    // default dashboard, so a failure here is not an error.
    let Ok(result) = crate::ha::ws::request(url, token, payload).await else {
        return Ok(out);
    };
    let Some(list) = result.as_array() else { return Ok(out) };
    for dashboard in list {
        let path = dashboard.get("url_path").and_then(Value::as_str).unwrap_or("");
        let title = dashboard.get("title").and_then(Value::as_str).unwrap_or(path);
        // "lovelace" is how Home Assistant names the default dashboard in its
        // storage, and that one is already offered as the empty path.
        if !path.is_empty() && path != "lovelace" {
            out.push((path.to_string(), title.to_string()));
        }
    }
    Ok(out)
}

/// The raw Lovelace config of one dashboard (`None` = the default one).
pub async fn fetch_config(url: &str, token: &str, url_path: Option<&str>) -> Result<Value, String> {
    let payload = serde_json::json!({
        "id": 1,
        "type": "lovelace/config",
        "url_path": url_path,
        "force": false,
    });
    crate::ha::ws::request(url, token, payload).await.map_err(|e| e.to_string())
}

/// Translate a Lovelace dashboard config into pages. `with_tabs` says whether
/// the dashboard will show a tab bar; that costs a row of height, so it decides
/// how much fits on a page.
pub fn import(config: &Value, screen: (u32, u32), with_tabs: bool) -> Result<ImportResult, ImportError> {
    import_with(config, screen, with_tabs, &BTreeMap::new())
}

/// `state_classes` maps an entity to its `state_class`, so a chart without
/// `stat_types` can pick the type Home Assistant would: `change` for a meter
/// (which has `sum`), `mean` for a measurement such as temperature.
pub fn import_with(
    config: &Value,
    screen: (u32, u32),
    with_tabs: bool,
    state_classes: &BTreeMap<String, String>,
) -> Result<ImportResult, ImportError> {
    if config.get("strategy").is_some() {
        return Err(ImportError::Strategy);
    }
    let views = config.get("views").and_then(Value::as_array).ok_or(ImportError::Empty)?;
    let mut skipped: BTreeMap<String, usize> = BTreeMap::new();
    let with_tabs = with_tabs || views.len() > 1;
    let mut pages = Vec::new();
    let mut used = std::collections::HashSet::new();
    let mut saw_strategy = false;

    for (idx, view) in views.iter().enumerate() {
        if view.get("strategy").is_some() {
            saw_strategy = true;
            skip(&mut skipped, "strategy");
            continue;
        }
        let mut cards = Vec::new();
        if let Some(list) = view.get("cards").and_then(Value::as_array) {
            flatten(list, &mut cards, &mut skipped);
        }
        // Views created in the dashboard editor keep their cards in sections.
        if let Some(sections) = view.get("sections").and_then(Value::as_array) {
            for section in sections {
                if let Some(list) = section.get("cards").and_then(Value::as_array) {
                    flatten(list, &mut cards, &mut skipped);
                }
            }
        }
        if cards.is_empty() {
            continue;
        }
        let title = view
            .get("title")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| view.get("path").and_then(Value::as_str).map(humanize))
            .unwrap_or_else(|| if idx == 0 { "Home".to_string() } else { format!("Page {}", idx + 1) });
        let icon = icon_name(view.get("icon")).unwrap_or_else(|| "home".to_string());
        pages.extend(paginate(&title, &icon, cards, screen, with_tabs, &mut used));
    }

    if pages.is_empty() {
        // Every view was a strategy: say so, instead of claiming it is empty.
        return Err(if saw_strategy { ImportError::Strategy } else { ImportError::Empty });
    }
    // A chart without an explicit type gets the one its entity supports.
    for page in &mut pages {
        for card in &mut page.cards {
            if let CardConfig::Statistics { entity_id, stat, .. } = &mut card.kind {
                if stat.is_empty() {
                    *stat = stat_for(entity_id, state_classes);
                }
            }
        }
    }
    let mut skipped: Vec<Skipped> = skipped.into_iter().map(|(card, count)| Skipped { card, count }).collect();
    skipped.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.card.cmp(&b.card)));
    Ok(ImportResult { pages, skipped })
}

fn skip(skipped: &mut BTreeMap<String, usize>, kind: &str) {
    *skipped.entry(kind.to_string()).or_default() += 1;
}

/// Charts read `change` from the `sum` column, which only meters have; any other
/// sensor gets `mean`, because a chart with no data points shows nothing at all.
fn stat_for(entity: &str, state_classes: &BTreeMap<String, String>) -> String {
    match state_classes.get(entity).map(String::as_str) {
        Some("total" | "total_increasing") => "change",
        _ => "mean",
    }
    .to_string()
}

/// `state_class` per entity, so a chart can use the type Home Assistant would
/// pick for that sensor.
pub async fn state_classes(url: &str, token: &str) -> BTreeMap<String, String> {
    let http = crate::setup::generate::client(token);
    let url = crate::setup::generate::base(url);
    let resp = match http.get(format!("{}/api/states", url)).send().await {
        Ok(resp) if resp.status().is_success() => resp,
        Ok(resp) => {
            tracing::warn!("state_class lookup failed: HTTP {}", resp.status());
            return BTreeMap::new();
        }
        Err(e) => {
            tracing::warn!("state_class lookup failed: {}", e);
            return BTreeMap::new();
        }
    };
    let states: Vec<Value> = match crate::ha::read_json(resp).await {
        Ok(list) => list,
        Err(e) => {
            tracing::warn!("state_class lookup failed: {}", e);
            return BTreeMap::new();
        }
    };
    let mut out = BTreeMap::new();
    for state in states {
        let Some(entity) = state.get("entity_id").and_then(Value::as_str) else { continue };
        if let Some(class) = state.get("attributes").and_then(|a| a.get("state_class")).and_then(Value::as_str) {
            out.insert(entity.to_string(), class.to_string());
        }
    }
    out
}

/// Walk a list of Lovelace cards in order, flattening stacks and grids.
fn flatten(cards: &[Value], out: &mut Vec<CardConfig>, skipped: &mut BTreeMap<String, usize>) {
    for card in cards {
        let kind = card.get("type").and_then(Value::as_str).unwrap_or("");
        match kind {
            "heading" => {
                if let Some(title) = card.get("heading").and_then(Value::as_str) {
                    let title = title.trim();
                    if !title.is_empty() {
                        out.push(CardConfig::Heading { title: title.to_string(), icon: icon_name(card.get("icon")) });
                    }
                }
            }
            "grid" | "vertical-stack" | "horizontal-stack" => {
                if let Some(list) = card.get("cards").and_then(Value::as_array) {
                    flatten(list, out, skipped);
                }
            }
            // Conditions are ignored: the card is shown whenever it exists.
            "conditional" => {
                if let Some(inner) = card.get("card") {
                    flatten(std::slice::from_ref(inner), out, skipped);
                } else if let Some(list) = card.get("cards").and_then(Value::as_array) {
                    flatten(list, out, skipped);
                }
            }
            "tile" | "entity" => {
                let Some(entity) = card.get("entity").and_then(Value::as_str) else {
                    skip(skipped, kind);
                    continue;
                };
                let graph = wants_graph(card);
                entity_card(entity, name_of(card), card.get("icon"), graph, out, skipped);
            }
            "entities" | "glance" => {
                let Some(list) = card.get("entities").and_then(Value::as_array) else {
                    skip(skipped, kind);
                    continue;
                };
                for item in list {
                    let entity = item
                        .get("entity")
                        .and_then(Value::as_str)
                        .or_else(|| item.as_str())
                        // An `attribute`/`divider` row inherits the card's entity.
                        .or_else(|| card.get("entity").and_then(Value::as_str));
                    match entity {
                        Some(entity) => entity_card(entity, name_of(item), item.get("icon"), false, out, skipped),
                        None => skip(skipped, kind),
                    }
                }
            }
            "button" => {
                let Some(entity) = card.get("entity").and_then(Value::as_str) else {
                    skip(skipped, kind);
                    continue;
                };
                let title = name_of(card).map(str::to_string).unwrap_or_else(|| entity_label(entity));
                out.push(CardConfig::Scene {
                    entity_id: entity.to_string(),
                    title,
                    icon: icon_name(card.get("icon")).or_else(|| Some("play".to_string())),
                });
            }
            "light" => {
                let Some(entity) = card.get("entity").and_then(Value::as_str) else {
                    skip(skipped, kind);
                    continue;
                };
                out.push(CardConfig::Light {
                    entity_id: entity.to_string(),
                    title: name_of(card).map(str::to_string),
                    subtitle: None,
                    icon: icon_name(card.get("icon")),
                    show_brightness: true,
                });
            }
            "thermostat" => {
                let Some(entity) = card.get("entity").and_then(Value::as_str) else {
                    skip(skipped, kind);
                    continue;
                };
                out.push(CardConfig::Climate { entity_id: entity.to_string(), title: name_of(card).map(str::to_string), step: 0.5 });
            }
            "media-control" => {
                let Some(entity) = card.get("entity").and_then(Value::as_str) else {
                    skip(skipped, kind);
                    continue;
                };
                out.push(CardConfig::MediaPlayer { entity_id: entity.to_string(), title: name_of(card).map(str::to_string) });
            }
            "weather-forecast" => {
                let Some(entity) = card.get("entity").and_then(Value::as_str) else {
                    skip(skipped, kind);
                    continue;
                };
                out.push(CardConfig::Weather { entity_id: entity.to_string(), title: name_of(card).map(str::to_string), forecast_days: 5 });
            }
            "sensor" => {
                let Some(entity) = card.get("entity").and_then(Value::as_str) else {
                    skip(skipped, kind);
                    continue;
                };
                let graph = card.get("graph").and_then(Value::as_str) != Some("none");
                entity_card(entity, name_of(card), card.get("icon"), graph, out, skipped);
            }
            "history-graph" => {
                let Some(list) = card.get("entities").and_then(Value::as_array) else {
                    skip(skipped, kind);
                    continue;
                };
                for item in list {
                    match item.get("entity").and_then(Value::as_str).or_else(|| item.as_str()) {
                        Some(entity) => entity_card(entity, name_of(item), None, true, out, skipped),
                        None => skip(skipped, kind),
                    }
                }
            }
            "statistics-graph" => statistics_cards(card, out, skipped),
            // A picture card: Home Assistant image entities are fetched and drawn by
// the image card. A picture from a URL is not fetched (the Kobo has no
// browser) and is reported.
            "picture" | "image" => match card.get("image_entity").and_then(Value::as_str) {
                Some(entity) => out.push(CardConfig::Image {
                    entity_id: entity.to_string(),
                    title: name_of(card).map(str::to_string),
                }),
                None => skip(skipped, kind),
            },
            "" => skip(skipped, "unknown"),
            other => skip(skipped, other),
        }
    }
}

fn name_of(card: &Value) -> Option<&str> {
    card.get("name").and_then(Value::as_str).or_else(|| card.get("title").and_then(Value::as_str))
}

/// Tile features that ask for a graph (Lovelace's "trend-graph").
fn wants_graph(card: &Value) -> bool {
    card.get("features")
        .and_then(Value::as_array)
        .map(|features| {
            features
                .iter()
                .any(|f| f.get("type").and_then(Value::as_str) == Some("trend-graph"))
        })
        .unwrap_or(false)
}

/// One entity, as the native card that fits its domain.
fn entity_card(
    entity: &str,
    name: Option<&str>,
    icon: Option<&Value>,
    graph: bool,
    out: &mut Vec<CardConfig>,
    skipped: &mut BTreeMap<String, usize>,
) {
    let entity_id = entity.to_string();
    let title = name.map(str::to_string);
    let icon = icon_name(icon);
    let domain = entity.split('.').next().unwrap_or("");
    let card = match domain {
        "light" => Some(CardConfig::Light { entity_id, title, subtitle: None, icon, show_brightness: true }),
        "switch" | "input_boolean" | "automation" | "humidifier" | "siren" => {
            Some(CardConfig::Switch { entity_id, title, subtitle: None, icon })
        }
        "fan" => Some(CardConfig::Switch { entity_id, title, subtitle: None, icon: icon.or_else(|| Some("fan".to_string())) }),
        "climate" => Some(CardConfig::Climate { entity_id, title, step: 0.5 }),
        "media_player" => Some(CardConfig::MediaPlayer { entity_id, title }),
        "weather" => Some(CardConfig::Weather { entity_id, title, forecast_days: 5 }),
        "scene" => Some(CardConfig::Scene {
            entity_id,
            title: title.unwrap_or_else(|| "Scene".to_string()),
            icon: icon.or_else(|| Some("play".to_string())),
        }),
        "script" | "button" | "input_button" => Some(CardConfig::Scene {
            entity_id,
            title: title.unwrap_or_else(|| "Run".to_string()),
            icon: icon.or_else(|| Some("zap".to_string())),
        }),
        "sensor" | "binary_sensor" | "number" | "input_number" | "counter" | "input_text" | "input_select" | "select"
        | "text" => Some(CardConfig::Sensor { entity_id, title, unit: None, icon, show_graph: graph, large_value: false }),
        "image" => Some(CardConfig::Image { entity_id, title }),
        other => {
            skip(skipped, other);
            None
        }
    };
    out.extend(card);
}

/// `statistics-graph` shows one series per entity; each becomes its own chart.
fn statistics_cards(card: &Value, out: &mut Vec<CardConfig>, skipped: &mut BTreeMap<String, usize>) {
    let Some(list) = card.get("entities").and_then(Value::as_array) else {
        skip(skipped, "statistics-graph");
        return;
    };
    let period = match card.get("period").and_then(Value::as_str) {
        None | Some("day") => "day",
        Some("week") => "week",
        Some("month") => "month",
        // A year of months is a month chart with a longer range.
        Some("year") => "month",
        // Hourly and 5-minute statistics do not fit an e-ink chart, and saying
        // "day" would quietly show different numbers than Home Assistant.
        Some(other) => {
            let label = format!("statistics-graph {}", other);
            skip(skipped, &label);
            return;
        }
    };
    // Home Assistant's own card shows 30 days unless told otherwise.
    let days = card
        .get("days_to_show")
        .and_then(Value::as_u64)
        .map(|d| d.clamp(2, 400) as u32)
        .unwrap_or(30);
    let types: Vec<&str> = card
        .get("stat_types")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    // Without `stat_types` Home Assistant lets the chart pick a type per
    // entity; that choice is made once the state_class is known, in `import`.
    let explicit = types.first().copied().filter(|s| matches!(*s, "change" | "mean" | "min" | "max" | "sum"));
    // One card carries one series: extra statistics are reported, not dropped.
    for extra in types.iter().skip(1) {
        let label = format!("statistics-graph {}", extra);
        skip(skipped, &label);
    }
    let title = card.get("title").and_then(Value::as_str).map(str::to_string);
    for item in list {
        match item.get("entity").and_then(Value::as_str).or_else(|| item.as_str()) {
            Some(entity) => out.push(CardConfig::Statistics {
                entity_id: entity.to_string(),
                title: title.clone(),
                period: period.to_string(),
                days,
                stat: explicit.unwrap_or("").to_string(),
                unit: None,
            }),
            None => skip(skipped, "statistics-graph"),
        }
    }
}

/// Split a view over as many pages as the screen needs. `used` keeps the page
/// ids unique across the whole import, so a later page never overwrites an
/// earlier one when the pages are merged into the dashboard.
fn paginate(
    title: &str,
    icon: &str,
    cards: Vec<CardConfig>,
    screen: (u32, u32),
    with_tabs: bool,
    used: &mut std::collections::HashSet<String>,
) -> Vec<PageConfig> {
    let (w, h) = screen;
    let mut pages = Vec::new();
    let mut chunk: Vec<CardConfig> = Vec::new();
    let mut part = 1;
    for card in cards {
        chunk.push(card);
        if flow_hidden_count(&chunk, w, h, with_tabs) > 0 && chunk.len() > 1 {
            let overflow = chunk.pop().expect("chunk has the card we just pushed");
            pages.push(page(title, part, icon, std::mem::take(&mut chunk), used));
            part += 1;
            chunk.push(overflow);
        }
    }
    if !chunk.is_empty() {
        pages.push(page(title, part, icon, chunk, used));
    }
    pages
}

fn page(title: &str, part: usize, icon: &str, cards: Vec<CardConfig>, used: &mut std::collections::HashSet<String>) -> PageConfig {
    let title = if part == 1 { title.to_string() } else { format!("{} {}", title, part) };
    let base: String = title
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();
    let base = if base.trim_matches('_').is_empty() { format!("page_{}", part) } else { base };
    let mut id = base.clone();
    let mut n = 2;
    while !used.insert(id.clone()) {
        id = format!("{}_{}", base, n);
        n += 1;
    }
    PageConfig { id, title, icon: icon.to_string(), grid: None, cards: crate::config::into_cards(cards), imported_from: None }
}

/// "living-room" -> "Living room"
fn humanize(path: &str) -> String {
    let text = path.replace(['-', '_'], " ");
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => text,
    }
}

fn entity_label(entity: &str) -> String {
    humanize(entity.split('.').nth(1).unwrap_or(entity))
}

/// Material Design Icon name -> one of the dashboard's own icons, if there is
/// a sensible one.
fn icon_name(icon: Option<&Value>) -> Option<String> {
    let raw = icon?.as_str()?.trim();
    let name = raw.strip_prefix("mdi:").unwrap_or(raw).replace('-', "_");
    let alias = match name.as_str() {
        "chart_line" | "chart_bar" | "chart_histogram" | "chart_areaspline" | "chart_timeline" | "poll" | "chart_donut" => {
            "progress"
        }
        "fire" | "radiator" => "flame",
        "water" | "water_percent" => "droplet",
        "flash" | "lightning_bolt" | "lightning" => "zap",
        "lightbulb_outline" | "ceiling_light" | "lamp" => "lightbulb_on",
        "power_plug" | "power_socket" => "plug",
        "washing_machine" => "washer",
        "air_conditioner" => "snowflake",
        "house" | "home_assistant" => "home",
        other => other,
    };
    crate::setup::ICONS.contains(&alias).then(|| alias.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::{SinkExt, StreamExt};
    use serde_json::json;
    use tokio_tungstenite::tungstenite::Message;

    /// A fake Home Assistant WebSocket endpoint: logs in, records the one
    /// request it receives and answers it with `result`.
    async fn fake_ha(result: Result<Value, &'static str>) -> (String, tokio::sync::oneshot::Receiver<Value>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
            ws.send(Message::Text(json!({ "type": "auth_required" }).to_string())).await.unwrap();
            let _ = ws.next().await;
            ws.send(Message::Text(json!({ "type": "auth_ok" }).to_string())).await.unwrap();
            if let Some(Ok(Message::Text(text))) = ws.next().await {
                let payload: Value = serde_json::from_str(&text).unwrap();
                let id = payload.get("id").cloned().unwrap_or(json!(1));
                let _ = tx.send(payload);
                let msg = match result {
                    Ok(reply) => json!({ "id": id, "type": "result", "success": true, "result": reply }),
                    Err(message) => json!({
                        "id": id,
                        "type": "result",
                        "success": false,
                        "error": { "code": "unauthorized", "message": message },
                    }),
                };
                ws.send(Message::Text(msg.to_string())).await.unwrap();
            }
            while ws.next().await.is_some() {}
        });
        (format!("http://127.0.0.1:{}", port), rx)
    }

    #[tokio::test]
    async fn fetch_config_asks_for_the_lovelace_config() {
        let (url, request) = fake_ha(Ok(json!({ "views": [{ "title": "Energie", "cards": [] }] }))).await;
        let config = fetch_config(&url, "t", Some("energie")).await.expect("the config comes back");
        let sent = request.await.expect("the request was recorded");
        assert_eq!(sent["type"], "lovelace/config");
        assert_eq!(sent["url_path"], "energie");
        assert_eq!(sent["force"], false);
        assert_eq!(config["views"][0]["title"], "Energie");
    }

    #[tokio::test]
    async fn dashboards_lists_what_home_assistant_returns() {
        let (url, request) = fake_ha(Ok(json!([
            { "url_path": "energie", "title": "Energie" },
            { "url_path": "", "title": "Overview" }
        ])))
        .await;
        let list = dashboards(&url, "t").await.expect("the list comes back");
        assert_eq!(request.await.unwrap()["type"], "lovelace/dashboards/list");
        // The default dashboard is always offered, and once only
        assert_eq!(list, vec![(String::new(), "Overview".to_string()), ("energie".to_string(), "Energie".to_string())]);
    }

    #[tokio::test]
    async fn dashboards_falls_back_to_the_default_when_listing_fails() {
        // Listing dashboards needs an admin token; a plain user still gets the default
        let (url, _request) = fake_ha(Err("unauthorized")).await;
        let list = dashboards(&url, "t").await.expect("the default dashboard is still offered");
        assert_eq!(list, vec![(String::new(), "Overview".to_string())]);
    }

    /// The Lovelace YAML of a real dashboard as Home Assistant returns it.
    fn as_config(yaml: &str) -> Value {
        let parsed: serde_yaml::Value = serde_yaml::from_str(yaml).expect("test YAML parses");
        serde_json::to_value(parsed).expect("YAML converts to JSON")
    }

    const ENERGIE: &str = r#"
views:
  - max_columns: 4
    path: energy
    title: Energy
    type: sections
    sections:
      - type: grid
        cards:
          - type: heading
            heading: Heating
            icon: mdi:sofa
          - type: tile
            entity: climate.living_room
            name: Living room
            features:
              - type: target-temperature
              - type: climate-hvac-modes
                style: dropdown
            grid_options:
              columns: 12
              rows: 3
          - type: tile
            entity: water_heater.hot_water
            name: Warm water
            features:
              - type: water-heater-operation-modes
                style: dropdown
            grid_options:
              columns: 12
              rows: 2
          - type: tile
            entity: climate.attic_thermostat
            name: Attic
            features:
              - type: target-temperature
            grid_options:
              columns: 12
              rows: 2
      - type: grid
        cards:
          - type: heading
            heading: Electricity price today
            icon: mdi:chart-line
          - type: picture
            image_entity: image.energy_prices_chart
            alt_text: Price per hour
            grid_options:
              columns: full
              rows: auto
          - type: heading
            heading: Hall thermometer
            icon: mdi:thermometer
          - type: tile
            entity: sensor.hall_temperature
            name: Temperature
            features:
              - type: trend-graph
            grid_options:
              columns: 12
              rows: 3
      - type: grid
        cards:
          - type: heading
            heading: Gas usage
            icon: mdi:fire
          - type: tile
            entity: sensor.gas_per_day
            name: Gas today
            icon: mdi:fire
            grid_options:
              columns: 6
              rows: auto
          - type: tile
            entity: sensor.gas_yesterday
            name: Gas yesterday
            icon: mdi:fire
            grid_options:
              columns: 6
              rows: auto
          - type: statistics-graph
            title: Daily gas usage (last 14 days)
            entities:
              - sensor.gas_meter
            chart_type: bar
            stat_types:
              - change
            period: day
            days_to_show: 14
            grid_options:
              columns: full
              rows: auto
      - type: grid
        cards:
          - type: heading
            heading: Gas usage long term
            icon: mdi:chart-bar
          - type: statistics-graph
            title: Weekly gas usage (last 60 days)
            entities:
              - sensor.gas_meter
            chart_type: bar
            stat_types:
              - change
            period: week
            days_to_show: 60
            grid_options:
              columns: full
              rows: auto
          - type: statistics-graph
            title: Monthly gas usage (last year)
            entities:
              - sensor.gas_meter
            chart_type: bar
            stat_types:
              - change
            period: month
            days_to_show: 365
            grid_options:
              columns: full
              rows: auto
"#;

    #[test]
    fn imports_a_sections_view() {
        let _guard = crate::i18n::TEST_LANGUAGE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::i18n::set_language("en");
        let result = import(&as_config(ENERGIE), (1072, 1448), false).expect("imports");
        let cards: Vec<&CardConfig> = result.pages.iter().flat_map(|p| p.cards.iter().map(|c| &c.kind)).collect();

        assert_eq!(result.pages[0].title, "Energy");
        assert_eq!(result.pages[0].icon, "home");
        // Every section keeps its heading, including the second one in a section
        let headings = cards.iter().filter(|c| matches!(c, CardConfig::Heading { .. })).count();
        assert_eq!(headings, 5);
        for title in ["Heating", "Electricity price today", "Hall thermometer", "Gas usage", "Gas usage long term"] {
            assert!(cards.iter().any(|c| matches!(c, CardConfig::Heading { title: t, .. } if t == title)), "{} missing", title);
        }
        assert!(cards.iter().any(|c| matches!(c, CardConfig::Climate { entity_id, .. } if entity_id == "climate.living_room")));
        assert!(cards.iter().any(|c| matches!(c, CardConfig::Climate { entity_id, .. } if entity_id == "climate.attic_thermostat")));
        assert!(cards.iter().any(|c| matches!(c, CardConfig::Sensor { entity_id, show_graph: true, .. } if entity_id == "sensor.hall_temperature")));
        assert!(cards.iter().any(|c| matches!(c, CardConfig::Sensor { entity_id, show_graph: false, .. } if entity_id == "sensor.gas_yesterday")));
        assert!(cards.iter().any(|c| matches!(c, CardConfig::Sensor { entity_id, show_graph: false, .. } if entity_id == "sensor.gas_per_day")));
        // The picture card is an image entity: fetched and drawn as a picture
        assert!(cards.iter().any(|c| matches!(c, CardConfig::Image { entity_id, .. } if entity_id == "image.energy_prices_chart")));
        // Three statistics charts, each with the period and range of its card
        let stats: Vec<&CardConfig> = cards.iter().copied().filter(|c| matches!(c, CardConfig::Statistics { .. })).collect();
        assert_eq!(stats.len(), 3);
        assert!(matches!(stats[0], CardConfig::Statistics { period, days: 14, .. } if period == "day"));
        assert!(matches!(stats[1], CardConfig::Statistics { period, days: 60, .. } if period == "week"));
        assert!(matches!(stats[2], CardConfig::Statistics { period, days: 365, .. } if period == "month"));
        // Everything the Kobo cannot draw is reported, not dropped silently
        assert_eq!(result.skipped, vec![Skipped { card: "water_heater".into(), count: 1 }]);
        // A view that needs more than one page keeps every card: 15 cards in
        // the dashboard, 1 of them skipped, so 14 land on two pages.
        assert_eq!(result.pages.len(), 2, "{} pages", result.pages.len());
        assert_eq!(result.pages[1].title, "Energy 2");
        assert_eq!(cards.len(), 14);
        for page in &result.pages {
            assert_eq!(hidden_card_count(page, 1072, 1448, false), 0, "{} overflows", page.title);
        }
    }

    #[test]
    fn a_strategy_dashboard_cannot_be_imported() {
        let config = serde_json::json!({ "strategy": { "type": "original-states" } });
        assert_eq!(import(&config, (1072, 1448), false).unwrap_err(), ImportError::Strategy);
    }

    #[test]
    fn stacks_are_flattened_and_unsupported_cards_counted() {
        let config = serde_json::json!({ "views": [{ "title": "Home", "cards": [
            { "type": "vertical-stack", "cards": [
                { "type": "light", "entity": "light.desk", "name": "Desk" },
                { "type": "custom:mushroom-template-card", "entity": "sensor.x" }
            ]},
            { "type": "gauge", "entity": "sensor.power" },
            { "type": "gauge", "entity": "sensor.power2" },
            { "type": "entities", "entities": ["switch.kettle", { "entity": "lock.front_door" }] }
        ]}]});
        let result = import(&config, (1072, 1448), false).expect("imports");
        let cards: Vec<&CardConfig> = result.pages.iter().flat_map(|p| p.cards.iter().map(|c| &c.kind)).collect();
        assert!(cards.iter().any(|c| matches!(c, CardConfig::Light { entity_id, .. } if entity_id == "light.desk")));
        assert!(cards.iter().any(|c| matches!(c, CardConfig::Switch { entity_id, .. } if entity_id == "switch.kettle")));
        assert_eq!(result.skipped[0], Skipped { card: "gauge".into(), count: 2 });
        assert!(result.skipped.iter().any(|s| s.card == "custom:mushroom-template-card"));
        assert!(result.skipped.iter().any(|s| s.card == "lock"));
    }

    #[test]
    fn a_long_view_is_split_over_pages() {
        let cards: Vec<Value> = (0..40)
            .map(|i| serde_json::json!({ "type": "light", "entity": format!("light.l{}", i) }))
            .collect();
        let config = serde_json::json!({ "views": [{ "title": "Alles", "cards": cards }] });
        let result = import(&config, (1072, 1448), false).expect("imports");
        assert!(result.pages.len() >= 3, "{} pages", result.pages.len());
        assert_eq!(result.pages[0].title, "Alles");
        assert_eq!(result.pages[1].title, "Alles 2");
        for page in &result.pages {
            assert_eq!(hidden_card_count(page, 1072, 1448, false), 0, "{} overflows", page.title);
        }
    }

    #[test]
    fn page_ids_stay_unique_across_views() {
        // Two views with the same title must not overwrite each other when the
        // editor merges them into the draft.
        let config = serde_json::json!({ "views": [
            { "title": "Huis", "cards": [{ "type": "light", "entity": "light.a" }] },
            { "title": "Huis", "cards": [{ "type": "light", "entity": "light.b" }] },
            { "title": "", "cards": [{ "type": "light", "entity": "light.c" }] }
        ]});
        let result = import(&config, (1072, 1448), false).expect("imports");
        let ids: Vec<&str> = result.pages.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, vec!["huis", "huis_2", "page_1"]);
        let mut unique = ids.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), ids.len(), "duplicate ids: {:?}", ids);
    }

    #[test]
    fn statistics_report_what_they_cannot_show() {
        let config = serde_json::json!({ "views": [{ "title": "Gas", "cards": [
            { "type": "statistics-graph", "entities": ["sensor.gas"], "period": "hour" },
            { "type": "statistics-graph", "entities": ["sensor.gas"], "period": "5minute" },
            { "type": "statistics-graph", "entities": ["sensor.gas"], "period": "year", "days_to_show": 400 },
            { "type": "statistics-graph", "entities": ["sensor.gas"], "stat_types": ["change", "max"] }
        ]}]});
        let result = import(&config, (1072, 1448), false).expect("imports");
        let stats: Vec<&CardConfig> = result.pages.iter().flat_map(|p| p.cards.iter().map(|c| &c.kind)).filter(|c| matches!(c, CardConfig::Statistics { .. })).collect();
        assert_eq!(stats.len(), 2);
        // A year of months is a month chart
        assert!(matches!(stats[0], CardConfig::Statistics { period, days: 400, .. } if period == "month"));
        // Without days_to_show a chart spans the month Home Assistant shows too
        assert!(matches!(stats[1], CardConfig::Statistics { days: 30, .. }));
        assert_eq!(
            result.skipped,
            vec![Skipped { card: "statistics-graph 5minute".into(), count: 1 }, Skipped { card: "statistics-graph hour".into(), count: 1 }, Skipped { card: "statistics-graph max".into(), count: 1 }]
        );
    }

    #[test]
    fn a_chart_without_a_type_follows_the_state_class() {
        let config = serde_json::json!({ "views": [{ "title": "Energy", "cards": [
            { "type": "statistics-graph", "entities": ["sensor.gas_meter"] },
            { "type": "statistics-graph", "entities": ["sensor.living_room_temperature"] },
            { "type": "statistics-graph", "entities": ["sensor.unknown"] }
        ]}]});
        let classes: BTreeMap<String, String> = [
            ("sensor.gas_meter".to_string(), "total_increasing".to_string()),
            ("sensor.living_room_temperature".to_string(), "measurement".to_string()),
        ]
        .into_iter()
        .collect();
        let result = import_with(&config, (1072, 1448), false, &classes).expect("imports");
        let stats: Vec<String> = result
            .pages
            .iter()
            .flat_map(|p| p.cards.iter().map(|c| &c.kind))
            .filter_map(|c| match c {
                CardConfig::Statistics { stat, .. } => Some(stat.clone()),
                _ => None,
            })
            .collect();
        // Only a meter has a `sum` to take a change from; everything else would
        // show an empty chart, so it gets the average Home Assistant also shows.
        assert_eq!(stats, vec!["change".to_string(), "mean".to_string(), "mean".to_string()]);
    }

    #[test]
    fn a_chart_with_an_explicit_type_keeps_it() {
        let config = serde_json::json!({ "views": [{ "title": "Energy", "cards": [
            { "type": "statistics-graph", "entities": ["sensor.living_room_temperature"], "stat_types": ["change"] }
        ]}]});
        let classes: BTreeMap<String, String> =
            [("sensor.living_room_temperature".to_string(), "measurement".to_string())].into_iter().collect();
        let result = import_with(&config, (1072, 1448), false, &classes).expect("imports");
        assert!(matches!(&result.pages[0].cards[0].kind, CardConfig::Statistics { stat, .. } if stat == "change"));
    }

    #[test]
    fn a_view_strategy_is_reported_as_such() {
        let config = serde_json::json!({ "views": [{ "title": "Home", "strategy": { "type": "original-states" } }] });
        assert_eq!(import(&config, (1072, 1448), false).unwrap_err(), ImportError::Strategy);
        let empty = serde_json::json!({ "views": [] });
        assert_eq!(import(&empty, (1072, 1448), false).unwrap_err(), ImportError::Empty);
    }

    #[test]
    fn an_entities_row_inherits_the_card_entity() {
        let config = serde_json::json!({ "views": [{ "title": "Home", "cards": [
            { "type": "entities", "entity": "sensor.power", "entities": [{ "type": "attribute", "attribute": "x" }] }
        ]}]});
        let result = import(&config, (1072, 1448), false).expect("imports");
        let cards: Vec<&CardConfig> = result.pages.iter().flat_map(|p| p.cards.iter().map(|c| &c.kind)).collect();
        assert!(cards.iter().any(|c| matches!(c, CardConfig::Sensor { entity_id, .. } if entity_id == "sensor.power")));
        assert!(result.skipped.is_empty(), "{:?}", result.skipped);
    }

    #[test]
    fn tabs_shrink_the_page() {
        // With a tab bar there is less room, so the same view needs more pages.
        let cards: Vec<Value> = (0..12)
            .map(|i| serde_json::json!({ "type": "light", "entity": format!("light.l{}", i) }))
            .collect();
        let config = serde_json::json!({ "views": [{ "title": "Alles", "cards": cards }] });
        let without = import(&config, (1072, 1448), false).expect("imports").pages.len();
        let with = import(&config, (1072, 1448), true).expect("imports").pages.len();
        assert!(with >= without, "{} pages with tabs, {} without", with, without);
    }

    #[test]
    fn icons_and_paths_are_mapped() {
        assert_eq!(icon_name(Some(&serde_json::json!("mdi:chart-line"))).as_deref(), Some("progress"));
        assert_eq!(icon_name(Some(&serde_json::json!("mdi:sofa"))).as_deref(), Some("sofa"));
        assert_eq!(icon_name(Some(&serde_json::json!("mdi:washing-machine"))).as_deref(), Some("washer"));
        assert_eq!(icon_name(Some(&serde_json::json!("mdi:something-unknown"))), None);
        assert_eq!(humanize("living-room"), "Living room");
    }
}