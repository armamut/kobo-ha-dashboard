#![allow(dead_code, unused_imports)]

mod config;
mod display;
mod frontlight;
mod i18n;
mod ha;
mod input;
mod models;
mod paths;
mod setup;
mod simulator;
mod ui;

use clap::{Parser, Subcommand};
use config::AppConfig;
use display::{create_framebuffer, Canvas};
use ha::HaClient;
use input::{InputEvent, TouchDriver, TouchMapping};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use ui::layout::UiAction;
use ui::ScreenRenderer;

#[derive(Parser, Debug)]
#[command(
    name = "kobo-ha-dashboard",
    version,
    about = "Home Assistant dashboard for Kobo e-readers. Run without arguments on a computer to open the setup tool."
)]
struct Args {
    #[command(subcommand)]
    command: Option<Command>,

    /// Path to YAML configuration file
    #[arg(short, long, global = true)]
    config: Option<PathBuf>,

    /// Run the interactive desktop simulator instead of the setup tool
    #[arg(short, long)]
    simulator: bool,

    /// Port for the simulator / setup web server
    #[arg(short, long, default_value_t = 3000, global = true)]
    port: u16,

    /// Force hardware device mode using Linux framebuffer
    #[arg(short, long)]
    device: bool,

    /// Override orientation ("portrait" or "landscape")
    #[arg(long)]
    orientation: Option<String>,

    /// Render a single frame and exit (useful for cron/batch mode)
    #[arg(long)]
    once: bool,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Open the setup tool: connect Home Assistant, design pages, install to a Kobo
    Setup {
        /// Don't open a browser window automatically
        #[arg(long)]
        no_browser: bool,

        /// Path to the Kobo (ARM) build to install
        #[arg(long)]
        kobo_binary: Option<PathBuf>,
    },
}

/// Ignore taps arriving faster than this (e-ink ghost/bounce protection)
const TAP_DEBOUNCE: Duration = Duration::from_millis(350);

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "kobo_ha_dashboard=info".into()),
        )
        .with(tracing_subscriber::fmt::layer().with_ansi(false))
        .init();

    let args = Args::parse();

    let is_kobo = cfg!(target_os = "linux") && Path::new("/mnt/onboard/.kobo").exists();

    // Setup tool: explicit subcommand, or plain launch on a computer.
    let wants_setup = matches!(args.command, Some(Command::Setup { .. }))
        || (args.command.is_none() && !args.simulator && !args.device && !args.once && !is_kobo);
    if wants_setup {
        let (no_browser, kobo_binary) = match args.command {
            Some(Command::Setup { no_browser, kobo_binary }) => (no_browser, kobo_binary),
            None => (false, None),
        };
        let config_path = args.config.clone().unwrap_or_else(|| {
            let local = PathBuf::from("config.yaml");
            if local.exists() { local } else { paths::data_dir().join("config.yaml") }
        });
        i18n::set_override_dir(config_path.parent().map(|d| d.join("lang")).as_deref());
        return setup::start_setup_server(config_path, args.port, !no_browser, kobo_binary).await;
    }

    // Check multiple candidate locations for config.yaml
    let requested = args.config.clone().unwrap_or_else(|| PathBuf::from("config.yaml"));
    let config_path = if requested.exists() {
        requested
    } else if Path::new("/mnt/onboard/.kobo-ha/config.yaml").exists() {
        PathBuf::from("/mnt/onboard/.kobo-ha/config.yaml")
    } else if Path::new("config.example.yaml").exists() {
        PathBuf::from("config.example.yaml")
    } else {
        requested
    };

    let mut config = AppConfig::load_or_default(&config_path);

    if let Some(orient) = args.orientation {
        config.device.orientation = orient;
    }

    // Language and the extra font some languages need
    let config_dir = config_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    i18n::set_override_dir(Some(&config_dir.join("lang")));
    i18n::set_language(&config.theme.language);
    display::canvas::load_fallback_font(&paths::font_dirs(&config_dir), i18n::font_for(&config.theme.language).as_deref());

    let ha_client = HaClient::for_app(&config);

    let is_linux_device = cfg!(target_os = "linux") && Path::new(&config.device.fb_device).exists();
    let run_as_device = !args.simulator && (args.device || is_linux_device);

    if !run_as_device {
        let ha_init = ha_client.clone();
        tokio::spawn(async move {
            let _ = ha_init.refresh_states().await;
        });

        tracing::info!("Starting in Simulator mode on port {}", args.port);
        println!();
        println!("  Kobo HA Dashboard simulator: http://localhost:{}", args.port);
        println!();
        simulator::start_simulator_server(config, ha_client, args.port).await?;
        return Ok(());
    }

    #[cfg(unix)]
    return run_device(config, ha_client, args.once).await;
    #[cfg(not(unix))]
    return Err("Device mode is only available on the Kobo".into());
}

#[cfg(unix)]
async fn run_device(
    config: AppConfig,
    ha_client: HaClient,
    once: bool,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing::info!("Starting on Kobo e-reader device");
    let probed = display::framebuffer::probe_resolution(&config.device.fb_device);
    let (width, height) = config.device.resolve_dimensions(probed);
    tracing::info!("Canvas {}x{} (framebuffer {:?}, model '{}')", width, height, probed, config.device.model);
    let canvas_rotation = config.device.canvas_rotation();
    let (portrait_w, portrait_h) = if canvas_rotation.swaps_axes() { (height, width) } else { (width, height) };

    let mut fb = create_framebuffer(
        &config.device.fb_device,
        width,
        height,
        canvas_rotation,
        config.device.fb_rotation(),
        config.device.color_panel,
    );
    let mut canvas = Canvas::new(width, height);
    let mut renderer = ScreenRenderer::new(&config);
    let mut active_page = 0;

    if once {
        let _ = ha_client.refresh_states().await;
        redraw(&mut renderer, &mut canvas, &config, active_page, &ha_client).await;
        fb.flush(&canvas, true)?;
        tracing::info!("Single frame rendered and flushed to framebuffer");
        // Leave the dashboard on screen instead of restoring Nickel's frame.
        std::mem::forget(fb);
        return Ok(());
    }

    let (mut touch_driver, _touch_tx) = TouchDriver::new(
        &config.device.touch_device,
        TouchMapping {
            portrait_w,
            portrait_h,
            swap_xy: config.device.swap_touch_xy,
            invert_x: config.device.invert_touch_x,
            invert_y: config.device.invert_touch_y,
            canvas_rotation,
        },
    );

    // Wakeups: periodic refresh, finished service calls, clock ticks.
    let (wake_tx, mut wake_rx) = tokio::sync::mpsc::channel::<()>(8);

    {
        let ha = ha_client.clone();
        let tx = wake_tx.clone();
        let interval = Duration::from_secs(config.home_assistant.refresh_interval_secs.max(5));
        tokio::spawn(async move {
            loop {
                let _ = ha.refresh_states().await;
                if tx.send(()).await.is_err() {
                    return;
                }
                tokio::time::sleep(interval).await;
            }
        });
    }

    let mut sigterm = signal(tokio::signal::unix::SignalKind::terminate());
    let mut sigint = signal(tokio::signal::unix::SignalKind::interrupt());
    let mut sighup = signal(tokio::signal::unix::SignalKind::hangup());

    let mut update_counter: u32 = 0;
    let mut last_tap = Instant::now() - TAP_DEBOUNCE;
    let mut flush = |fb: &mut Box<dyn display::FramebufferDriver>, canvas: &Canvas, force_full: bool| {
        update_counter = update_counter.wrapping_add(1);
        let cycles = config.device.full_refresh_cycles;
        let full = force_full || (cycles > 0 && update_counter % cycles == 0);
        if let Err(e) = fb.flush(canvas, full) {
            tracing::error!("Framebuffer flush failed: {}", e);
        }
    };

    // Frontlight: header button toggles it; optional auto-off after idle.
    let mut light = frontlight::Frontlight::detect();
    let light_timeout = Duration::from_secs(config.device.frontlight_timeout_secs as u64);
    // True when the timer (not the user) switched the light off; the next tap
    // then only wakes the light.
    let mut light_dimmed = false;
    let mut last_activity = Instant::now();
    renderer.frontlight = light.as_ref().map(|l| l.is_on());

    {
        redraw(&mut renderer, &mut canvas, &config, active_page, &ha_client).await;
        flush(&mut fb, &canvas, true);
    }

    let mut clock = tokio::time::interval(Duration::from_secs(60));
    clock.tick().await;

    loop {
        let light_deadline = match &light {
            Some(l) if l.is_on() && !light_timeout.is_zero() => Some(tokio::time::Instant::from_std(last_activity + light_timeout)),
            _ => None,
        };
        tokio::select! {
            _ = async {
                match light_deadline {
                    Some(at) => tokio::time::sleep_until(at).await,
                    None => std::future::pending::<()>().await,
                }
            } => {
                if let Some(l) = light.as_mut() {
                    tracing::info!("No taps for {:?}, switching the light off", light_timeout);
                    l.set_on(false);
                    light_dimmed = true;
                    renderer.frontlight = Some(false);
                    redraw(&mut renderer, &mut canvas, &config, active_page, &ha_client).await;
                    flush(&mut fb, &canvas, false);
                }
            }

            event = touch_driver.recv() => {
                let Some(event) = event else { continue };
                let (x, y) = match event {
                    InputEvent::PowerButton => {
                        tracing::info!("Power button pressed, exiting");
                        break;
                    }
                    InputEvent::Tap { x, y } => (x, y),
                };
                if last_tap.elapsed() < TAP_DEBOUNCE {
                    continue;
                }
                last_tap = Instant::now();
                last_activity = Instant::now();
                tracing::debug!("Tap at ({:.0}, {:.0})", x, y);

                // After an automatic switch-off, a tap only brings the light back.
                if light_dimmed {
                    light_dimmed = false;
                    if let Some(l) = light.as_mut() {
                        l.set_on(true);
                        renderer.frontlight = Some(true);
                        redraw(&mut renderer, &mut canvas, &config, active_page, &ha_client).await;
                        flush(&mut fb, &canvas, false);
                        continue;
                    }
                }

                let Some(action) = renderer.handle_touch(x, y) else { continue };
                let mut force_full = false;
                let service = match action {
                    UiAction::ExitToReader => {
                        tracing::info!("Exit button tapped");
                        break;
                    }
                    UiAction::SwitchPage(idx) => {
                        active_page = idx;
                        force_full = true;
                        None
                    }
                    UiAction::Refresh => {
                        force_full = true;
                        let ha = ha_client.clone();
                        let tx = wake_tx.clone();
                        tokio::spawn(async move {
                            let _ = ha.refresh_states().await;
                            let _ = tx.send(()).await;
                        });
                        None
                    }
                    UiAction::ToggleFrontlight => {
                        if let Some(l) = light.as_mut() {
                            l.set_on(!l.is_on());
                            renderer.frontlight = Some(l.is_on());
                        }
                        None
                    }
                    UiAction::ToggleEntity(id) => Some(ha_client.toggle_request(&id)),
                    UiAction::SetClimateTemp(id, delta) => Some(ha_client.climate_request(&id, delta).await),
                    UiAction::MediaPlayPause(id) => Some(media("media_play_pause", &id)),
                    UiAction::MediaNext(id) => Some(media("media_next_track", &id)),
                    UiAction::MediaPrevious(id) => Some(media("media_previous_track", &id)),
                    UiAction::ActivateScene(id) => Some(ha_client.activate_request(&id)),
                };

                if let Some((domain, svc, data)) = service {
                    // Optimistic local update, then talk to HA in the background
                    // so the UI never blocks on the network.
                    ha_client.apply_optimistic(&domain, &svc, &data).await;
                    let ha = ha_client.clone();
                    let tx = wake_tx.clone();
                    tokio::spawn(async move {
                        if let Err(e) = ha.call_service(&domain, &svc, data).await {
                            tracing::warn!("{}.{} failed: {}", domain, svc, e);
                        }
                        let _ = tx.send(()).await;
                    });
                }

                redraw(&mut renderer, &mut canvas, &config, active_page, &ha_client).await;
                flush(&mut fb, &canvas, force_full);
            }

            _ = wake_rx.recv() => {
                redraw(&mut renderer, &mut canvas, &config, active_page, &ha_client).await;
                flush(&mut fb, &canvas, false);
            }

            _ = clock.tick() => {
                redraw(&mut renderer, &mut canvas, &config, active_page, &ha_client).await;
                flush(&mut fb, &canvas, false);
            }

            _ = recv_signal(&mut sigterm) => { tracing::info!("SIGTERM received"); break; }
            _ = recv_signal(&mut sigint) => { tracing::info!("SIGINT received"); break; }
            _ = recv_signal(&mut sighup) => { tracing::info!("SIGHUP received"); break; }
        }
    }

    // Leave the light the way Nickel had it, and put its last frame back.
    if let Some(l) = light.as_mut() {
        l.restore();
    }
    fb.restore();
    drop(fb);
    tracing::info!("Dashboard exited cleanly");
    // Don't wait for in-flight HTTP requests or blocked input threads.
    std::process::exit(0);
}

async fn redraw(renderer: &mut ScreenRenderer, canvas: &mut Canvas, config: &AppConfig, page: usize, ha: &HaClient) {
    let states = ha.get_all_states().await;
    let is_live = ha.is_connected().await;
    renderer.last_update = ha.last_update().await;
    renderer.render(canvas, config, page, &states, is_live);
}

fn media(service: &str, entity_id: &str) -> (String, String, serde_json::Value) {
    (
        "media_player".to_string(),
        service.to_string(),
        serde_json::json!({ "entity_id": entity_id }),
    )
}

#[cfg(unix)]
fn signal(kind: tokio::signal::unix::SignalKind) -> Option<tokio::signal::unix::Signal> {
    tokio::signal::unix::signal(kind).ok()
}

#[cfg(unix)]
async fn recv_signal(sig: &mut Option<tokio::signal::unix::Signal>) {
    match sig {
        Some(s) => {
            s.recv().await;
        }
        None => std::future::pending::<()>().await,
    }
}
