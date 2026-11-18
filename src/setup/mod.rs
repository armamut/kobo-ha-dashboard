//! Desktop setup tool: a local web app to connect Home Assistant, design the
//! dashboard with a pixel-exact preview, and install everything on a Kobo.

pub mod generate;
pub mod kobo;
pub mod lovelace;
pub mod timezone;
pub mod versions;

use crate::config::AppConfig;
use crate::display::canvas::{has_fallback_font, load_fallback_font};
use crate::display::Canvas;
use crate::ha::HaClient;
use crate::i18n;
use crate::paths;
use crate::ui::screen::{grid_layout, grid_pages, hidden_card_count, GridLayout};
use crate::ui::ScreenRenderer;
use axum::extract::{Query, State};
use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use generate::EntityInfo;
use serde::Deserialize;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

const EDITOR_HTML: &str = include_str!("editor.html");
/// Drag-and-drop grid for the layout editor (gridstack.js, MIT, see
/// vendor/gridstack/LICENSE). Served from the binary: the editor works offline.
const GRIDSTACK_JS: &str = include_str!("vendor/gridstack/gridstack-all.js");
const GRIDSTACK_CSS: &str = include_str!("vendor/gridstack/gridstack.min.css");

/// Icons offered in the editor's icon picker
pub const ICONS: &[&str] = &[
    "lightbulb", "lightbulb_on", "power", "plug", "fan", "thermometer", "flame", "snowflake", "droplet", "wind", "zap",
    "sun", "moon", "cloud", "home", "sofa", "bed", "door", "lock", "tv", "music", "coffee", "washer", "printer",
    "battery", "clock", "layers", "progress", "play", "refresh", "check", "picture", "lamp", "radiator", "leaf", "plant", "window",
    "blinds", "garage", "car", "ev_charger", "solar", "gauge", "chart", "camera", "shield", "bell", "person", "motion",
    "key", "calendar", "mail", "trash", "faucet", "shower", "oven", "fridge", "vacuum", "laptop", "router", "gamepad",
    "heart", "star",
];

struct Inner {
    config_path: PathBuf,
    /// What the editor shows; only written to disk on Save.
    draft: RwLock<AppConfig>,
    /// YAML of the last saved state, to detect unsaved changes and revert.
    saved: RwLock<String>,
    ha: RwLock<HaClient>,
    entities: RwLock<Vec<EntityInfo>>,
    menu_name: RwLock<String>,
    kobo_binary: Option<PathBuf>,
    /// Rendering touches global state (language, fallback font); one at a time.
    render_lock: Mutex<()>,
    loaded_font: Mutex<Option<String>>,
    /// Fired by the "Quit" button to stop the server.
    shutdown: tokio::sync::Notify,
}

type Shared = Arc<Inner>;

pub async fn start_setup_server(
    config_path: PathBuf,
    port: u16,
    open_browser: bool,
    kobo_binary: Option<PathBuf>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let config = if config_path.exists() {
        AppConfig::load_or_default(&config_path)
    } else {
        // Fresh start: no demo pages, the user connects HA first.
        AppConfig { pages: Vec::new(), ..AppConfig::default() }
    };
    let ha = setup_client(&config);
    spawn_refresh(ha.clone());
    // The file keeps its old layout until the user saves.
    let saved = serde_yaml::to_string(&config).unwrap_or_default();
    let mut config = config;
    with_grid(&mut config);

    let state: Shared = Arc::new(Inner {
        config_path,
        saved: RwLock::new(saved),
        draft: RwLock::new(config),
        ha: RwLock::new(ha),
        entities: RwLock::new(Vec::new()),
        menu_name: RwLock::new(kobo::DEFAULT_MENU_NAME.to_string()),
        kobo_binary: kobo::find_kobo_binary(kobo_binary.as_deref()),
        render_lock: Mutex::new(()),
        loaded_font: Mutex::new(None),
        shutdown: tokio::sync::Notify::new(),
    });

    let app = Router::new()
        .route("/", get(|| async { ([(axum::http::header::CACHE_CONTROL, "no-store")], Html(EDITOR_HTML)) }))
        .route("/vendor/gridstack-all.js", get(|| async { ([(axum::http::header::CONTENT_TYPE, "text/javascript; charset=utf-8")], GRIDSTACK_JS) }))
        .route("/vendor/gridstack.min.css", get(|| async { ([(axum::http::header::CONTENT_TYPE, "text/css; charset=utf-8")], GRIDSTACK_CSS) }))
        .route("/api/state", get(get_state))
        .route("/api/config", post(put_config))
        .route("/api/save", post(save_config))
        .route("/api/revert", post(revert_config))
        .route("/api/versions", get(list_versions))
        .route("/api/versions/restore", post(restore_version))
        .route("/api/ha/test", post(test_ha))
        .route("/api/ha/entities", get(get_entities))
        .route("/api/generate", post(generate_pages))
        .route("/api/lovelace/dashboards", get(lovelace_dashboards))
        .route("/api/lovelace/import", post(import_lovelace))
        .route("/api/preview.png", get(preview))
        .route("/api/icon.png", get(icon_png))
        .route("/api/kobo", get(list_kobos))
        .route("/api/kobo/install", post(install_kobo))
        .route("/api/kobo/import", post(import_kobo))
        .route("/api/kobo/nickelmenu", post(install_nickelmenu))
        .route("/api/fonts/ensure", post(ensure_font))
        .route("/api/setup-strings", get(setup_strings))
        .route("/api/shutdown", post(shutdown))
        .with_state(state.clone());

    // Local only: the page handles a Home Assistant token.
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let url = format!("http://localhost:{}", port);
    println!();
    println!("  Kobo HA Dashboard setup is running at {}", url);
    println!("  Config file: {}", state.config_path.display());
    match &state.kobo_binary {
        Some(p) => println!("  Kobo program: {}", p.display()),
        None => println!("  Warning: the Kobo (ARM) program was not found next to this tool; installing will not work."),
    }
    println!("  Press Ctrl+C to stop.");
    println!();
    if open_browser {
        open_url(&url);
    }
    let stop = state.clone();
    axum::serve(listener, app)
        .with_graceful_shutdown(async move { stop.shutdown.notified().await })
        .await?;
    println!("  Setup closed. Your configuration is saved in {}", state.config_path.display());
    Ok(())
}

fn open_url(url: &str) {
    let result = if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(url).spawn()
    } else if cfg!(windows) {
        std::process::Command::new("cmd").args(["/C", "start", "", url]).spawn()
    } else {
        std::process::Command::new("xdg-open").arg(url).spawn()
    };
    if let Err(e) = result {
        tracing::warn!("Could not open a browser ({}); open {} manually", e, url);
    }
}

fn spawn_refresh(ha: HaClient) {
    tokio::spawn(async move {
        let _ = ha.refresh_states().await;
    });
}

fn bad_request(msg: impl Into<String>) -> Response {
    (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": msg.into() }))).into_response()
}

/// Error the editor can translate: `key` under `errors.`
fn bad_request_key(key: &str, english: &str, args: serde_json::Value) -> Response {
    (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": english, "key": key, "args": args }))).into_response()
}

/// Pages from before the grid get one, so the editor only sees grid pages.
/// Returns whether anything changed.
fn with_grid(config: &mut AppConfig) -> bool {
    let (w, h) = config.device.get_dimensions();
    let tabs = config.pages.len() > 1;
    grid_pages(&mut config.pages, w, h, tabs)
}

/// For the layout editor: how each grid page is laid out on the screen.
fn layouts(config: &AppConfig) -> Vec<Option<GridLayout>> {
    let (w, h) = config.device.get_dimensions();
    let tabs = config.pages.len() > 1;
    config.pages.iter().map(|p| grid_layout(p, w, h, tabs)).collect()
}

fn hidden_per_page(config: &AppConfig) -> Vec<usize> {
    let (w, h) = config.device.get_dimensions();
    let tabs = config.pages.len() > 1;
    config.pages.iter().map(|p| hidden_card_count(p, w, h, tabs)).collect()
}

async fn get_state(State(s): State<Shared>) -> Json<serde_json::Value> {
    let config = s.draft.read().await.clone();
    let devices = kobo::detect();
    // Prefer the name already on a connected Kobo.
    if let Some(name) = devices.iter().find_map(|d| kobo::read_menu_name(Path::new(&d.path))) {
        *s.menu_name.write().await = name;
    }
    Json(serde_json::json!({
        "config": config,
        "config_path": s.config_path,
        "hidden": hidden_per_page(&config),
        "layouts": layouts(&config),
        "languages": i18n::available(),
        "icons": ICONS,
        "models": crate::models::MODELS,
        "devices": devices,
        "kobo_binary": s.kobo_binary.is_some(),
        "menu_name": *s.menu_name.read().await,
        "connected": s.ha.read().await.is_connected().await,
        "dirty": is_dirty(&s).await,
    }))
}

/// Replace the draft. Nothing is written to disk until Save; the HA client
/// is rebuilt when the connection settings change.
/// A page sent without a grid gets one; the reply then carries the config, for
/// the editor to take over.
async fn put_config(State(s): State<Shared>, Json(mut config): Json<AppConfig>) -> Response {
    let changed = with_grid(&mut config);
    set_draft(&s, config.clone()).await;
    let mut status = draft_status(&s).await;
    if changed {
        status["config"] = serde_json::to_value(config).unwrap_or_default();
    }
    Json(status).into_response()
}

/// The Home Assistant client of the setup tool: it fetches what the pages
/// need, like the Kobo's, but keeps the state of every entity, so a card
/// added with an entity no page had yet shows its state at once.
fn setup_client(config: &AppConfig) -> HaClient {
    let mut ha = HaClient::for_app(config);
    ha.entity_ids = std::sync::Arc::new(Vec::new());
    ha
}

async fn set_draft(s: &Shared, config: AppConfig) {
    let old = s.draft.read().await.clone();
    let ha_changed = old.home_assistant.url != config.home_assistant.url
        || old.home_assistant.token != config.home_assistant.token
        || entity_set(&old) != entity_set(&config);
    if ha_changed {
        let ha = setup_client(&config);
        spawn_refresh(ha.clone());
        *s.ha.write().await = ha;
    }
    *s.draft.write().await = config;
}

async fn is_dirty(s: &Shared) -> bool {
    let draft = serde_yaml::to_string(&*s.draft.read().await).unwrap_or_default();
    draft != *s.saved.read().await
}

async fn draft_status(s: &Shared) -> serde_json::Value {
    let draft = s.draft.read().await.clone();
    serde_json::json!({ "hidden": hidden_per_page(&draft), "layouts": layouts(&draft), "dirty": is_dirty(s).await })
}

/// Write the draft to disk, keeping the previous file in `backups/`.
async fn write_draft(s: &Shared) -> Result<(), String> {
    let yaml = serde_yaml::to_string(&*s.draft.read().await).map_err(|e| e.to_string())?;
    versions::save(&s.config_path, &yaml).map_err(|e| format!("{}: {}", s.config_path.display(), e))?;
    *s.saved.write().await = yaml;
    Ok(())
}

async fn save_config(State(s): State<Shared>) -> Response {
    match write_draft(&s).await {
        Ok(()) => Json(draft_status(&s).await).into_response(),
        Err(e) => bad_request_key("save_failed", &e, serde_json::json!({ "error": e })),
    }
}

/// Throw away unsaved changes.
async fn revert_config(State(s): State<Shared>) -> Response {
    let saved = s.saved.read().await.clone();
    match serde_yaml::from_str::<AppConfig>(&saved) {
        Ok(mut config) => {
            with_grid(&mut config);
            set_draft(&s, config.clone()).await;
            let mut status = draft_status(&s).await;
            status["config"] = serde_json::to_value(config).unwrap_or_default();
            Json(status).into_response()
        }
        Err(e) => bad_request(e.to_string()),
    }
}

async fn list_versions(State(s): State<Shared>) -> Json<Vec<versions::Version>> {
    Json(versions::list(&s.config_path))
}

#[derive(Deserialize)]
struct RestoreRequest {
    id: String,
}

/// Load an earlier version into the draft (saved only when the user saves).
async fn restore_version(State(s): State<Shared>, Json(req): Json<RestoreRequest>) -> Response {
    match versions::load(&s.config_path, &req.id) {
        Ok(mut config) => {
            with_grid(&mut config);
            set_draft(&s, config.clone()).await;
            let mut status = draft_status(&s).await;
            status["config"] = serde_json::to_value(config).unwrap_or_default();
            Json(status).into_response()
        }
        Err(e) => bad_request(e),
    }
}

/// Weather/graph entities need extra API calls, so a new HA client is built
/// when they change.
fn entity_set(c: &AppConfig) -> Vec<String> {
    use crate::config::CardConfig;
    c.pages
        .iter()
        .flat_map(|p| p.cards.iter())
        .filter_map(|card| match &card.kind {
            CardConfig::Weather { entity_id, .. } => Some(entity_id.clone()),
            CardConfig::Sensor { entity_id, show_graph: true, .. } => Some(entity_id.clone()),
            // A chart's period and range decide what is fetched, so they are
            // part of the signature that rebuilds the Home Assistant client.
            CardConfig::Statistics { entity_id, period, days, hours, stat, .. } => {
                Some(format!("{}#{}:{}:{:?}:{}", entity_id, period, days, hours, stat))
            }
            CardConfig::Image { entity_id, .. } => Some(entity_id.clone()),
            _ => None,
        })
        .collect()
}

#[derive(Deserialize)]
struct TestRequest {
    url: String,
    token: String,
    /// Set when battery reporting is on: check that the token may write it.
    #[serde(default)]
    battery_entity: Option<String>,
}

async fn test_ha(Json(req): Json<TestRequest>) -> Json<generate::ConnectionReport> {
    let mut report = generate::test_connection(&req.url, &req.token).await;
    if report.ok {
        if let Some(entity) = &req.battery_entity {
            if let Some(warning) = generate::prepare_battery_sensor(&req.url, &req.token, entity).await {
                report.warnings.push(warning);
            }
        }
    }
    Json(report)
}

#[derive(Deserialize)]
struct LovelaceImportRequest {
    /// Lovelace dashboard `url_path`; empty or absent for the default one.
    #[serde(default)]
    url_path: Option<String>,
}

/// Dashboards of the connected Home Assistant, to pick from in the editor.
async fn lovelace_dashboards(State(s): State<Shared>) -> Response {
    let (url, token) = {
        let d = s.draft.read().await;
        (d.home_assistant.url.clone(), d.home_assistant.token.clone())
    };
    if token.trim().is_empty() {
        return bad_request_key("lovelace_no_token", "Connect Home Assistant first", serde_json::json!({}));
    }
    match lovelace::dashboards(&url, &token).await {
        Ok(list) => {
            let body: Vec<serde_json::Value> = list
                .into_iter()
                .map(|(url_path, title)| serde_json::json!({ "url_path": url_path, "title": title }))
                .collect();
            Json(body).into_response()
        }
        Err(e) => bad_request(e),
    }
}

/// Read one Lovelace dashboard and translate it to pages. The editor decides
/// what to do with them (merge into the draft); nothing is saved here.
async fn import_lovelace(State(s): State<Shared>, Json(req): Json<LovelaceImportRequest>) -> Response {
    let config = s.draft.read().await.clone();
    let (url, token) = (config.home_assistant.url.clone(), config.home_assistant.token.clone());
    if token.trim().is_empty() {
        return bad_request_key("lovelace_no_token", "Connect Home Assistant first", serde_json::json!({}));
    }
    let url_path = req.url_path.as_deref().filter(|p| !p.is_empty());
    tracing::info!("Lovelace import: reading dashboard {:?}", url_path);
    let raw = match lovelace::fetch_config(&url, &token, url_path).await {
        Ok(raw) => raw,
        Err(e) => {
            tracing::warn!("Lovelace import failed: {}", e);
            return bad_request(e);
        }
    };
    // Mark where these pages came from, so a later import of the same dashboard
    // replaces its own pages and leaves the rest alone.
    let source = format!("lovelace:{}", url_path.unwrap_or("default"));
    let classes = lovelace::state_classes(&url, &token).await;
    match lovelace::import_with(&raw, config.device.get_dimensions(), !config.pages.is_empty(), &classes) {
        Ok(mut result) => {
            for page in &mut result.pages {
                page.imported_from = Some(source.clone());
            }
            let (w, h) = config.device.get_dimensions();
            grid_pages(&mut result.pages, w, h, true);
            let cards: usize = result.pages.iter().map(|p| p.cards.len()).sum();
            tracing::info!(
                "Lovelace import: {} pages, {} cards, {} skipped kinds",
                result.pages.len(),
                cards,
                result.skipped.len()
            );
            Json(serde_json::json!({ "pages": result.pages, "skipped": result.skipped, "source": source }))
                .into_response()
        }
        Err(e) => {
            tracing::warn!("Lovelace import refused: {:?}", e);
            match e {
                lovelace::ImportError::Strategy => bad_request_key(
                    "lovelace_strategy",
                    "This dashboard is generated by a strategy, so its cards cannot be read",
                    serde_json::json!({}),
                ),
                lovelace::ImportError::Empty => bad_request_key(
                    "lovelace_empty",
                    "This dashboard has no cards the Kobo can show",
                    serde_json::json!({}),
                ),
            }
        }
    }
}

async fn load_entities(s: &Shared, force: bool) -> Result<Vec<EntityInfo>, String> {
    if !force {
        let cached = s.entities.read().await;
        if !cached.is_empty() {
            return Ok(cached.clone());
        }
    }
    let (url, token) = {
        let d = s.draft.read().await;
        (d.home_assistant.url.clone(), d.home_assistant.token.clone())
    };
    let list = generate::fetch_entities(&url, &token).await.map_err(|e| e.to_string())?;
    *s.entities.write().await = list.clone();
    Ok(list)
}

#[derive(Deserialize)]
struct EntitiesQuery {
    #[serde(default)]
    refresh: bool,
}

async fn get_entities(State(s): State<Shared>, Query(q): Query<EntitiesQuery>) -> Response {
    match load_entities(&s, q.refresh).await {
        Ok(list) => Json(list).into_response(),
        Err(e) => bad_request(e),
    }
}

async fn generate_pages(State(s): State<Shared>) -> Response {
    let entities = match load_entities(&s, true).await {
        Ok(list) => list,
        Err(e) => return bad_request(e),
    };
    let (lang, dims) = {
        let d = s.draft.read().await;
        (d.theme.language.clone(), d.device.get_dimensions())
    };
    let _guard = s.render_lock.lock().await;
    i18n::set_language(&lang);
    let mut pages = generate::generate(&entities, dims);
    grid_pages(&mut pages, dims.0, dims.1, true);
    Json(pages).into_response()
}

/// Load the fallback font a language needs (if downloaded) before rendering.
async fn prepare_font(s: &Shared, lang: &str) {
    let family = i18n::font_for(lang);
    let mut loaded = s.loaded_font.lock().await;
    if *loaded != family || (family.is_some() && !has_fallback_font()) {
        let dir = s.config_path.parent().map(Path::to_path_buf).unwrap_or_default();
        load_fallback_font(&paths::font_dirs(&dir), family.as_deref());
        *loaded = family;
    }
}

#[derive(Deserialize)]
struct PreviewQuery {
    #[serde(default)]
    page: usize,
    #[serde(default)]
    bw: bool,
    /// Draw the sleep-mode note, as the Kobo shows it while asleep
    #[serde(default)]
    sleep: bool,
}

async fn preview(State(s): State<Shared>, Query(q): Query<PreviewQuery>) -> Response {
    let config = s.draft.read().await.clone();
    let ha = s.ha.read().await.clone();
    // The setup tool has no live updates: states older than half a minute
    // are fetched again, for the next look
    let stale = ha.last_update().await.is_none_or(|t| chrono::Utc::now() - t > chrono::Duration::seconds(30));
    if stale {
        spawn_refresh(ha.clone());
    }
    let states = ha.get_all_states().await;
    let live = ha.is_connected().await;
    let (w, h) = config.device.get_dimensions();

    let _guard = s.render_lock.lock().await;
    i18n::set_language(&config.theme.language);
    prepare_font(&s, &config.theme.language).await;
    let mut canvas = Canvas::for_screen(w, h);
    let mut renderer = ScreenRenderer::new(&config);
    renderer.last_update = ha.last_update().await;
    renderer.set_images(ha.images().await);
    // Most Kobos have a frontlight; show its button as on the device.
    renderer.frontlight = Some(true);
    if q.sleep {
        let next = crate::ui::widgets::header::now_local(&config.theme) + chrono::Duration::seconds(config.device.sleep_wake_secs.max(120) as i64);
        renderer.sleep_until = Some(crate::ui::widgets::header::format_time(&next, &config.theme));
    }
    renderer.render(&mut canvas, &config, q.page.min(config.pages.len().saturating_sub(1)), &states, live);
    let hidden = renderer.hidden_cards;
    let png = if q.bw { canvas.to_grayscale_png_bytes() } else { canvas.to_png_bytes() };
    match png {
        Ok(bytes) => (
            [
                (header::CONTENT_TYPE, "image/png".to_string()),
                (header::CACHE_CONTROL, "no-store".to_string()),
                (header::HeaderName::from_static("x-hidden-cards"), hidden.to_string()),
            ],
            bytes,
        )
            .into_response(),
        Err(e) => bad_request(e.to_string()),
    }
}

#[derive(Deserialize)]
struct IconQuery {
    name: String,
}

async fn icon_png(Query(q): Query<IconQuery>) -> Response {
    let mut canvas = Canvas::new(48, 48);
    canvas.clear(crate::display::Color::rgba(0, 0, 0, 0));
    canvas.draw_icon(&q.name, 6.0, 6.0, 36.0, crate::display::Color::rgb(20, 24, 32), 2.2);
    match canvas.to_png_bytes() {
        Ok(bytes) => ([(header::CONTENT_TYPE, "image/png"), (header::CACHE_CONTROL, "max-age=3600")], bytes).into_response(),
        Err(e) => bad_request(e.to_string()),
    }
}

async fn list_kobos(State(s): State<Shared>) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "devices": kobo::detect(), "kobo_binary": s.kobo_binary.is_some() }))
}

#[derive(Deserialize)]
struct InstallRequest {
    path: String,
    menu_name: String,
}

async fn install_kobo(State(s): State<Shared>, Json(req): Json<InstallRequest>) -> Response {
    let Some(binary) = s.kobo_binary.clone() else {
        return bad_request_key("binary_missing", "The Kobo program was not found next to the setup tool.", serde_json::json!({}));
    };
    // What goes on the Kobo is also what is saved on this computer.
    if let Err(e) = write_draft(&s).await {
        return bad_request_key("save_failed", &e, serde_json::json!({ "error": e }));
    }
    let config = s.draft.read().await.clone();
    let mut log = vec![kobo::Msg::new("saved")];

    let mut fonts = Vec::new();
    if let Some(family) = i18n::font_for(&config.theme.language) {
        log.push(kobo::Msg::new("font_download").arg("family", family.as_str()));
        match paths::ensure_font(&family).await {
            Ok(files) => fonts = files,
            Err(e) => return bad_request_key("font_failed", "Font download failed", serde_json::json!({ "error": e.to_string() })),
        }
    }

    *s.menu_name.write().await = req.menu_name.clone();
    match kobo::install(Path::new(&req.path), &config, &req.menu_name, &binary, &fonts) {
        Ok(mut steps) => {
            log.append(&mut steps);
            Json(serde_json::json!({ "log": log })).into_response()
        }
        Err(e) => bad_request_key("install_failed", &e, serde_json::json!({ "error": e })),
    }
}

#[derive(Deserialize)]
struct NickelMenuRequest {
    path: String,
}

async fn install_nickelmenu(Json(req): Json<NickelMenuRequest>) -> Response {
    let root = Path::new(&req.path);
    // .kobo/version: serial,kernel,firmware,...
    let firmware = std::fs::read_to_string(root.join(".kobo/version"))
        .ok()
        .and_then(|v| v.trim().split(',').nth(2).map(|f| f.trim().to_string()))
        .filter(|f| !f.is_empty());
    let Some(package) = kobo::nickelmenu_for(firmware.as_deref()) else {
        let version = firmware.unwrap_or_default();
        return bad_request_key("firmware_unsupported", "No NickelMenu for this Kobo software", serde_json::json!({ "version": version }));
    };
    let mut log = vec![kobo::Msg::new("nm_download").arg("version", package.version)];
    let bytes = match paths::nickelmenu_package(package).await {
        Ok(p) => p,
        Err(e) => return bad_request_key("nm_failed", "Could not install NickelMenu", serde_json::json!({ "error": e.to_string() })),
    };
    match kobo::install_nickelmenu(root, &bytes, package.file_name) {
        Ok(()) => {
            log.push(kobo::Msg::new("nm_copied").arg("version", package.version));
            Json(serde_json::json!({ "log": log })).into_response()
        }
        Err(e) if e == "other_package_pending" => {
            bad_request_key("nm_other_pending", "Another update package is already waiting on this Kobo", serde_json::json!({}))
        }
        Err(e) => bad_request_key("nm_failed", "Could not install NickelMenu", serde_json::json!({ "error": e })),
    }
}

#[derive(Deserialize)]
struct ImportRequest {
    path: String,
}

async fn import_kobo(Json(req): Json<ImportRequest>) -> Response {
    let path = Path::new(&req.path).join(".kobo-ha/config.yaml");
    match AppConfig::load_from_file(&path) {
        Ok(mut config) => {
            with_grid(&mut config);
            Json(config).into_response()
        }
        Err(e) => bad_request_key(
            "import_failed",
            "Could not read the config",
            serde_json::json!({ "path": path.display().to_string(), "error": e.to_string() }),
        ),
    }
}

#[derive(Deserialize)]
struct FontQuery {
    lang: String,
}

async fn ensure_font(State(s): State<Shared>, Query(q): Query<FontQuery>) -> Response {
    let Some(family) = i18n::font_for(&q.lang) else {
        return Json(serde_json::json!({ "needed": false })).into_response();
    };
    match paths::ensure_font(&family).await {
        Ok(_) => {
            *s.loaded_font.lock().await = None;
            Json(serde_json::json!({ "needed": true, "ready": true, "family": family })).into_response()
        }
        Err(e) => bad_request_key("font_failed", "Font download failed", serde_json::json!({ "error": e.to_string() })),
    }
}

#[derive(Deserialize)]
struct StringsQuery {
    lang: String,
}

/// Editor translations for one language, English filling any gaps.
async fn setup_strings(Query(q): Query<StringsQuery>) -> Json<std::collections::HashMap<String, String>> {
    Json(i18n::setup_strings(&q.lang))
}

/// Stop the setup tool. The reply goes out first, then the server shuts down.
async fn shutdown(State(s): State<Shared>) -> Json<serde_json::Value> {
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        s.shutdown.notify_one();
        // A browser tab holding a connection open must not keep us alive.
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        std::process::exit(0);
    });
    Json(serde_json::json!({ "ok": true }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every translation key the editor uses must exist in lang/setup/en.yaml
    /// (the other languages are checked against en.yaml in i18n.rs).
    #[test]
    fn editor_keys_exist_in_english_strings() {
        let strings = i18n::setup_strings("en");
        let mut keys = Vec::new();
        for attr in ["data-i18n=\"", "data-i18n-html=\"", "data-i18n-title=\"", "data-i18n-aria=\""] {
            for part in EDITOR_HTML.split(attr).skip(1) {
                keys.push(part.split('"').next().unwrap_or_default().to_string());
            }
        }
        for part in EDITOR_HTML.split("T(\"").skip(1) {
            let key = part.split('"').next().unwrap_or_default();
            // `T("install.log." + key)` style prefixes are built at runtime
            if !key.ends_with('.') {
                keys.push(key.to_string());
            }
        }
        assert!(keys.len() > 100, "found only {} keys; did the markup change?", keys.len());
        let missing: Vec<&String> = keys.iter().filter(|k| !strings.contains_key(*k)).collect();
        assert!(missing.is_empty(), "editor uses keys missing from lang/setup/en.yaml: {:?}", missing);
    }

    #[test]
    fn every_card_type_has_a_label() {
        let strings = i18n::setup_strings("en");
        for t in [
            "weather", "light", "switch", "climate", "sensor", "sensor_grid", "media_player", "scenes_row", "scene", "heading",
            "statistics", "image",
        ] {
            assert!(strings.contains_key(&format!("card_types.{}", t)), "card_types.{} missing", t);
        }
    }

    #[test]
    fn the_editor_only_sees_grid_pages() {
        // An older config: no page has a grid
        let mut config = AppConfig::default();
        let before = serde_yaml::to_string(&config).unwrap();
        assert!(!before.contains("grid:"));
        assert!(with_grid(&mut config));
        for page in &config.pages {
            assert!(page.grid.is_some(), "{} has no grid", page.id);
            assert!(page.cards.iter().all(|c| c.pos.is_some()), "{}: a card has no place", page.id);
        }
        // What is shown does not change, nothing more is hidden
        assert_eq!(hidden_per_page(&config), vec![0; config.pages.len()]);
        // A second pass leaves grid pages alone
        let once = serde_yaml::to_string(&config).unwrap();
        assert!(!with_grid(&mut config));
        assert_eq!(serde_yaml::to_string(&config).unwrap(), once);
    }
}
