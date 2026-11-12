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

/// Log timestamps in local time (the Kobo's TZ, as the clock on screen),
/// with the offset, so they line up with launch.sh's `date` line.
struct LocalTime;

impl tracing_subscriber::fmt::time::FormatTime for LocalTime {
    fn format_time(&self, w: &mut tracing_subscriber::fmt::format::Writer<'_>) -> std::fmt::Result {
        write!(w, "{}", chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%.6f%:z"))
    }
}

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

/// For this long after a tap, pushed updates are drawn right away (so a
/// toggled light shows its new state). Otherwise they wait for the next
/// minute tick: a busy sensor can push every few seconds, and redrawing for
/// each one keeps the CPU and the panel busy all the time.
const LIVE_REDRAW_WINDOW: Duration = Duration::from_secs(30);
/// With an empty battery, wake this often to see if a charger was connected.
const BATTERY_EMPTY_WAKE: Duration = Duration::from_secs(300);
/// After a press wakes it, stay up this long before suspending again. A cable
/// that just went in takes a few seconds to show up, and suspending while
/// plugged in hangs MediaTek Kobos.
const BATTERY_EMPTY_PRESS_WAIT: Duration = Duration::from_secs(15);
/// When it can't suspend, try again after this long.
const BATTERY_EMPTY_RETRY: Duration = Duration::from_secs(60);
/// Wake-ups by something else than the user or the timer (the Wi-Fi/Bluetooth
/// chip while Bluetooth is on) before the sleep period is waited out awake.
const OTHER_WAKES_MAX: u32 = 3;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "kobo_ha_dashboard=info".into()),
        )
        .with(tracing_subscriber::fmt::layer().with_ansi(false).with_timer(LocalTime))
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
    let mut canvas = Canvas::for_screen(width, height);
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

    let (swap_xy, invert_x, invert_y) = config.device.touch_axes(&std::env::var("PRODUCT").unwrap_or_default());
    let (mut touch_driver, _touch_tx) = TouchDriver::new(
        &config.device.touch_device,
        TouchMapping {
            portrait_w,
            portrait_h,
            swap_xy,
            invert_x,
            invert_y,
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
    // "Bluetooth is on, no sleep" said once per session.
    let mut bluetooth_noted = false;
    // The power button asks for sleep right away, like it does in Nickel.
    let mut sleep_requested = false;
    let mut last_power_press = Instant::now() - Duration::from_secs(10);
    let battery_critical = config.device.battery_critical_percent;

    {
        redraw(&mut renderer, &mut canvas, &config, active_page, &ha_client).await;
        flush(&mut fb, &canvas, true);
    }

    // Wi-Fi is handed back to Nickel switched off after sleep mode used it, and
    // Nickel may not have switched it on again: bring it up here. Only where
    // sleep mode works, since that switches Wi-Fi the same way anyway.
    if sleep_enabled && read_wifi_state().as_deref() != Some("up") {
        tracing::info!("Wi-Fi is not up; switching it on");
        wifi_on_in_background(wifi_script.clone(), ha_client.clone(), wake_tx.clone());
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
                if power::bluetooth_on() {
                    // A suspend attempt with Bluetooth on crashed the
                    // Wi-Fi/Bluetooth chip (firmware 6.0): Wi-Fi was gone
                    // until a restart. The light still goes off.
                    if !bluetooth_noted {
                        tracing::warn!("Bluetooth is on: no sleep mode while it is (switch Bluetooth off in the Kobo's settings to use it)");
                        bluetooth_noted = true;
                    }
                    last_activity = Instant::now();
                    continue;
                }
                tracing::info!("Entering sleep mode");
                let light_was_on = light.as_ref().map(|l| l.is_on()).unwrap_or(false) || light_dimmed;
                let mut woke_by_button = false;
                // Wi-Fi is left off when waking by button, and switched back on
                // after the dashboard is on screen again.
                let mut wifi_down = false;
                // A computer connected while asleep: back to the Kobo afterwards.
                let mut computer = false;
                if let Some(l) = light.as_mut() {
                    l.set_on(false);
                    renderer.frontlight = Some(false);
                }
                loop {
                    // The wake-up time on the screen is the alarm itself, so it
                    // can't drift from the drawing and Wi-Fi time before suspending.
                    let alarm_at = wake_time(chrono::Utc::now(), sleep_wake);
                    renderer.sleep_until = Some(ui::widgets::header::format_time(&ui::widgets::header::to_local(alarm_at, &config.theme), &config.theme));
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
                            Some(ev) = touch_driver.recv() => {
                                computer |= ev == InputEvent::Plugged;
                                cancelled = wants_awake(&ev, last_power_press, ignore_power_until);
                            }
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
                    ha_client.network_down().await;
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
                        computer |= ev == InputEvent::Plugged;
                        cancelled |= wants_awake(&ev, last_power_press, ignore_power_until);
                    }
                    if cancelled {
                        tracing::info!("Sleep cancelled by a tap or the power button");
                        woke_by_button = true;
                        wifi_down = true;
                        break;
                    }
                    if let Err(e) = power::set_alarm(seconds_until(alarm_at)) {
                        tracing::warn!("Could not set the wakeup alarm ({}); not suspending", e);
                        let script = wifi_script.clone();
                        let _ = tokio::task::spawn_blocking(move || power::wifi(&script, true)).await;
                        break;
                    }
                    let mut suspended;
                    let mut woke_early;
                    let mut other_wakes = 0;
                    loop {
                        // A refused suspend fails at once (usually the display still
                        // busy); a few seconds later the kernel takes it.
                        suspended = Err(String::new());
                        let mut bluetooth_blocks = false;
                        for attempt in 1..=3 {
                            suspended = tokio::task::spawn_blocking(power::suspend).await.unwrap_or_else(|e| Err(e.to_string()));
                            let Err(e) = &suspended else { break };
                            tracing::warn!("Suspend attempt {} failed: {}", attempt, e);
                            if power::suspend_blocked_by_bluetooth() {
                                // Retrying crashed the Wi-Fi/Bluetooth chip on firmware 6.0.
                                tracing::warn!("Bluetooth is on and the kernel won't suspend with it; sleep mode is off for this session (switch Bluetooth off in the Kobo's settings to use it)");
                                bluetooth_blocks = true;
                                break;
                            }
                            if attempt < 3 {
                                tokio::time::sleep(Duration::from_secs(3)).await;
                            }
                        }
                        if suspended.is_err() && sleep_failures == 0 {
                            power::log_suspend_diagnostics();
                        }
                        power::clear_alarm();
                        sleep_failures = if suspended.is_ok() { 0 } else { sleep_failures + 1 };
                        if bluetooth_blocks {
                            sleep_failures = sleep_failures.max(3);
                        }
                        woke_early = chrono::Utc::now() < alarm_at - chrono::Duration::seconds(20);
                        let source = if suspended.is_ok() { power::log_wakeup_reason() } else { None };
                        let other = match &source {
                            Some(s) if suspended.is_ok() && woke_early && !power::is_power_button_source(s) => s.clone(),
                            _ => {
                                woke_by_button = suspended.is_ok() && woke_early;
                                break;
                            }
                        };
                        // Woken by something else than the user (the Wi-Fi/Bluetooth
                        // chip while Bluetooth is on): no light, no Wi-Fi, back to sleep.
                        other_wakes += 1;
                        let left = (alarm_at - chrono::Utc::now()).num_seconds().max(0) as u64;
                        if other_wakes <= OTHER_WAKES_MAX {
                            tracing::info!("Woke up: {}, not the power button; back to sleep", other);
                            match power::set_alarm(left) {
                                Ok(()) => continue,
                                Err(e) => tracing::warn!("Could not set the wakeup alarm ({}); not suspending", e),
                            }
                        } else {
                            tracing::info!("Woke up: {} again, {} times; staying awake until the wake-up time", other, other_wakes);
                        }
                        // Can't stay suspended: wait awake (light and Wi-Fi off)
                        // for the wake-up time or the user.
                        let deadline = tokio::time::Instant::now() + Duration::from_secs(left);
                        woke_by_button = false;
                        loop {
                            tokio::select! {
                                _ = tokio::time::sleep_until(deadline) => break,
                                Some(ev) = touch_driver.recv() => {
                                    computer |= ev == InputEvent::Plugged;
                                    if wants_awake(&ev, last_power_press, ignore_power_until) {
                                        woke_by_button = true;
                                        break;
                                    }
                                }
                            }
                        }
                        woke_early = woke_by_button;
                        break;
                    }
                    if woke_by_button && suspended.is_ok() && other_wakes == 0 {
                        // The press that woke us shows up in the power-key watcher a moment later.
                        ignore_power_until = Instant::now() + Duration::from_secs(5);
                    }
                    match &suspended {
                        Ok(()) if woke_by_button => tracing::info!("Woke up: power button"),
                        Ok(()) => tracing::info!("Woke up: timer"),
                        Err(_) => tracing::warn!("The kernel refused to suspend"),
                    }
                    power::sync();
                    // Give the display controller a moment after resume before drawing.
                    tokio::time::sleep(Duration::from_millis(1500)).await;
                    computer |= touch_driver.drain();
                    if suspended.is_err() || woke_early || computer {
                        wifi_down = true;
                        break;
                    }

                    // No Wi-Fi for this wake-up if the battery is out: stay down
                    // until it's charged, then the dashboard first, Wi-Fi after.
                    if power::battery_critical(battery_critical) {
                        computer |= battery_empty_sleep(&mut fb, &mut canvas, &mut renderer, &mut light, &config, active_page, &ha_client, &mut touch_driver, &wifi_script, false).await;
                        wifi_down = true;
                        break;
                    }

                    // Wi-Fi takes seconds (a minute with no known network): the
                    // power button or a tap meanwhile gets the dashboard at once.
                    let script = wifi_script.clone();
                    let mut switching = tokio::task::spawn_blocking(move || power::wifi(&script, true));
                    let mut pressed = false;
                    let wifi_ok = loop {
                        tokio::select! {
                            done = &mut switching => break done.unwrap_or(false),
                            Some(ev) = touch_driver.recv() => {
                                computer |= ev == InputEvent::Plugged;
                                if wants_awake(&ev, last_power_press, ignore_power_until) {
                                    pressed = true;
                                    break false;
                                }
                            }
                        }
                    };
                    if pressed {
                        tracing::info!("Woke up: power button or tap, while Wi-Fi was coming up");
                        woke_by_button = true;
                        let ha = ha_client.clone();
                        let tx = wake_tx.clone();
                        tokio::spawn(async move {
                            if switching.await.unwrap_or(false) {
                                ha.reconnect_now();
                                let _ = tokio::time::timeout(Duration::from_secs(20), ha.refresh_states()).await;
                                let _ = tx.send(()).await;
                            }
                        });
                        break;
                    }
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
                if computer {
                    tracing::info!("Connected to a computer while asleep: returning to the Kobo");
                    break;
                }
                if wifi_down {
                    wifi_on_in_background(wifi_script.clone(), ha_client.clone(), wake_tx.clone());
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
                        if sleep_enabled && !power::is_plugged() && !power::bluetooth_on() {
                            tracing::info!("Power button pressed: going to sleep");
                            sleep_requested = true;
                            continue;
                        }
                        // No suspend while plugged in, with Bluetooth on or sleep mode off:
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
                // Bounces are already filtered by the touch listener, on the
                // kernel's event times (not when we get round to reading them).
                last_activity = Instant::now();

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

                let action = renderer.handle_touch(x, y);
                // Every tap with what it hit: to see where the touch lands on new models.
                tracing::info!("Tap at ({:.0}, {:.0}) -> {:?}", x, y, action);
                let Some(action) = action else { continue };
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
                if power::battery_critical(battery_critical) {
                    let light_was_on = light.as_ref().map(|l| l.is_on()).unwrap_or(false) || light_dimmed;
                    let computer = battery_empty_sleep(&mut fb, &mut canvas, &mut renderer, &mut light, &config, active_page, &ha_client, &mut touch_driver, &wifi_script, true).await;
                    if computer {
                        tracing::info!("Connected to a computer: returning to the Kobo");
                        break;
                    }
                    // As after sleep mode: the next tap brings the light back.
                    light_dimmed = light_was_on;
                    last_activity = Instant::now();
                    redraw(&mut renderer, &mut canvas, &config, active_page, &ha_client).await;
                    flush(&mut fb, &canvas, true);
                    wifi_on_in_background(wifi_script.clone(), ha_client.clone(), wake_tx.clone());
                    continue;
                }
                redraw(&mut renderer, &mut canvas, &config, active_page, &ha_client).await;
                flush(&mut fb, &canvas, false);
            }

            _ = recv_signal(&mut sigterm) => { tracing::info!("SIGTERM received"); break; }
            _ = recv_signal(&mut sigint) => { tracing::info!("SIGINT received"); break; }
            _ = recv_signal(&mut sighup) => { tracing::info!("SIGHUP received"); break; }
        }
    }

    // A Wi-Fi switch still running (say "on" with no known network, which
    // takes a minute) is stopped rather than waited for: launch.sh switches
    // Wi-Fi off for Nickel after we exit, and the two must not overlap.
    if power::wifi_busy() {
        tracing::info!("Stopping a Wi-Fi switch still running before exiting");
        let _ = tokio::task::spawn_blocking(|| power::stop_wifi(Duration::from_secs(3))).await;
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

/// Bring Wi-Fi back without holding up the screen, then fetch fresh states.
#[cfg(unix)]
fn wifi_on_in_background(script: PathBuf, ha: HaClient, tx: tokio::sync::mpsc::Sender<()>) {
    tokio::spawn(async move {
        if tokio::task::spawn_blocking(move || power::wifi(&script, true)).await.unwrap_or(false) {
            ha.reconnect_now();
            let _ = tokio::time::timeout(Duration::from_secs(20), ha.refresh_states()).await;
            let _ = tx.send(()).await;
        }
    });
}

/// The battery is out: say so on the screen (e-ink keeps it without power) and
/// suspend until the Kobo is charged. Nickel is paused and can't do this for
/// us. Wakes on a timer to look for a charger, because suspending while plugged
/// in hangs MediaTek Kobos; the power button wakes it for the same check. Where
/// suspending isn't possible, it just idles with the message up. True if a
/// computer was connected meanwhile.
#[cfg(unix)]
#[allow(clippy::too_many_arguments)]
async fn battery_empty_sleep(
    fb: &mut Box<dyn display::FramebufferDriver>,
    canvas: &mut Canvas,
    renderer: &mut ScreenRenderer,
    light: &mut Option<frontlight::Frontlight>,
    config: &AppConfig,
    page: usize,
    ha: &HaClient,
    touch_driver: &mut TouchDriver,
    wifi_script: &Path,
    mut wifi_up: bool,
) -> bool {
    let critical = config.device.battery_critical_percent;
    tracing::warn!("Battery at or below {}%: sleeping until the Kobo is charged", critical);
    power::log_battery();
    if let Some(l) = light.as_mut() {
        l.set_on(false);
        renderer.frontlight = Some(false);
    }
    renderer.battery_empty = true;
    redraw(renderer, canvas, config, page, ha).await;
    renderer.battery_empty = false;
    if let Err(e) = fb.flush(canvas, Refresh::Full) {
        tracing::error!("Framebuffer flush failed: {}", e);
    }
    power::sync();

    let unsupported = power::sleep_unsupported_reason(wifi_script);
    if let Some(reason) = &unsupported {
        tracing::warn!("Battery empty: can't suspend, {}", reason);
    }
    // The display controller refuses to suspend right after an update.
    tokio::time::sleep(Duration::from_secs(7)).await;

    // Suspending with Bluetooth on crashes the Wi-Fi/Bluetooth chip (firmware 6.0).
    let mut bluetooth_blocks = power::bluetooth_on();
    if bluetooth_blocks {
        tracing::warn!("Battery empty: Bluetooth is on, waiting awake instead of suspending");
    }
    while !power::battery_recovered(critical) {
        if wifi_up && unsupported.is_none() {
            ha.network_down().await;
            let script = wifi_script.to_path_buf();
            wifi_up = !tokio::task::spawn_blocking(move || power::wifi(&script, false)).await.unwrap_or(false);
        }
        let mut wait = BATTERY_EMPTY_RETRY;
        if !wifi_up && unsupported.is_none() && !bluetooth_blocks {
            match power::set_alarm(BATTERY_EMPTY_WAKE.as_secs()) {
                Ok(()) => {
                    // Wall-clock time: monotonic clocks stop while suspended.
                    let alarm_at = chrono::Utc::now() + chrono::Duration::from_std(BATTERY_EMPTY_WAKE).unwrap_or_default();
                    let result = tokio::task::spawn_blocking(power::suspend).await.unwrap_or_else(|e| Err(e.to_string()));
                    power::clear_alarm();
                    match result {
                        Ok(()) if chrono::Utc::now() < alarm_at - chrono::Duration::seconds(20) => wait = BATTERY_EMPTY_PRESS_WAIT,
                        Ok(()) => wait = Duration::ZERO,
                        Err(e) => {
                            tracing::warn!("Battery empty: {}", e);
                            if power::suspend_blocked_by_bluetooth() {
                                tracing::warn!("Battery empty: Bluetooth is on and blocks suspending; waiting awake instead");
                                bluetooth_blocks = true;
                            }
                        }
                    }
                }
                Err(e) => tracing::warn!("Could not set the wakeup alarm ({}); not suspending", e),
            }
        }
        power::sync();
        // Keep looking for the charger meanwhile: the dashboard should be back
        // within a moment of it going in.
        let until = Instant::now() + wait;
        while Instant::now() < until && !power::battery_recovered(critical) {
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }
    let computer = touch_driver.drain();
    tracing::info!("Battery empty mode over");
    power::log_battery();
    power::sync();
    computer
}

fn read_wifi_state() -> Option<String> {
    std::fs::read_to_string(power::wifi_operstate_path()).ok().map(|s| s.trim().to_string())
}

/// When sleep mode wakes up next: `every` from now, rounded up to a whole
/// minute so the time on the screen is the moment it happens. Wall-clock
/// time, because monotonic clocks stop while suspended.
fn wake_time(now: chrono::DateTime<chrono::Utc>, every: Duration) -> chrono::DateTime<chrono::Utc> {
    let at = now + chrono::Duration::from_std(every).unwrap_or_else(|_| chrono::Duration::minutes(15));
    let into_minute = at.timestamp().rem_euclid(60);
    let rounded = at - chrono::Duration::seconds(into_minute) - chrono::Duration::nanoseconds(at.timestamp_subsec_nanos() as i64);
    if rounded == at { at } else { rounded + chrono::Duration::minutes(1) }
}

/// Seconds from now until `at` (at least 1, for the RTC alarm).
fn seconds_until(at: chrono::DateTime<chrono::Utc>) -> u64 {
    (at - chrono::Utc::now()).num_seconds().max(1) as u64
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn wake_time_is_the_whole_minute_shown_on_screen() {
        let at = |h, m, s| chrono::Utc.with_ymd_and_hms(2026, 11, 12, h, m, s).unwrap();
        // 00:14:01 + 5 min = 00:19:01, shown as 00:19 but woke ~a minute later: now 00:20:00
        assert_eq!(wake_time(at(0, 14, 1), Duration::from_secs(300)), at(0, 20, 0));
        assert_eq!(wake_time(at(0, 14, 59), Duration::from_secs(300)), at(0, 20, 0));
        // Already on the minute: stays
        assert_eq!(wake_time(at(0, 14, 0), Duration::from_secs(300)), at(0, 19, 0));
        let with_nanos = at(0, 14, 0) + chrono::Duration::milliseconds(300);
        assert_eq!(wake_time(with_nanos, Duration::from_secs(300)), at(0, 20, 0));
    }
}
