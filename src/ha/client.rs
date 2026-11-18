use crate::config::{AppConfig, CardConfig, HomeAssistantConfig};
use crate::ha::mock::generate_mock_states;
use crate::ha::models::EntityState;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// Attribute key under which recent numeric history is stored for sparklines
pub const HISTORY_ATTR: &str = "__history";

/// Forecasts and graph history change slowly; fetching them rarely saves power.
const HISTORY_REFRESH: Duration = Duration::from_secs(1800);
/// Pictures change at most daily (a rendered chart), so fetch them rarely.
const IMAGE_REFRESH: Duration = Duration::from_secs(6 * 3600);
/// A failed picture fetch (a restarting chart, a slow Home Assistant) is retried
/// after this instead of leaving the card empty for six hours.
const IMAGE_RETRY: Duration = Duration::from_secs(300);
/// A camera picture is a snapshot of now: fetched again much sooner.
const CAMERA_REFRESH: Duration = Duration::from_secs(600);
/// Largest picture file kept in memory.
const MAX_IMAGE_BYTES: usize = 10_000_000;
const HISTORY_SPAN_HOURS: i64 = 24;
const HISTORY_POINTS: usize = 48;

#[derive(Clone)]
pub struct HaClient {
    config: HomeAssistantConfig,
    pub(crate) client: reqwest::Client,
    states: Arc<RwLock<HashMap<String, EntityState>>>,
    is_live: Arc<RwLock<bool>>,
    /// Weather entities whose forecast must be fetched via weather.get_forecasts
    weather_entities: Arc<Vec<String>>,
    /// Sensors shown with a graph
    history_entities: Arc<Vec<String>>,
    /// Statistics charts (Lovelace `statistics-graph`)
    stat_requests: Arc<Vec<StatRequest>>,
    /// `image.*` entities shown by image cards
    image_entities: Arc<Vec<String>>,
    /// Thermostats drawn with a graph, and how many hours back
    climate_graphs: Arc<Vec<(String, u32)>>,
    /// Their pictures, as fetched from `/api/image_proxy`
    images: Arc<RwLock<HashMap<String, Arc<Vec<u8>>>>>,
    /// When the pictures were last fetched
    last_image_fetch: Arc<std::sync::Mutex<Option<Instant>>>,
    /// State of each `image` entity when its picture was fetched: the state
    /// is the time the picture changed, so a new one is fetched right away.
    image_versions: Arc<std::sync::Mutex<HashMap<String, String>>>,
    /// True when the last picture fetch brought nothing back: retry sooner.
    image_retry: Arc<AtomicBool>,
    /// True while a picture fetch runs, so refreshes never stack up.
    images_running: Arc<AtomicBool>,
    /// True while forecasts, history and statistics are being fetched.
    extras_running: Arc<AtomicBool>,
    /// Asks the dashboard to redraw once new pictures have arrived: they are
    /// fetched in the background, after the refresh that started them.
    redraw: Arc<std::sync::Mutex<Option<tokio::sync::mpsc::Sender<()>>>>,
    extras: Arc<RwLock<Extras>>,
    last_update: Arc<RwLock<Option<chrono::DateTime<chrono::Utc>>>>,
    /// Every entity the dashboard shows (the WebSocket subscription list)
    pub(crate) entity_ids: Arc<Vec<String>>,
    /// True while the WebSocket subscription delivers live updates
    pub(crate) ws_live: Arc<AtomicBool>,
    /// Wakes the WebSocket task out of its reconnect backoff
    pub(crate) ws_nudge: Arc<tokio::sync::Notify>,
    /// Ends the WebSocket session in progress (Wi-Fi is going off)
    pub(crate) ws_drop: Arc<tokio::sync::Notify>,
    /// Last battery report sent to Home Assistant
    pub(crate) battery_sent: Arc<std::sync::Mutex<Option<crate::ha::battery::Sent>>>,
}

/// One statistics chart: which statistic, over which range.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct StatRequest {
    pub entity_id: String,
    /// "5minute", "hour", "day", "week" or "month"
    pub period: String,
    /// The range, see `statistics_card::window_hours`
    pub hours: u32,
    /// "change", "mean", "min", "max" or "sum"
    pub stat: String,
}

#[derive(Default)]
struct Extras {
    forecasts: HashMap<String, Value>,
    history: HashMap<String, Vec<f32>>,
    /// Thermostat graphs: `[[ms, room, target, 1 heating / 2 cooling / 0], ...]`
    climate: HashMap<String, Value>,
    last_climate_fetch: Option<Instant>,
    /// Per entity, a map of chart key -> `[[start_ms, value], ...]`
    stats: HashMap<String, Value>,
    last_history_fetch: Option<Instant>,
    last_forecast_fetch: Option<Instant>,
    last_stats_fetch: Option<Instant>,
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

        // No idle connections kept: Wi-Fi goes off between sleep-mode
        // refreshes, and a pooled connection from before is dead afterwards.
        let client = reqwest::Client::builder()
            .pool_max_idle_per_host(0)
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
            stat_requests: Arc::new(Vec::new()),
            image_entities: Arc::new(Vec::new()),
            climate_graphs: Arc::new(Vec::new()),
            images: Arc::new(RwLock::new(HashMap::new())),
            last_image_fetch: Arc::new(std::sync::Mutex::new(None)),
            image_versions: Arc::new(std::sync::Mutex::new(HashMap::new())),
            image_retry: Arc::new(AtomicBool::new(false)),
            images_running: Arc::new(AtomicBool::new(false)),
            extras_running: Arc::new(AtomicBool::new(false)),
            redraw: Arc::new(std::sync::Mutex::new(None)),
            extras: Arc::new(RwLock::new(Extras::default())),
            last_update: Arc::new(RwLock::new(None)),
            entity_ids: Arc::new(Vec::new()),
            ws_live: Arc::new(AtomicBool::new(false)),
            ws_nudge: Arc::new(tokio::sync::Notify::new()),
            ws_drop: Arc::new(tokio::sync::Notify::new()),
            battery_sent: Arc::new(std::sync::Mutex::new(None)),
        }
    }

    /// Collect entities that need extra API calls (forecasts, history,
    /// statistics) from the pages.
    pub fn for_app(app: &AppConfig) -> Self {
        let mut client = Self::new(app.home_assistant.clone());
        let mut weather = Vec::new();
        let mut history = Vec::new();
        let mut stats = Vec::new();
        let mut images = Vec::new();
        let mut climate = Vec::new();
        let mut all = Vec::new();
        for card in app.pages.iter().flat_map(|p| p.cards.iter()).map(|c| &c.kind) {
            match card {
                CardConfig::Weather { entity_id, .. } => weather.push(entity_id.clone()),
                CardConfig::Sensor { entity_id, show_graph: true, .. } => history.push(entity_id.clone()),
                CardConfig::Image { entity_id, .. } => images.push(entity_id.clone()),
                CardConfig::Climate { entity_id, show_graph: true, graph_hours, .. } => {
                    climate.push((entity_id.clone(), graph_hours.unwrap_or(24).clamp(1, 24 * 14)))
                }
                CardConfig::Statistics { entity_id, period, days, hours, stat, .. } => stats.push(StatRequest {
                    entity_id: entity_id.clone(),
                    period: period.clone(),
                    hours: crate::ui::widgets::statistics_card::window_hours(*days, *hours),
                    stat: stat.clone(),
                }),
                _ => {}
            }
            match card {
                CardConfig::Weather { entity_id, .. }
                | CardConfig::Light { entity_id, .. }
                | CardConfig::Switch { entity_id, .. }
                | CardConfig::Climate { entity_id, .. }
                | CardConfig::Sensor { entity_id, .. }
                | CardConfig::Statistics { entity_id, .. }
                | CardConfig::Image { entity_id, .. }
                | CardConfig::MediaPlayer { entity_id, .. }
                | CardConfig::Cover { entity_id, .. }
                | CardConfig::Scene { entity_id, .. } => all.push(entity_id.clone()),
                CardConfig::Progress { entity_id, secondary_entity, .. } => {
                    all.push(entity_id.clone());
                    all.extend(secondary_entity.clone());
                }
                CardConfig::SensorGrid { sensors, .. } => all.extend(sensors.iter().map(|s| s.entity_id.clone())),
                CardConfig::Glance { entities, .. } => all.extend(entities.iter().map(|s| s.entity_id.clone())),
                CardConfig::ScenesRow { scenes, .. } => all.extend(scenes.iter().map(|s| s.entity_id.clone())),
                CardConfig::Heading { .. } => {}
            }
        }
        weather.dedup();
        history.dedup();
        stats.dedup();
        images.dedup();
        all.retain(|e| !e.is_empty());
        all.sort();
        all.dedup();
        client.weather_entities = Arc::new(weather);
        client.history_entities = Arc::new(history);
        client.stat_requests = Arc::new(stats);
        client.image_entities = Arc::new(images);
        climate.sort();
        climate.dedup();
        client.climate_graphs = Arc::new(climate);
        client.entity_ids = Arc::new(all);
        client
    }

    /// Reconnect the live updates now (after Wi-Fi came back).
    pub fn reconnect_now(&self) {
        self.ws_nudge.notify_one();
    }

    /// Wi-Fi is going off: end the live session instead of keeping a dead
    /// socket that still counts as "live" (and stops polling) until a ping
    /// times out minutes later.
    pub async fn network_down(&self) {
        self.ws_drop.notify_waiters();
        self.ws_live.store(false, Ordering::Relaxed);
        *self.is_live.write().await = false;
    }

    pub fn is_live_push(&self) -> bool {
        self.ws_live.load(Ordering::Relaxed)
    }

    pub(crate) fn config(&self) -> &HomeAssistantConfig {
        &self.config
    }

    pub(crate) fn states_handle(&self) -> &Arc<RwLock<HashMap<String, EntityState>>> {
        &self.states
    }

    pub(crate) async fn set_live(&self, live: bool) {
        *self.is_live.write().await = live;
        if live {
            *self.last_update.write().await = Some(chrono::Utc::now());
        }
    }

    /// Forecast/history attributes are fetched separately; copy them onto a state.
    pub(crate) async fn decorate(&self, state: &mut EntityState) {
        let extras = self.extras.read().await;
        if let Some(fc) = extras.forecasts.get(&state.entity_id) {
            state.attributes.insert("forecast".to_string(), fc.clone());
        }
        if let Some(h) = extras.history.get(&state.entity_id) {
            state.attributes.insert(HISTORY_ATTR.to_string(), serde_json::json!(h));
        }
        if let Some(c) = extras.climate.get(&state.entity_id) {
            state.attributes.insert(crate::ui::widgets::climate_card::CLIMATE_HISTORY_ATTR.to_string(), c.clone());
        }
        if let Some(stats) = extras.stats.get(&state.entity_id) {
            state.attributes.insert(crate::ui::widgets::statistics_card::STATS_ATTR.to_string(), stats.clone());
        }
    }

    /// With live updates running, only the slow extras need polling.
    /// True when something was fetched, so a redraw is worth it.
    pub async fn refresh_extras_only(&self) -> bool {
        if self.is_demo() {
            return false;
        }
        let fetched = self.refresh_extras().await;
        let ids: Vec<String> = self.states.read().await.keys().cloned().collect();
        for id in ids {
            let current = self.states.read().await.get(&id).cloned();
            if let Some(mut state) = current {
                self.decorate(&mut state).await;
                self.states.write().await.insert(id, state);
            }
        }
        fetched
    }

    fn is_demo(&self) -> bool {
        self.config.token.is_empty()
    }

    pub(crate) fn url(&self, path: &str) -> String {
        format!("{}{}", crate::ha::base_url(&self.config.url).0, path)
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
                // Keep only what the dashboard shows (or everything, e.g. in the setup tool).
                let wanted = |id: &str| self.entity_ids.is_empty() || self.entity_ids.iter().any(|e| e == id);
                let mut kept = Vec::new();
                for mut state in raw_states.into_iter().filter(|s| wanted(&s.entity_id)) {
                    self.decorate(&mut state).await;
                    kept.push(state);
                }
                let mut map = self.states.write().await;
                map.clear();
                for state in kept {
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
        crate::ha::read_json(resp).await
    }

    /// Forecasts, history, statistics and pictures. One run at a time: the
    /// WebSocket asks for one as soon as it connects, which can overlap with
    /// the regular refresh.
    async fn refresh_extras(&self) -> bool {
        if self.extras_running.swap(true, Ordering::Relaxed) {
            return false;
        }
        let fetched = self.refresh_extras_inner().await;
        self.extras_running.store(false, Ordering::Relaxed);
        fetched
    }

    /// Forecasts (HA 2024.3+ removed the `forecast` attribute) and sensor history.
    /// True when forecasts, history or statistics were fetched (pictures
    /// arrive on their own and ask for their own redraw).
    async fn refresh_extras_inner(&self) -> bool {
        let (need_forecast, need_history, need_stats, need_climate) = {
            let ex = self.extras.read().await;
            let stale = |t: Option<Instant>| t.map(|t| t.elapsed() > HISTORY_REFRESH).unwrap_or(true);
            (stale(ex.last_forecast_fetch), stale(ex.last_history_fetch), stale(ex.last_stats_fetch), stale(ex.last_climate_fetch))
        };
        let mut fetched = false;
        if need_climate && !self.climate_graphs.is_empty() {
            fetched = true;
            match self.fetch_climate_history().await {
                Ok(c) => {
                    let mut ex = self.extras.write().await;
                    ex.climate = c;
                    ex.last_climate_fetch = Some(Instant::now());
                }
                Err(e) => tracing::warn!("Thermostat history fetch failed: {}", e),
            }
        }

        if need_forecast && !self.weather_entities.is_empty() {
            fetched = true;
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
            fetched = true;
            match self.fetch_history().await {
                Ok(history) => {
                    let mut ex = self.extras.write().await;
                    ex.history = history;
                    ex.last_history_fetch = Some(Instant::now());
                }
                Err(e) => tracing::warn!("History fetch failed: {}", e),
            }
        }

        if need_stats && !self.stat_requests.is_empty() {
            fetched = true;
            match self.fetch_statistics().await {
                Ok(stats) => {
                    let mut ex = self.extras.write().await;
                    ex.stats = stats;
                    ex.last_stats_fetch = Some(Instant::now());
                }
                Err(e) => tracing::warn!("Statistics fetch failed: {}", e),
            }
        }

        // Pictures are fetched off the way: they are not needed for the state
        // refresh, and a slow image proxy must not eat its time budget.
        let cameras = self.image_entities.iter().any(|e| e.starts_with("camera."));
        let stale_images = self
            .last_image_fetch
            .lock()
            .map(|t| {
                let wait = if self.image_retry.load(Ordering::Relaxed) {
                    IMAGE_RETRY
                } else if cameras {
                    CAMERA_REFRESH
                } else {
                    IMAGE_REFRESH
                };
                t.map(|t| t.elapsed() > wait).unwrap_or(true)
            })
            .unwrap_or(false);
        let changed = {
            let states = self.states.read().await;
            let versions = self.image_versions.lock().map(|v| v.clone()).unwrap_or_default();
            self.image_entities
                .iter()
                .filter(|e| e.starts_with("image."))
                .any(|e| states.get(e).map(|s| &s.state) != versions.get(e))
        };
        let stale_images = stale_images || changed;
        if stale_images
            && !self.image_entities.is_empty()
            && !self.images_running.swap(true, Ordering::Relaxed)
        {
            let this = self.clone();
            tokio::spawn(async move {
                this.refresh_images().await;
                this.images_running.store(false, Ordering::Relaxed);
            });
        }
        fetched
    }

    /// Fetch the pictures of the dashboard's `image` entities and remember when.
    /// Nothing came back (a chart being regenerated, a restarting Home
    /// Assistant): remember that too, so the next try is a few minutes away.
    async fn refresh_images(&self) {
        // What each picture's entity said before the fetch: a picture that
        // changes during it is fetched again next time.
        let seen: HashMap<String, String> = {
            let states = self.states.read().await;
            self.image_entities.iter().filter_map(|e| Some((e.clone(), states.get(e)?.state.clone()))).collect()
        };
        let (images, unreachable) = self.fetch_images().await;
        // Remembered for a picture that failed too: it is tried again when its
        // entity changes or the retry time is up, not on every refresh
        if let Ok(mut v) = self.image_versions.lock() {
            v.extend(seen);
        }
        let empty = images.is_empty();
        if !empty {
            let mut map = self.images.write().await;
            for (entity, bytes) in images {
                map.insert(entity, bytes);
            }
        }
        self.image_retry.store(empty, Ordering::Relaxed);
        if let Ok(mut t) = self.last_image_fetch.lock() {
            // Home Assistant was not reachable at all (no Wi-Fi yet): try again
            // with the next refresh instead of in a few minutes.
            *t = if unreachable && empty { None } else { Some(Instant::now()) };
        }
        if !empty {
            let tx = self.redraw.lock().ok().and_then(|r| r.clone());
            if let Some(tx) = tx {
                let _ = tx.try_send(());
            }
        }
    }

    /// Where to ask for a redraw when pictures arrive in the background.
    pub fn set_redraw(&self, tx: tokio::sync::mpsc::Sender<()>) {
        if let Ok(mut r) = self.redraw.lock() {
            *r = Some(tx);
        }
    }

    /// Ask for the pictures again on the next refresh (a tap on the screen, or
    /// a test in the setup tool).
    pub fn force_images(&self) {
        if let Ok(mut t) = self.last_image_fetch.lock() {
            *t = None;
        }
    }

    /// The pictures of the dashboard's `image` entities, fetched once in a
    /// while: they are rendered by Home Assistant, not by this dashboard.
    /// Also says whether a request could not reach Home Assistant at all.
    async fn fetch_images(&self) -> (Vec<(String, Arc<Vec<u8>>)>, bool) {
        let mut out = Vec::new();
        let mut unreachable = false;
        for entity in self.image_entities.iter() {
            // A camera's picture is a snapshot taken for the request
            let proxy = if entity.starts_with("camera.") { "camera_proxy" } else { "image_proxy" };
            let url = self.url(&format!("/api/{}/{}", proxy, entity));
            match self.client.get(url).send().await {
                Ok(resp) if resp.status().is_success() => match resp.bytes().await {
                    // A file this big is far beyond the 4K the Kobo decodes anyway.
                    Ok(bytes) if bytes.len() > MAX_IMAGE_BYTES => {
                        tracing::warn!("Image {} is {} MB, too large for the Kobo: skipped", entity, bytes.len() / 1_000_000)
                    }
                    Ok(bytes) => out.push((entity.clone(), Arc::new(bytes.to_vec()))),
                    Err(e) => tracing::warn!("Image {} could not be read: {}", entity, e),
                },
                Ok(resp) => tracing::warn!("Image {} failed: HTTP {}", entity, resp.status()),
                Err(e) => {
                    unreachable = true;
                    tracing::warn!("Image {} failed: {}", entity, e)
                }
            }
        }
        (out, unreachable)
    }

    /// Pictures for `image` cards, keyed by entity id.
    pub async fn images(&self) -> HashMap<String, Arc<Vec<u8>>> {
        self.images.read().await.clone()
    }

    /// Home Assistant statistics for the charts on the pages. These live on the
    /// WebSocket API; the REST API has no equivalent.
    async fn fetch_statistics(&self) -> Result<HashMap<String, Value>, Box<dyn std::error::Error + Send + Sync>> {
        let end = chrono::Utc::now();
        let stamp = |t: chrono::DateTime<chrono::Utc>| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        // One request per period/statistic combination: the command takes one
        // period and one set of types for any number of entities.
        let mut groups: std::collections::BTreeMap<(String, u32, String), Vec<String>> = std::collections::BTreeMap::new();
        for req in self.stat_requests.iter() {
            // Home Assistant accepts only these periods and types; a typo in the
            // config would otherwise fail the whole batch and empty every chart.
            if !matches!(req.period.as_str(), "5minute" | "hour" | "day" | "week" | "month" | "year") {
                tracing::warn!("Unknown statistics period '{}', skipped", req.period);
                continue;
            }
            if !matches!(req.stat.as_str(), "change" | "last_reset" | "max" | "mean" | "min" | "state" | "sum") {
                tracing::warn!("Unknown statistics type '{}', skipped", req.stat);
                continue;
            }
            groups
                .entry((req.period.clone(), req.hours, req.stat.clone()))
                .or_default()
                .push(req.entity_id.clone());
        }

        if groups.is_empty() {
            return Ok(HashMap::new());
        }
        let mut out: HashMap<String, serde_json::Map<String, Value>> = HashMap::new();
        let mut plan = Vec::new();
        let mut payloads = Vec::new();
        for ((period, hours, stat), mut ids) in groups {
            ids.sort();
            ids.dedup();
            let start = end - chrono::Duration::hours(hours as i64);
            let id = payloads.len() as u64 + 1;
            payloads.push(serde_json::json!({
                "id": id,
                "type": "recorder/statistics_during_period",
                "start_time": stamp(start),
                "end_time": stamp(end),
                "statistic_ids": ids,
                "period": period.clone(),
                "types": [stat.clone()],
            }));
            plan.push((period, hours, stat, ids));
        }

        let results = crate::ha::ws::requests(&self.config.url, &self.config.token, payloads).await?;
        for ((period, hours, stat, _), result) in plan.iter().cloned().zip(results) {
            let Some(by_entity) = result.as_object() else { continue };
            for (entity_id, rows) in by_entity {
                let series: Vec<(f64, f64)> = rows
                    .as_array()
                    .map(|rows| {
                        rows.iter()
                            .filter_map(|row| {
                                let start = row.get("start")?.as_f64()?;
                                let value = row.get(stat.as_str())?.as_f64()?;
                                Some((start, value))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                if series.is_empty() {
                    continue;
                }
                out.entry(entity_id.clone()).or_default().insert(
                    crate::ui::widgets::statistics_card::stat_key(&period, hours, &stat),
                    serde_json::json!(series),
                );
            }
        }
        // Entities without long-term statistics (a `number`, a sensor without
        // a state_class) are charted from their recorded history instead
        let missing: Vec<(String, String, u32, String)> = plan
            .iter()
            .flat_map(|(period, hours, stat, ids)| {
                let key = crate::ui::widgets::statistics_card::stat_key(period, *hours, stat);
                let out = &out;
                ids.iter()
                    .filter(move |id| !out.get(*id).is_some_and(|c| c.contains_key(&key)))
                    .map(move |id| (id.clone(), period.clone(), *hours, stat.clone()))
            })
            .collect();
        if !missing.is_empty() {
            match self.fetch_history_series(&missing).await {
                Ok(series) => {
                    for (entity, period, hours, stat) in &missing {
                        let Some(points) = series.get(entity) else { continue };
                        let start = (end - chrono::Duration::hours(*hours as i64)).timestamp_millis() as f64;
                        let buckets = bucket_history(points, start, end.timestamp_millis() as f64, period, stat);
                        if !buckets.is_empty() {
                            out.entry(entity.clone()).or_default().insert(
                                crate::ui::widgets::statistics_card::stat_key(period, *hours, stat),
                                serde_json::json!(buckets),
                            );
                        }
                    }
                }
                Err(e) => tracing::warn!("History for charts failed: {}", e),
            }
        }
        Ok(out.into_iter().map(|(entity, charts)| (entity, Value::Object(charts))).collect())
    }

    /// Room temperature, target and heating of the thermostats with a graph,
    /// from their recorded history with attributes, at most a few hundred
    /// points each.
    async fn fetch_climate_history(&self) -> Result<HashMap<String, Value>, Box<dyn std::error::Error + Send + Sync>> {
        let hours = self.climate_graphs.iter().map(|c| c.1).max().unwrap_or(24);
        let ids: Vec<&str> = self.climate_graphs.iter().map(|c| c.0.as_str()).collect();
        let start = chrono::Utc::now() - chrono::Duration::hours(hours as i64);
        let resp = self
            .client
            .get(self.url(&format!("/api/history/period/{}", start.format("%Y-%m-%dT%H:%M:%SZ"))))
            // Attribute changes (the room temperature) are what the graph shows
            .query(&[("filter_entity_id", ids.join(",")), ("significant_changes_only", "0".to_string())])
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(format!("HTTP {}", resp.status()).into());
        }
        let series: Vec<Vec<Value>> = crate::ha::read_json(resp).await?;
        let mut out = HashMap::new();
        for points in series {
            let Some(entity) = points.first().and_then(|p| p.get("entity_id")).and_then(Value::as_str).map(str::to_string) else { continue };
            let want = self.climate_graphs.iter().find(|c| c.0 == entity).map(|c| c.1).unwrap_or(hours);
            let from = (chrono::Utc::now() - chrono::Duration::hours(want as i64)).timestamp_millis() as f64;
            let rows: Vec<[f64; 4]> = points
                .iter()
                .filter_map(|p| {
                    let t = p.get("last_updated").or_else(|| p.get("last_changed"))?.as_str()?;
                    let t = (chrono::DateTime::parse_from_rfc3339(t).ok()?.timestamp_millis() as f64).max(from);
                    let a = p.get("attributes")?;
                    let room = a.get("current_temperature")?.as_f64()?;
                    let target = a.get("temperature").and_then(Value::as_f64).unwrap_or(f64::NAN);
                    let action = match a.get("hvac_action").and_then(Value::as_str) {
                        Some("heating") => 1.0,
                        Some("cooling") => 2.0,
                        _ => 0.0,
                    };
                    Some([t, room, target, action])
                })
                .collect();
            out.insert(entity, serde_json::json!(thin_steps(&rows, 400)));
        }
        Ok(out)
    }

    /// Recorded numeric states of some entities over the longest range they
    /// need (at most a month: the history API returns every change), as
    /// (milliseconds, value), oldest first.
    async fn fetch_history_series(
        &self,
        wanted: &[(String, String, u32, String)],
    ) -> Result<HashMap<String, Vec<(f64, f64)>>, Box<dyn std::error::Error + Send + Sync>> {
        let hours = wanted.iter().map(|w| w.2).max().unwrap_or(24).min(31 * 24);
        let mut ids: Vec<&str> = wanted.iter().map(|w| w.0.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        let start = chrono::Utc::now() - chrono::Duration::hours(hours as i64);
        let resp = self
            .client
            .get(self.url(&format!("/api/history/period/{}", start.format("%Y-%m-%dT%H:%M:%SZ"))))
            .query(&[("filter_entity_id", ids.join(",")), ("minimal_response", String::new()), ("no_attributes", String::new())])
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(format!("HTTP {}", resp.status()).into());
        }
        let series: Vec<Vec<Value>> = crate::ha::read_json(resp).await?;
        let mut out = HashMap::new();
        for (idx, points) in series.into_iter().enumerate() {
            let entity = points
                .first()
                .and_then(|p| p.get("entity_id"))
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .or_else(|| ids.get(idx).map(|s| s.to_string()));
            let values: Vec<(f64, f64)> = points
                .iter()
                .filter_map(|p| {
                    let t = p.get("last_changed")?.as_str()?;
                    let t = chrono::DateTime::parse_from_rfc3339(t).ok()?.timestamp_millis() as f64;
                    Some((t, p.get("state")?.as_str()?.parse::<f64>().ok()?))
                })
                .collect();
            if let Some(entity) = entity {
                out.insert(entity, values);
            }
        }
        Ok(out)
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
        let body: Value = crate::ha::read_json(resp).await?;
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
        let series: Vec<Vec<Value>> = crate::ha::read_json(resp).await?;
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
            // Live updates deliver the new state by themselves.
            if !self.is_live_push() {
                tokio::time::sleep(Duration::from_millis(300)).await;
                let _ = self.refresh_states().await;
            }
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

/// At most `max` rows of a step series: the last row of each equal slice
/// of time stands for it, and a change of heating is never dropped.
fn thin_steps(rows: &[[f64; 4]], max: usize) -> Vec<[f64; 4]> {
    if rows.len() <= max {
        return rows.iter().map(|r| r.map(|v| if v.is_nan() { -999.0 } else { v })).collect();
    }
    let (t0, t1) = (rows[0][0], rows[rows.len() - 1][0]);
    let slice = ((t1 - t0) / max as f64).max(1.0);
    let mut out: Vec<[f64; 4]> = Vec::new();
    for r in rows {
        let r = r.map(|v| if v.is_nan() { -999.0 } else { v });
        match out.last_mut() {
            Some(last) if ((r[0] - t0) / slice).floor() == ((last[0] - t0) / slice).floor() && last[3] == r[3] => *last = [last[0], r[1], r[2], r[3]],
            _ => out.push(r),
        }
    }
    out
}

/// Start of the period of `kind` that holds `t` (milliseconds), in local
/// time as Home Assistant's statistics periods are.
fn period_start(t: f64, kind: &str) -> f64 {
    use chrono::{Datelike, Local, TimeZone, Timelike};
    let Some(at) = Local.timestamp_millis_opt(t as i64).single() else { return t };
    let day = |d: chrono::NaiveDate| Local.from_local_datetime(&d.and_hms_opt(0, 0, 0).unwrap_or_default()).earliest();
    let start = match kind {
        "5minute" => at.with_minute(at.minute() / 5 * 5).and_then(|a| a.with_second(0)).and_then(|a| a.with_nanosecond(0)),
        "hour" => at.with_minute(0).and_then(|a| a.with_second(0)).and_then(|a| a.with_nanosecond(0)),
        "week" => day(at.date_naive() - chrono::Duration::days(at.weekday().num_days_from_monday() as i64)),
        "month" => at.date_naive().with_day(1).and_then(day),
        _ => day(at.date_naive()),
    };
    start.map(|s| s.timestamp_millis() as f64).unwrap_or(t)
}

/// Start of the period after the one starting at `start`
fn next_period(start: f64, kind: &str) -> f64 {
    match kind {
        "5minute" => start + 300_000.0,
        "hour" => start + 3_600_000.0,
        // Days, weeks and months: past daylight saving changes, so the next
        // one is found from a point well inside it
        "week" => period_start(start + 8.0 * 86_400_000.0, "week"),
        "month" => period_start(start + 32.0 * 86_400_000.0, "month"),
        _ => period_start(start + 1.5 * 86_400_000.0, "day"),
    }
}

/// Recorded states turned into statistics like Home Assistant's: one value
/// per period from `from` to `to` (milliseconds), the mean, min or max of
/// the states during it (a state lasts until the next), or for `change`
/// how much it rose. Periods before the first state are left out.
pub fn bucket_history(points: &[(f64, f64)], from: f64, to: f64, kind: &str, stat: &str) -> Vec<(f64, f64)> {
    let mut out = Vec::new();
    if points.is_empty() {
        return out;
    }
    let mut start = period_start(from, kind);
    let mut k = 0;
    // The state when a period starts: the last one recorded before it
    let mut current: Option<f64> = None;
    while start < to && out.len() < 2000 {
        let end = next_period(start, kind);
        while k < points.len() && points[k].0 < start {
            current = Some(points[k].1);
            k += 1;
        }
        let mut values: Vec<f64> = current.into_iter().collect();
        let first = current;
        let mut j = k;
        while j < points.len() && points[j].0 < end {
            values.push(points[j].1);
            j += 1;
        }
        if !values.is_empty() {
            let v = match stat {
                "min" => values.iter().copied().fold(f64::INFINITY, f64::min),
                "max" => values.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                "change" => values.last().copied().unwrap_or(0.0) - first.unwrap_or(values[0]),
                _ => values.iter().sum::<f64>() / values.len() as f64,
            };
            out.push((start, v));
        }
        start = end;
    }
    out
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

#[cfg(test)]
mod bucket_tests {
    use super::*;

    #[test]
    fn history_becomes_one_value_per_period() {
        let h = 3_600_000.0;
        let t0 = period_start(1_763_000_000_000.0, "hour");
        // 20 at the start, 22 half way through the first hour, 18 in the third
        let points = [(t0 - h, 20.0), (t0 + h / 2.0, 22.0), (t0 + 2.5 * h, 18.0)];
        let mean = bucket_history(&points, t0, t0 + 3.0 * h, "hour", "mean");
        assert_eq!(mean.iter().map(|p| p.0).collect::<Vec<_>>(), vec![t0, t0 + h, t0 + 2.0 * h]);
        assert_eq!(mean[0].1, 21.0);
        // A state lasts: the second hour carries 22
        assert_eq!(mean[1].1, 22.0);
        assert_eq!(bucket_history(&points, t0, t0 + 3.0 * h, "hour", "min")[2].1, 18.0);
        assert_eq!(bucket_history(&points, t0, t0 + 3.0 * h, "hour", "max")[0].1, 22.0);
        assert_eq!(bucket_history(&points, t0, t0 + 3.0 * h, "hour", "change")[0].1, 2.0);
        // Before the first state there is nothing to show
        assert!(bucket_history(&[(t0 + 2.5 * h, 1.0)], t0, t0 + 3.0 * h, "hour", "mean").len() == 1);
        assert!(bucket_history(&[], t0, t0 + h, "hour", "mean").is_empty());
    }

    #[test]
    fn days_weeks_and_months_follow_the_calendar() {
        let t = 1_763_000_000_000.0;
        for kind in ["5minute", "hour", "day", "week", "month"] {
            let s = period_start(t, kind);
            let n = next_period(s, kind);
            assert!(s <= t && t < n, "{kind}: {s} {t} {n}");
            assert_eq!(period_start(n, kind), n, "{kind}");
        }
    }
}
