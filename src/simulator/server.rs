use crate::config::AppConfig;
use crate::display::Canvas;
use crate::ha::HaClient;
use crate::ui::layout::UiAction;
use crate::ui::ScreenRenderer;
use axum::extract::State;
use axum::http::header::CONTENT_TYPE;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct SimulatorState {
    pub config: Arc<RwLock<AppConfig>>,
    pub ha_client: HaClient,
    pub renderer: Arc<RwLock<ScreenRenderer>>,
    pub active_page: Arc<RwLock<usize>>,
    pub is_landscape: Arc<RwLock<bool>>,
    pub bw_mode: Arc<RwLock<bool>>,
}

#[derive(Debug, Deserialize)]
pub struct TouchRequest {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Serialize)]
pub struct StatusResponse {
    pub is_connected: bool,
    pub orientation: String,
    pub active_page: usize,
    pub bw_mode: bool,
    pub width: u32,
    pub height: u32,
}

pub async fn start_simulator_server(
    config: AppConfig,
    ha_client: HaClient,
    port: u16,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut renderer = ScreenRenderer::new(&config);
    // Show the light button like on a real Kobo.
    renderer.frontlight = Some(true);
    let is_landscape = config.device.is_landscape();

    let state = SimulatorState {
        config: Arc::new(RwLock::new(config)),
        ha_client,
        renderer: Arc::new(RwLock::new(renderer)),
        active_page: Arc::new(RwLock::new(0)),
        is_landscape: Arc::new(RwLock::new(is_landscape)),
        bw_mode: Arc::new(RwLock::new(false)),
    };

    let app = Router::new()
        .route("/", get(handle_index))
        .route("/screen.png", get(handle_screen_png))
        .route("/api/touch", post(handle_touch))
        .route("/api/toggle_orientation", post(handle_toggle_orientation))
        .route("/api/toggle_bw", post(handle_toggle_bw))
        .route("/api/refresh", post(handle_refresh))
        .route("/api/status", get(handle_status))
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!("Kobo Clara Colour Simulator running at http://localhost:{}", port);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn render_canvas_internal(state: &SimulatorState) -> Canvas {
    let is_landscape = *state.is_landscape.read().await;
    let mut config = state.config.read().await.clone();
    config.device.orientation = if is_landscape { "landscape".to_string() } else { "portrait".to_string() };
    let (w, h) = config.device.get_dimensions();
    let mut canvas = Canvas::new(w, h);

    let active_page = *state.active_page.read().await;
    let states = state.ha_client.get_all_states().await;
    let is_live = state.ha_client.is_connected().await;

    let mut renderer = state.renderer.write().await;
    renderer.last_update = state.ha_client.last_update().await;
    renderer.render(&mut canvas, &config, active_page, &states, is_live);

    canvas
}

async fn handle_screen_png(State(state): State<SimulatorState>) -> Response {
    let canvas = render_canvas_internal(&state).await;
    let is_bw = *state.bw_mode.read().await;

    let png_bytes = if is_bw {
        canvas.to_grayscale_png_bytes().unwrap_or_default()
    } else {
        canvas.to_png_bytes().unwrap_or_default()
    };

    ([(CONTENT_TYPE, "image/png")], png_bytes).into_response()
}

async fn handle_touch(
    State(state): State<SimulatorState>,
    Json(payload): Json<TouchRequest>,
) -> StatusCode {
    let action = {
        let renderer = state.renderer.read().await;
        renderer.handle_touch(payload.x, payload.y)
    };

    if let Some(action) = action {
        match action {
            UiAction::ToggleEntity(entity_id) => {
                state.ha_client.toggle_entity(&entity_id).await;
            }
            UiAction::SetClimateTemp(entity_id, delta) => {
                state.ha_client.set_climate_temp(&entity_id, delta).await;
            }
            UiAction::MediaPlayPause(entity_id) => {
                let p = serde_json::json!({ "entity_id": entity_id });
                let _ = state.ha_client.call_service("media_player", "media_play_pause", p).await;
            }
            UiAction::MediaNext(entity_id) => {
                let p = serde_json::json!({ "entity_id": entity_id });
                let _ = state.ha_client.call_service("media_player", "media_next_track", p).await;
            }
            UiAction::MediaPrevious(entity_id) => {
                let p = serde_json::json!({ "entity_id": entity_id });
                let _ = state.ha_client.call_service("media_player", "media_previous_track", p).await;
            }
            UiAction::ActivateScene(entity_id) => {
                let (domain, service, data) = state.ha_client.activate_request(&entity_id);
                let _ = state.ha_client.call_service(&domain, &service, data).await;
            }
            UiAction::SwitchPage(idx) => {
                let mut p = state.active_page.write().await;
                *p = idx;
            }
            UiAction::Refresh => {
                let _ = state.ha_client.refresh_states().await;
            }
            UiAction::ToggleFrontlight => {
                let mut r = state.renderer.write().await;
                r.frontlight = Some(!r.frontlight.unwrap_or(true));
            }
            UiAction::ExitToReader => {
                tracing::info!("Simulator received ExitToReader action");
            }
        }
    }

    StatusCode::OK
}

async fn handle_toggle_orientation(State(state): State<SimulatorState>) -> Json<StatusResponse> {
    {
        let mut orient = state.is_landscape.write().await;
        *orient = !*orient;
    }
    handle_status(State(state)).await
}

async fn handle_toggle_bw(State(state): State<SimulatorState>) -> Json<StatusResponse> {
    {
        let mut bw = state.bw_mode.write().await;
        *bw = !*bw;
    }
    handle_status(State(state)).await
}

async fn handle_refresh(State(state): State<SimulatorState>) -> StatusCode {
    let _ = state.ha_client.refresh_states().await;
    StatusCode::OK
}

async fn handle_status(State(state): State<SimulatorState>) -> Json<StatusResponse> {
    let is_landscape = *state.is_landscape.read().await;
    let mut device = state.config.read().await.device.clone();
    device.orientation = if is_landscape { "landscape".to_string() } else { "portrait".to_string() };
    let (width, height) = device.get_dimensions();

    Json(StatusResponse {
        is_connected: state.ha_client.is_connected().await,
        orientation: if is_landscape { "landscape".to_string() } else { "portrait".to_string() },
        active_page: *state.active_page.read().await,
        bw_mode: *state.bw_mode.read().await,
        width,
        height,
    })
}

async fn handle_index() -> Html<&'static str> {
    Html(INDEX_HTML)
}

static INDEX_HTML: &str = r#"<!DOCTYPE html>
<html lang="tr">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>Kobo Clara Colour - Home Assistant Dashboard Simulator</title>
  <style>
    :root {
      --bg: #1e1e24;
      --bezel: #2b2b36;
      --bezel-edge: #18181f;
      --accent: #d97706;
    }
    * { box-sizing: border-box; margin: 0; padding: 0; font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif; }
    body {
      background: var(--bg);
      color: #e2e8f0;
      min-height: 100vh;
      display: flex;
      flex-direction: column;
      align-items: center;
      padding: 24px;
    }
    header {
      width: 100%;
      max-width: 1200px;
      display: flex;
      justify-content: space-between;
      align-items: center;
      margin-bottom: 20px;
      padding-bottom: 16px;
      border-bottom: 1px solid #334155;
    }
    .title-group h1 { font-size: 20px; font-weight: 700; color: #fff; }
    .title-group p { font-size: 13px; color: #94a3b8; }
    .controls { display: flex; gap: 10px; align-items: center; }
    button {
      background: #334155;
      color: #fff;
      border: 1px solid #475569;
      padding: 8px 16px;
      border-radius: 8px;
      font-size: 13px;
      font-weight: 600;
      cursor: pointer;
      transition: all 0.2s;
    }
    button:hover { background: #475569; }
    button.active { background: var(--accent); border-color: var(--accent); }
    .status-badge {
      font-size: 12px;
      padding: 4px 10px;
      border-radius: 12px;
      font-weight: 600;
    }
    .status-live { background: #065f46; color: #34d399; }
    .status-demo { background: #78350f; color: #fbbf24; }
    
    /* Device Bezel Simulator */
    .kobo-wrapper {
      position: relative;
      background: var(--bezel);
      border: 14px solid var(--bezel-edge);
      border-radius: 36px;
      box-shadow: 0 25px 60px rgba(0,0,0,0.6), inset 0 2px 6px rgba(255,255,255,0.08);
      padding: 24px 20px 42px 20px;
      transition: all 0.3s ease;
      cursor: crosshair;
    }
    .kobo-logo {
      text-align: center;
      margin-top: 16px;
      font-size: 15px;
      font-weight: 800;
      letter-spacing: 3px;
      color: #64748b;
      user-select: none;
    }
    .screen-container {
      position: relative;
      overflow: hidden;
      border-radius: 6px;
      box-shadow: inset 0 0 10px rgba(0,0,0,0.4);
      background: #f8f9fa;
    }
    #screen-img {
      display: block;
      width: 100%;
      height: 100%;
      user-select: none;
      -webkit-user-drag: none;
    }
    /* E-ink tactile ripple on click */
    .ripple {
      position: absolute;
      width: 32px;
      height: 32px;
      background: rgba(0,0,0,0.25);
      border-radius: 50%;
      transform: translate(-50%, -50%);
      pointer-events: none;
      animation: ripple-anim 0.35s ease-out forwards;
    }
    @keyframes ripple-anim {
      0% { transform: translate(-50%, -50%) scale(0.2); opacity: 0.8; }
      100% { transform: translate(-50%, -50%) scale(2.2); opacity: 0; }
    }
    .info-footer {
      margin-top: 24px;
      font-size: 13px;
      color: #64748b;
      text-align: center;
    }
  </style>
</head>
<body>
  <header>
    <div class="title-group">
      <h1>Kobo Clara Colour • E-Ink Home Assistant Dashboard</h1>
      <p>Interactive simulator</p>
    </div>
    <div class="controls">
      <span id="badge" class="status-badge status-demo">Loading…</span>
      <button id="btn-orient" onclick="toggleOrientation()">Rotate (portrait/landscape)</button>
      <button id="btn-bw" onclick="toggleBw()">Colour / grayscale</button>
      <button onclick="forceRefresh()">Refresh</button>
    </div>
  </header>

  <div class="kobo-wrapper" id="kobo-frame">
    <div class="screen-container" id="screen-box">
      <img id="screen-img" src="/screen.png" alt="Kobo Dashboard" onclick="handleScreenClick(event)">
    </div>
    <div class="kobo-logo">kobo</div>
  </div>

  <div class="info-footer">
    Tip: click cards and buttons on the screen to toggle lights, change the thermostat or switch pages.
  </div>

  <script>
    let currentWidth = 1072;
    let currentHeight = 1448;
    let isLandscape = false;
    let isBw = false;

    async function updateStatus() {
      try {
        const res = await fetch('/api/status');
        const data = await res.json();
        isLandscape = data.orientation === 'landscape';
        isBw = data.bw_mode;
        currentWidth = data.width;
        currentHeight = data.height;

        const badge = document.getElementById('badge');
        if (data.is_connected) {
          badge.textContent = 'Home Assistant connected';
          badge.className = 'status-badge status-live';
        } else {
          badge.textContent = 'Demo mode';
          badge.className = 'status-badge status-demo';
        }

        const btnBw = document.getElementById('btn-bw');
        btnBw.className = isBw ? 'active' : '';
        btnBw.textContent = isBw ? 'Mod: Carta S&B' : 'Mod: Kaleido 3 Renkli';

        // Adjust display scale for comfortable desktop viewing
        const scale = isLandscape ? 0.65 : 0.52;
        const screenBox = document.getElementById('screen-box');
        screenBox.style.width = (currentWidth * scale) + 'px';
        screenBox.style.height = (currentHeight * scale) + 'px';
      } catch (e) {
        console.error(e);
      }
    }

    function refreshImage() {
      const img = document.getElementById('screen-img');
      img.src = '/screen.png?t=' + Date.now();
    }

    async function handleScreenClick(event) {
      const img = document.getElementById('screen-img');
      const rect = img.getBoundingClientRect();
      const clickX = event.clientX - rect.left;
      const clickY = event.clientY - rect.top;

      // Create ripple animation at click location
      const ripple = document.createElement('div');
      ripple.className = 'ripple';
      ripple.style.left = clickX + 'px';
      ripple.style.top = clickY + 'px';
      img.parentElement.appendChild(ripple);
      setTimeout(() => ripple.remove(), 400);

      // Map to hardware coordinate space
      const normX = (clickX / rect.width) * currentWidth;
      const normY = (clickY / rect.height) * currentHeight;

      await fetch('/api/touch', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ x: normX, y: normY })
      });

      refreshImage();
    }

    async function toggleOrientation() {
      await fetch('/api/toggle_orientation', { method: 'POST' });
      await updateStatus();
      refreshImage();
    }

    async function toggleBw() {
      await fetch('/api/toggle_bw', { method: 'POST' });
      await updateStatus();
      refreshImage();
    }

    async function forceRefresh() {
      await fetch('/api/refresh', { method: 'POST' });
      refreshImage();
    }

    // Auto-poll for state changes every 3 seconds
    setInterval(refreshImage, 3000);
    updateStatus();
  </script>
</body>
</html>
"#;
