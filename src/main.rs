#![allow(dead_code, unused_imports)]

mod config;
mod display;
mod frontlight;
mod i18n;
mod ha;
mod input;
mod models;
mod paths;
mod power;
mod setup;
mod simulator;
mod ui;

use clap::{Parser, Subcommand};
use config::AppConfig;
use display::{create_framebuffer, Canvas, Refresh};
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
/// For this long after a tap, pushed updates are drawn right away (so a
/// toggled light shows its new state). Otherwise they wait for the next
/// minute tick: a busy sensor can push every few seconds, and redrawing for
/// each one keeps the CPU and the panel busy all the time.
const LIVE_REDRAW_WINDOW: Duration = Duration::from_secs(30);

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
        fb.flush(&canvas, Refresh::Full)?;
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

    // Live updates over WebSocket; the loop below only polls when that is
    // down, and otherwise just refreshes forecasts/graph history now and then.
    ha_client.spawn_live_updates(wake_tx.clone());
    {
        let ha = ha_client.clone();
        let tx = wake_tx.clone();
        let interval = Duration::from_secs(config.home_assistant.refresh_interval_secs.max(5));
        tokio::spawn(async move {
            loop {
                if ha.is_live_push() {
                    ha.refresh_extras_only().await;
                } else {
                    let _ = ha.refresh_states().await;
                }
                if tx.send(()).await.is_err() {
                    return;
                }
                tokio::time::sleep(interval).await;
            }
        });
    }

    // Battery log, to compare power settings.
    tokio::spawn(async {
        loop {
            power::log_battery();
            tokio::time::sleep(Duration::from_secs(600)).await;
        }
    });

    let mut sigterm = signal(tokio::signal::unix::SignalKind::terminate());
    let mut sigint = signal(tokio::signal::unix::SignalKind::interrupt());
    let mut sighup = signal(tokio::signal::unix::SignalKind::hangup());

    // Partial updates since the last full refresh; only updates that changed
    // pixels count, so an unchanged screen never triggers a full flash.
    let mut partials: u32 = 0;
    let mut last_tap = Instant::now() - TAP_DEBOUNCE;
    let mut flush = |fb: &mut Box<dyn display::FramebufferDriver>, canvas: &Canvas, force_full: bool| {
        let cycles = config.device.full_refresh_cycles;
        let refresh = if force_full {
            Refresh::Full
        } else if cycles > 0 && partials >= cycles {
            Refresh::FullIfChanged
        } else {
            Refresh::Partial
        };
        match fb.flush(canvas, refresh) {
            Ok(changed) if refresh == Refresh::Full || (changed && refresh == Refresh::FullIfChanged) => partials = 0,
            Ok(true) => partials += 1,
            Ok(false) => {}
            Err(e) => tracing::error!("Framebuffer flush failed: {}", e),
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
    match (&light, light_timeout.is_zero()) {
        (None, _) => tracing::info!("Frontlight: not available"),
        (Some(_), true) => tracing::info!("Frontlight: auto-off disabled (frontlight_timeout_secs = 0)"),
        (Some(_), false) => tracing::info!("Frontlight: auto-off after {:?} without taps", light_timeout),
    }

    // Sleep mode: suspend after a while without taps, wake on a timer to
    // refresh, or on the power button to use the dashboard again.
    let wifi_script = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("wifi.sh")))
        .unwrap_or_else(|| PathBuf::from("/mnt/onboard/.kobo-ha/wifi.sh"));
    let sleep_after = Duration::from_secs(config.device.sleep_after_secs as u64);
    let sleep_wake = Duration::from_secs(config.device.sleep_wake_secs.max(120) as u64);
    let mut sleep_enabled = if sleep_after.is_zero() {
        tracing::info!("Sleep mode: disabled (sleep_after_secs = 0)");
        false
    } else if let Some(reason) = power::sleep_unsupported_reason(&wifi_script) {
        tracing::warn!("Sleep mode: unavailable, {}", reason);
        false
    } else {
        tracing::info!("Sleep mode: after {:?} without taps, waking every {:?}", sleep_after, sleep_wake);
        true
    };
    // The power-button press that wakes the device must not count as "exit".
    let mut ignore_power_until = Instant::now();
    // Sleep rounds in a row where the kernel refused to suspend.
    let mut sleep_failures = 0u32;
    // The power button asks for sleep right away, like it does in Nickel.
    let mut sleep_requested = false;
    let mut last_power_press = Instant::now() - Duration::from_secs(10);

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
        let sleep_deadline = if sleep_requested {
            Some(tokio::time::Instant::now())
        } else {
            sleep_enabled.then(|| tokio::time::Instant::from_std(last_activity + sleep_after))
        };
        tokio::select! {
            _ = async {
                match sleep_deadline {
                    Some(at) => tokio::time::sleep_until(at).await,
                    None => std::future::pending::<()>().await,
                }
            } => {
                sleep_requested = false;
                if power::is_plugged() {
                    // Suspending while plugged in hangs MediaTek Kobos; and power isn't an issue.
                    tracing::info!("Sleep postponed: the Kobo is plugged in");
                    last_activity = Instant::now();
                    continue;
                }
                tracing::info!("Entering sleep mode");
                let light_was_on = light.as_ref().map(|l| l.is_on()).unwrap_or(false) || light_dimmed;
                let mut woke_by_button = false;
                // Wi-Fi is left off when waking by button, and switched back on
                // after the dashboard is on screen again.
                let mut wifi_down = false;
                if let Some(l) = light.as_mut() {
                    l.set_on(false);
                    renderer.frontlight = Some(false);
                }
                loop {
                    let next = ui::widgets::header::now_local(&config.theme)
                        + chrono::Duration::from_std(sleep_wake).unwrap_or_else(|_| chrono::Duration::minutes(15));
                    renderer.sleep_until = Some(ui::widgets::header::format_time(&next, &config.theme));
                    redraw(&mut renderer, &mut canvas, &config, active_page, &ha_client).await;
                    flush(&mut fb, &canvas, true);
                    // The display controller (hwtcon) refuses to suspend while its
                    // panel power-off timer runs, a few seconds after the last update
                    // ("xon off timer pending" in dmesg). With Wi-Fi off and the
                    // state-extended pause this adds up to about 9 s. A tap or the
                    // power button meanwhile means "stay awake".
                    let wait_until = tokio::time::Instant::now() + Duration::from_secs(7);
                    let mut cancelled = false;
                    while !cancelled {
                        tokio::select! {
                            _ = tokio::time::sleep_until(wait_until) => break,
                            Some(ev) = touch_driver.recv() => cancelled = wants_awake(&ev, last_power_press, ignore_power_until),
                        }
                    }
                    if cancelled {
                        tracing::info!("Sleep cancelled by a tap or the power button");
                        woke_by_button = true;
                        break;
                    }

                    if power::is_plugged() {
                        tracing::info!("Plugged in: staying awake");
                        break;
                    }
                    let script = wifi_script.clone();
                    if !tokio::task::spawn_blocking(move || power::wifi(&script, false)).await.unwrap_or(false) {
                        tracing::warn!("Could not switch Wi-Fi off; not suspending");
                        let script = wifi_script.clone();
                        let _ = tokio::task::spawn_blocking(move || power::wifi(&script, true)).await;
                        break;
                    }
                    // Pressed while Wi-Fi was going down?
                    let mut cancelled = false;
                    while let Some(ev) = touch_driver.try_recv() {
                        cancelled |= wants_awake(&ev, last_power_press, ignore_power_until);
                    }
                    if cancelled {
                        tracing::info!("Sleep cancelled by a tap or the power button");
                        woke_by_button = true;
                        wifi_down = true;
                        break;
                    }
                    if let Err(e) = power::set_alarm(sleep_wake.as_secs()) {
                        tracing::warn!("Could not set the wakeup alarm ({}); not suspending", e);
                        let script = wifi_script.clone();
                        let _ = tokio::task::spawn_blocking(move || power::wifi(&script, true)).await;
                        break;
                    }
                    // Wall-clock time: monotonic clocks stop while suspended.
                    let alarm_at = chrono::Utc::now() + chrono::Duration::from_std(sleep_wake).unwrap_or_default();
                    // A refused suspend fails at once (usually the display still
                    // busy); a few seconds later the kernel takes it.
                    let mut suspended = Err(String::new());
                    for attempt in 1..=3 {
                        suspended = tokio::task::spawn_blocking(power::suspend).await.unwrap_or_else(|e| Err(e.to_string()));
                        let Err(e) = &suspended else { break };
                        tracing::warn!("Suspend attempt {} failed: {}", attempt, e);
                        if attempt < 3 {
                            tokio::time::sleep(Duration::from_secs(3)).await;
                        }
                    }
                    if suspended.is_err() && sleep_failures == 0 {
                        power::log_suspend_diagnostics();
                    }
                    power::clear_alarm();
                    sleep_failures = if suspended.is_ok() { 0 } else { sleep_failures + 1 };
                    let woke_early = chrono::Utc::now() < alarm_at - chrono::Duration::seconds(20);
                    if woke_early {
                        // The press that woke us shows up in the power-key watcher a moment later.
                        ignore_power_until = Instant::now() + Duration::from_secs(5);
                    }
                    match &suspended {
                        Ok(()) if woke_early => tracing::info!("Woke up: power button"),
                        Ok(()) => tracing::info!("Woke up: timer"),
                        Err(_) => tracing::warn!("The kernel refused to suspend"),
                    }
                    if suspended.is_ok() {
                        power::log_wakeup_reason();
                    }
                    power::sync();
                    woke_by_button = suspended.is_ok() && woke_early;
                    // Give the display controller a moment after resume before drawing.
                    tokio::time::sleep(Duration::from_millis(1500)).await;
                    touch_driver.drain();
                    if suspended.is_err() || woke_early {
                        wifi_down = true;
                        break;
                    }

                    let script = wifi_script.clone();
                    let wifi_ok = tokio::task::spawn_blocking(move || power::wifi(&script, true)).await.unwrap_or(false);
                    ha_client.reconnect_now();
                    if wifi_ok {
                        let _ = tokio::time::timeout(Duration::from_secs(20), ha_client.refresh_states()).await;
                    }
                    power::log_battery();
                }
                renderer.sleep_until = None;
                // Pressing the power button means "I want to use it": light back
                // on. Otherwise it stays off and the next tap brings it back, as
                // after the auto-off.
                light_dimmed = false;
                if light_was_on {
                    if woke_by_button {
                        if let Some(l) = light.as_mut() {
                            l.set_on(true);
                            renderer.frontlight = Some(true);
                        }
                    } else {
                        light_dimmed = true;
                    }
                }
                if sleep_failures >= 3 {
                    sleep_enabled = false;
                    tracing::warn!("Sleep mode: the kernel refused to suspend {} times in a row; staying awake for this session", sleep_failures);
                }
                last_activity = Instant::now();
                // Screen first, so the power button feels immediate; fresh
                // states follow once Wi-Fi is back.
                redraw(&mut renderer, &mut canvas, &config, active_page, &ha_client).await;
                flush(&mut fb, &canvas, true);
                tracing::info!("Left sleep mode");
                power::sync();
                if wifi_down {
                    let script = wifi_script.clone();
                    let ha = ha_client.clone();
                    let tx = wake_tx.clone();
                    tokio::spawn(async move {
                        if tokio::task::spawn_blocking(move || power::wifi(&script, true)).await.unwrap_or(false) {
                            ha.reconnect_now();
                            let _ = tokio::time::timeout(Duration::from_secs(20), ha.refresh_states()).await;
                            let _ = tx.send(()).await;
                        }
                    });
                }
            }

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
                    InputEvent::PowerButton if Instant::now() < ignore_power_until => continue,
                    // Press and release can arrive as two events.
                    InputEvent::PowerButton if last_power_press.elapsed() < Duration::from_secs(2) => continue,
                    InputEvent::PowerButton => {
                        last_power_press = Instant::now();
                        if sleep_enabled && !power::is_plugged() {
                            tracing::info!("Power button pressed: going to sleep");
                            sleep_requested = true;
                            continue;
                        }
                        // No suspend while plugged in (or with sleep mode off):
                        // the button switches the light instead, like a screen sleep.
                        if let Some(l) = light.as_mut() {
                            let on = !l.is_on();
                            tracing::info!("Power button pressed: light {} (sleep mode not available now)", if on { "on" } else { "off" });
                            l.set_on(on);
                            light_dimmed = !on;
                            renderer.frontlight = Some(on);
                            last_activity = Instant::now();
                            redraw(&mut renderer, &mut canvas, &config, active_page, &ha_client).await;
                            flush(&mut fb, &canvas, false);
                        }
                        continue;
                    }
                    InputEvent::Plugged => {
                        // Nickel is paused and can't ask "connect to computer?"; give it the screen back.
                        tracing::info!("Connected to a computer: returning to the Kobo");
                        break;
                    }
                    InputEvent::PowerSourceChanged => {
                        // Charging bolt on the battery icon
                        redraw(&mut renderer, &mut canvas, &config, active_page, &ha_client).await;
                        flush(&mut fb, &canvas, false);
                        continue;
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
                // Several entities often change together: draw once for the batch.
                tokio::time::sleep(Duration::from_millis(400)).await;
                while wake_rx.try_recv().is_ok() {}
                // When idle, the minute tick below draws the new states.
                if last_activity.elapsed() < LIVE_REDRAW_WINDOW {
                    redraw(&mut renderer, &mut canvas, &config, active_page, &ha_client).await;
                    flush(&mut fb, &canvas, false);
                }
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

    // Presses meant for us must not put Nickel to sleep once it resumes.
    input::discard_nickel_power_presses();
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

/// During the run-up to sleep: does this event mean the user wants the
/// dashboard? Skips the press that asked for sleep (press and release can
/// arrive a poll apart) and the one that woke the device.
fn wants_awake(event: &InputEvent, last_power_press: Instant, ignore_power_until: Instant) -> bool {
    match event {
        InputEvent::PowerButton => last_power_press.elapsed() >= Duration::from_secs(2) && Instant::now() >= ignore_power_until,
        InputEvent::Tap { .. } | InputEvent::Plugged => true,
        InputEvent::PowerSourceChanged => false,
    }
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
