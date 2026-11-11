//! Battery, Wi-Fi and suspend-to-RAM helpers for the Kobo.
//!
//! The suspend sequence follows what Nickel and KOReader do on Kobo:
//! flag `/sys/power/state-extended`, wait, sync, write `mem` to
//! `/sys/power/state` (returns after wakeup), then unflag. Two rules from
//! KOReader's notes are enforced by the caller: never suspend with Wi-Fi up
//! (it can crash the kernel), and never suspend while plugged in on MediaTek
//! devices (the kernel hangs).

use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// How long `wifi.sh` may take. Normally 0.3 s to switch off and 7 s to switch
/// on; these limits also cover its 60 s wait for the lock and, for "on", its
/// own timeouts for joining a network and getting an address.
const WIFI_OFF_TIMEOUT: Duration = Duration::from_secs(90);
const WIFI_ON_TIMEOUT: Duration = Duration::from_secs(180);

/// Battery level to leave the "battery empty" state at without a charger, a
/// little above the critical level so the gauge's jitter doesn't flip it.
const BATTERY_RECOVERED_MARGIN: u32 = 3;

/// The battery, preferring one whose charge level can be read (as the header
/// does).
fn battery_dir() -> Option<PathBuf> {
    let batteries: Vec<PathBuf> = std::fs::read_dir("/sys/class/power_supply")
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| std::fs::read_to_string(p.join("type")).map(|t| t.trim() == "Battery").unwrap_or(false))
        .collect();
    batteries.iter().find(|p| read_trim(&p.join("capacity")).is_some()).or(batteries.first()).cloned()
}

/// `operstate` of the Wi-Fi interface (`INTERFACE` from Nickel: eth0 on
/// older Kobos such as the Glo, wlan0 on newer ones).
pub fn wifi_operstate_path() -> PathBuf {
    operstate_path(std::env::var("INTERFACE").ok().as_deref())
}

fn operstate_path(interface: Option<&str>) -> PathBuf {
    let iface = interface.filter(|i| !i.is_empty()).unwrap_or("wlan0");
    PathBuf::from(format!("/sys/class/net/{}/operstate", iface))
}

/// Left by `wifi.sh off` until Wi-Fi is back on.
pub fn wifi_switched_off() -> bool {
    Path::new("/tmp/kobo-ha-wifi-off").exists()
}

fn read_trim(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

/// Charge level, if the kernel reports one.
fn battery_percent() -> Option<u32> {
    read_trim(&battery_dir()?.join("capacity"))?.parse().ok()
}

fn is_critical(percent: Option<u32>, plugged: bool, critical: u32) -> bool {
    !plugged && matches!(percent, Some(p) if p <= critical)
}

fn is_recovered(percent: Option<u32>, plugged: bool, critical: u32) -> bool {
    plugged || !matches!(percent, Some(p) if p < critical + BATTERY_RECOVERED_MARGIN)
}

/// On battery and at or below `critical` percent (0 = never). The gauge can
/// jump, 5% to 0% between two readings, so this is checked against a level
/// with some reserve; "0%" still ran for hours in sleep mode.
pub fn battery_critical(critical: u32) -> bool {
    critical > 0 && is_critical(battery_percent(), is_plugged(), critical)
}

/// Plugged in again, or (a false alarm) well above the critical level.
pub fn battery_recovered(critical: u32) -> bool {
    is_recovered(battery_percent(), is_plugged(), critical)
}

/// One line in the log with charge level, charging state and current draw.
pub fn log_battery() {
    let Some(dir) = battery_dir() else {
        tracing::info!("No battery found in /sys/class/power_supply");
        return;
    };
    // Unreadable on the Kobo Nia's log (issue #4): say why.
    let capacity = match std::fs::read_to_string(dir.join("capacity")) {
        Ok(c) => c.trim().to_string(),
        Err(e) => format!("? ({}: {})", dir.join("capacity").display(), e),
    };
    let status = read_trim(&dir.join("status")).unwrap_or_else(|| "?".into());
    // current_now is in µA on most kernels; the sign convention varies.
    let current = read_trim(&dir.join("current_now"))
        .and_then(|c| c.parse::<i64>().ok())
        .map(|ua| format!(", {} mA", ua.abs() / 1000))
        .unwrap_or_default();
    let wifi = read_trim(&wifi_operstate_path()).unwrap_or_else(|| "absent".into());
    let plugged = if is_plugged() { "plugged in" } else { "on battery" };
    let supplies: Vec<String> = chargers().iter().map(|(name, online)| format!("{}={}", name, online)).collect();
    tracing::info!("Battery: {}% {}{} ({}; online: {}) | Wi-Fi: {}", capacity, status, current, plugged, supplies.join(" "), wifi);
}

/// Non-battery supplies and their `online` value, e.g. ("usb", "0").
fn chargers() -> Vec<(String, String)> {
    std::fs::read_dir("/sys/class/power_supply")
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| read_trim(&p.join("type")).map(|t| t != "Battery").unwrap_or(false))
        .filter_map(|p| Some((p.file_name()?.to_string_lossy().into_owned(), read_trim(&p.join("online"))?)))
        .collect()
}

/// Plugged into USB/charger. Errs towards "plugged", since suspending while
/// plugged in hangs MediaTek Kobos: only a discharging battery with no charger
/// online counts as unplugged. (The Clara Colour's `bd71827_ac` reads 2, not
/// 1, while plugged in.)
pub fn is_plugged() -> bool {
    if chargers().iter().any(|(_, online)| online != "0") {
        return true;
    }
    match battery_dir().and_then(|d| read_trim(&d.join("status"))) {
        Some(status) => status != "Discharging",
        None => true,
    }
}

/// USB device-controller states, e.g. ("musb-hdrc", "configured").
fn udc_states() -> Vec<(String, String)> {
    std::fs::read_dir("/sys/class/udc")
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| Some((e.file_name().to_string_lossy().into_owned(), read_trim(&e.path().join("state"))?)))
        .collect()
}

/// Whether the cable goes to a computer rather than a wall charger. A host
/// resets and addresses the USB controller; a charger only powers it. Kobos
/// without a bound gadget may not report that, so a supply reading `online=2`
/// (USB, as opposed to 1 for a charger) counts too.
pub fn connected_to_computer() -> bool {
    let udc = udc_states();
    let supplies = chargers();
    tracing::info!("USB: controller {:?}, supplies {:?}", udc, supplies);
    udc.iter().any(|(_, s)| matches!(s.as_str(), "default" | "addressed" | "configured" | "suspended"))
        || supplies.iter().any(|(_, online)| online == "2")
}

/// Why sleep mode can't be used here, or None when it can.
pub fn sleep_unsupported_reason(wifi_script: &Path) -> Option<String> {
    let states = read_trim(Path::new("/sys/power/state")).unwrap_or_default();
    if !states.split_whitespace().any(|s| s == "mem") {
        return Some("the kernel does not offer suspend to RAM".into());
    }
    if !Path::new("/dev/rtc0").exists() {
        return Some("no real-time clock for timed wakeups".into());
    }
    if !wifi_script.exists() {
        return Some(format!("{} is missing", wifi_script.display()));
    }
    match std::env::var("WIFI_MODULE").ok().filter(|m| !m.is_empty()) {
        None => Some("WIFI_MODULE is not set (start the dashboard from NickelMenu)".into()),
        // NXP "moal" chips need an ioctl to power off that a shell script can't do.
        Some(m) if m == "moal" => Some("Wi-Fi chip 'moal' is not supported yet".into()),
        Some(_) => None,
    }
}

/// Output of a finished command; `status` is None if it ran out of time.
struct Finished {
    status: Option<ExitStatus>,
    lines: Vec<String>,
}

/// Run a command for at most `timeout`, then kill it with everything it
/// started (except daemons, which leave its process group). `before_kill` can
/// look at the stuck command while it still exists. Unlike
/// `Command::output`, this returns as soon as the command itself exits, even
/// if a daemon it started still holds the pipes, and even if the command can't
/// be killed.
fn run_with_timeout(command: Command, timeout: Duration, before_kill: fn()) -> std::io::Result<Finished> {
    run_tracked(command, timeout, before_kill, None)
}

/// `run_with_timeout`, with the command's process group listed in `groups`
/// while it runs, so it can be stopped from elsewhere.
fn run_tracked(mut command: Command, timeout: Duration, before_kill: fn(), groups: Option<&std::sync::Mutex<Vec<u32>>>) -> std::io::Result<Finished> {
    use std::io::BufRead;

    command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    let mut child = command.spawn()?;
    let pid = child.id();
    if let Some(groups) = groups {
        groups.lock().unwrap_or_else(|e| e.into_inner()).push(pid);
    }

    let (tx, rx) = std::sync::mpsc::channel::<String>();
    fn forward(pipe: impl std::io::Read + Send + 'static, tx: std::sync::mpsc::Sender<String>) {
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(pipe).split(b'\n').map_while(Result::ok) {
                let _ = tx.send(String::from_utf8_lossy(&line).trim_end().to_string());
            }
        });
    }
    if let Some(pipe) = child.stdout.take() {
        forward(pipe, tx.clone());
    }
    if let Some(pipe) = child.stderr.take() {
        forward(pipe, tx.clone());
    }
    drop(tx);

    let poll = Duration::from_millis(50);
    let deadline = Instant::now() + timeout;
    let mut lines = Vec::new();
    let status = loop {
        match rx.recv_timeout(poll) {
            Ok(line) => lines.push(line),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            // Both pipes closed: nothing to wait on but the process.
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => std::thread::sleep(poll),
        }
        if let Some(status) = child.try_wait()? {
            break Some(status);
        }
        if Instant::now() >= deadline {
            before_kill();
            #[cfg(unix)]
            unsafe {
                libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL);
            }
            let _ = child.kill();
            // Stuck in the kernel it can't die: don't get stuck with it.
            let give_up = Instant::now() + Duration::from_secs(2);
            while Instant::now() < give_up && !matches!(child.try_wait(), Ok(Some(_))) {
                std::thread::sleep(poll);
            }
            break None;
        }
    };
    while let Ok(line) = rx.recv_timeout(poll) {
        lines.push(line);
    }
    if let Some(groups) = groups {
        groups.lock().unwrap_or_else(|e| e.into_inner()).retain(|p| *p != pid);
    }
    Ok(Finished { status, lines })
}

/// What the Kobo is doing while `wifi.sh` is stuck, into our log.
fn log_wifi_hang() {
    let mut command = Command::new("/bin/sh");
    command.arg("-c").arg("ps; df /tmp; ls -la /tmp | head -40; dmesg | tail -30");
    if let Ok(out) = run_with_timeout(command, Duration::from_secs(10), || {}) {
        for line in out.lines {
            tracing::info!("  {}", line);
        }
    }
}

/// `wifi.sh` runs in progress, including ones in the background.
static WIFI_RUNS: AtomicUsize = AtomicUsize::new(0);
/// Their process groups, to stop them on exit.
static WIFI_GROUPS: std::sync::Mutex<Vec<u32>> = std::sync::Mutex::new(Vec::new());

/// Whether `wifi.sh` is running.
pub fn wifi_busy() -> bool {
    WIFI_RUNS.load(Ordering::SeqCst) > 0
}

/// Stop `wifi.sh` runs still going (on exit: launch.sh switches Wi-Fi off for
/// Nickel anyway, and a "wifi.sh on" can take minutes without a network).
/// Waits up to `wait` for them to be gone.
pub fn stop_wifi(wait: Duration) {
    #[cfg(unix)]
    for pid in WIFI_GROUPS.lock().unwrap_or_else(|e| e.into_inner()).iter() {
        unsafe {
            libc::kill(-(*pid as libc::pid_t), libc::SIGKILL);
        }
    }
    let until = Instant::now() + wait;
    while wifi_busy() && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(50));
    }
}

struct WifiRun;

impl WifiRun {
    fn start() -> Self {
        WIFI_RUNS.fetch_add(1, Ordering::SeqCst);
        WifiRun
    }
}

impl Drop for WifiRun {
    fn drop(&mut self) {
        WIFI_RUNS.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Run `wifi.sh on|off`; true on success. Returns in bounded time: a stuck
/// `wifi.sh off` once left the dashboard frozen and the Kobo awake until the
/// battery was empty.
pub fn wifi(script: &Path, on: bool) -> bool {
    let arg = if on { "on" } else { "off" };
    let _running = WifiRun::start();
    tracing::info!("Wi-Fi {}…", arg);
    let mut command = Command::new("/bin/sh");
    command.arg(script).arg(arg);
    let timeout = if on { WIFI_ON_TIMEOUT } else { WIFI_OFF_TIMEOUT };
    let out = match run_tracked(command, timeout, log_wifi_hang, Some(&WIFI_GROUPS)) {
        Ok(out) => out,
        Err(e) => {
            tracing::warn!("Could not run {}: {}", script.display(), e);
            return false;
        }
    };
    // "step:" lines say how far the script got; only of interest if it failed.
    let (steps, lines): (Vec<&String>, Vec<&String>) = out.lines.iter().partition(|l| l.starts_with("step: "));
    for line in lines {
        tracing::info!("  wifi.sh: {}", line);
    }
    let ok = out.status.is_some_and(|s| s.success());
    if !ok {
        match out.status {
            Some(status) => tracing::warn!("wifi.sh {} failed ({})", arg, status),
            None => tracing::warn!("wifi.sh {} did not finish in {:?}, killed it", arg, timeout),
        }
        for line in steps {
            tracing::warn!("  wifi.sh: {}", line);
        }
    }
    ok
}

#[cfg(target_os = "linux")]
mod rtc {
    use std::os::unix::io::AsRawFd;

    #[repr(C)]
    #[derive(Default)]
    struct RtcTime {
        tm_sec: i32,
        tm_min: i32,
        tm_hour: i32,
        tm_mday: i32,
        tm_mon: i32,
        tm_year: i32,
        tm_wday: i32,
        tm_yday: i32,
        tm_isdst: i32,
    }

    #[repr(C)]
    #[derive(Default)]
    struct RtcWkalrm {
        enabled: u8,
        pending: u8,
        time: RtcTime,
    }

    // _IOW('p', 0x0f, struct rtc_wkalrm)
    const RTC_WKALM_SET: libc::c_ulong = 0x4000_0000 | ((std::mem::size_of::<RtcWkalrm>() as libc::c_ulong) << 16) | ((b'p' as libc::c_ulong) << 8) | 0x0f;

    /// Program the RTC to wake the device `seconds` from now (RTC runs in UTC).
    pub fn set_alarm(seconds: u64) -> Result<(), String> {
        use chrono::{Datelike, Timelike};
        let at = chrono::Utc::now() + chrono::Duration::seconds(seconds as i64);
        let alarm = RtcWkalrm {
            enabled: 1,
            pending: 0,
            time: RtcTime {
                tm_sec: at.second() as i32,
                tm_min: at.minute() as i32,
                tm_hour: at.hour() as i32,
                tm_mday: at.day() as i32,
                tm_mon: at.month0() as i32,
                tm_year: at.year() - 1900,
                tm_wday: -1,
                tm_yday: -1,
                tm_isdst: -1,
            },
        };
        write_alarm(&alarm)
    }

    #[cfg(test)]
    #[test]
    fn ioctl_number_matches_the_kernel() {
        // struct rtc_wkalrm is 40 bytes: _IOW('p', 0x0f, struct rtc_wkalrm) = 0x4028700f
        assert_eq!(std::mem::size_of::<RtcWkalrm>(), 40);
        assert_eq!(RTC_WKALM_SET, 0x4028_700f);
    }

    pub fn clear_alarm() {
        let _ = write_alarm(&RtcWkalrm::default());
    }

    fn write_alarm(alarm: &RtcWkalrm) -> Result<(), String> {
        let file = std::fs::OpenOptions::new().read(true).open("/dev/rtc0").map_err(|e| e.to_string())?;
        let ret = unsafe { libc::ioctl(file.as_raw_fd(), RTC_WKALM_SET as _, alarm as *const RtcWkalrm) };
        if ret < 0 {
            Err(std::io::Error::last_os_error().to_string())
        } else {
            Ok(())
        }
    }
}

#[cfg(target_os = "linux")]
pub use rtc::{clear_alarm, set_alarm};

#[cfg(not(target_os = "linux"))]
pub fn set_alarm(_seconds: u64) -> Result<(), String> {
    Err("not a Kobo".into())
}

#[cfg(not(target_os = "linux"))]
pub fn clear_alarm() {}

/// Flush the log to the FAT partition, so it survives a forced restart.
pub fn sync() {
    #[cfg(unix)]
    unsafe {
        libc::sync();
    }
}

/// What woke the device, from the kernel log.
pub fn log_wakeup_reason() -> Option<String> {
    let out = std::process::Command::new("dmesg").output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = text.lines().collect();
    let tail = &lines[lines.len().saturating_sub(150)..];
    for line in tail
        .iter()
        .filter(|l| l.contains("wake") || l.contains("Wake") || l.contains("PM: suspend") || l.contains("pwrkey") || l.contains("rtc"))
    {
        tracing::info!("  dmesg: {}", line);
    }
    // From the whole log: Wi-Fi and display drivers can push the line out of the tail.
    wake_source(&lines)
}

/// What the MediaTek sleep manager says woke the last suspend ("wake up by
/// R12_EINT_EVENT_B, timeout = ..."), if it says.
fn wake_source(lines: &[&str]) -> Option<String> {
    let line = lines.iter().rev().take_while(|l| !l.contains("PM: suspend entry")).find(|l| l.contains("wake up by "))?;
    let source = line.split("wake up by ").nth(1)?.split(',').next()?.trim();
    (!source.is_empty() && source != "(null)").then(|| source.to_string())
}

/// Bluetooth is switched on in Nickel: an `hci` device exists (it is gone
/// when Bluetooth is off), and its rfkill switch, if any, isn't blocking it.
/// On firmware 6.0 a suspend attempt with Bluetooth on crashes the
/// Wi-Fi/Bluetooth chip, so sleep mode must not even try.
pub fn bluetooth_on() -> bool {
    bluetooth_on_in(Path::new("/sys/class/bluetooth"))
}

fn bluetooth_on_in(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else { return false };
    entries.flatten().any(|e| {
        let name = e.file_name().to_string_lossy().into_owned();
        if !name.starts_with("hci") || name.contains(':') {
            return false;
        }
        // hci0/rfkillN/state: 1 = on, 0 = blocked
        let rfkill_state = std::fs::read_dir(e.path())
            .into_iter()
            .flatten()
            .flatten()
            .find(|r| r.file_name().to_string_lossy().starts_with("rfkill"))
            .and_then(|r| read_trim(&r.path().join("state")));
        rfkill_state.as_deref() != Some("0")
    })
}

/// Did the last suspend fail because Bluetooth is on? On firmware 6.0 the
/// Bluetooth interface refuses ("PM: Device 1100c000.btif failed to suspend"),
/// and trying again soon after crashed the Wi-Fi/Bluetooth chip's firmware,
/// leaving Wi-Fi dead until Nickel reset it.
pub fn suspend_blocked_by_bluetooth() -> bool {
    std::process::Command::new("dmesg")
        .output()
        .map(|out| blocked_by_bluetooth(&String::from_utf8_lossy(&out.stdout)))
        .unwrap_or(false)
}

fn blocked_by_bluetooth(dmesg: &str) -> bool {
    let lines: Vec<&str> = dmesg.lines().collect();
    let last = lines.iter().rposition(|l| l.contains("PM: suspend entry")).unwrap_or(0);
    lines[last..].iter().any(|l| l.contains("btif") && l.contains("failed to suspend"))
}

/// The power button and the timer both come through the PMIC interrupt
/// (EINT). Anything else, like the Wi-Fi/Bluetooth chip (CONN2AP, which
/// keeps waking the Kobo while Bluetooth is on), is not the user.
pub fn is_power_button_source(source: &str) -> bool {
    source.contains("EINT")
}

/// Kernel messages around a failed suspend, into our log: the kernel names
/// the driver that refused ("PM: Device ... failed to suspend").
pub fn log_suspend_diagnostics() {
    if let Ok(out) = std::process::Command::new("dmesg").output() {
        let text = String::from_utf8_lossy(&out.stdout);
        let lines: Vec<&str> = text.lines().collect();
        for line in &lines[lines.len().saturating_sub(40)..] {
            tracing::info!("  dmesg: {}", line);
        }
    }
    for file in ["/sys/power/state", "/sys/power/wakeup_count", "/sys/power/wake_lock", "/sys/power/state-extended"] {
        if let Some(v) = read_trim(Path::new(file)) {
            tracing::info!("  {}: {}", file, v);
        }
    }
    // Active wakeup sources block suspend; debugfs is not always mounted.
    for file in ["/sys/kernel/debug/wakeup_sources", "/d/wakeup_sources"] {
        if let Ok(text) = std::fs::read_to_string(file) {
            for line in text.lines().skip(1) {
                let cols: Vec<&str> = line.split_whitespace().collect();
                // name active_count event_count wakeup_count expire_count active_since ...
                if cols.len() > 5 && cols[5] != "0" {
                    tracing::info!("  active wakeup source: {}", line);
                }
            }
            break;
        }
    }
}

/// Suspend to RAM. Blocks until the device wakes up (timer or power button).
pub fn suspend() -> Result<(), String> {
    let extended = Path::new("/sys/power/state-extended");
    let has_extended = extended.exists();
    if has_extended {
        std::fs::write(extended, "1").map_err(|e| format!("state-extended: {}", e))?;
        // Nickel and KOReader both wait between the two writes.
        std::thread::sleep(Duration::from_secs(2));
    }
    // The caller checked, but a cable can go in during the wait above.
    let result = if is_plugged() {
        Err("plugged in, not suspending".to_string())
    } else {
        tracing::info!("Suspending…");
        // Flush the log (and anything else) to the FAT partition before sleeping.
        sync();
        std::fs::write("/sys/power/state", "mem").map_err(|e| format!("suspend refused: {}", e))
    };
    if has_extended {
        let _ = std::fs::write(extended, "0");
        std::thread::sleep(Duration::from_millis(100));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wifi_interface_comes_from_nickel() {
        // eth0 on the Glo and Aura (issue #1), wlan0 on newer models
        assert_eq!(operstate_path(Some("eth0")), Path::new("/sys/class/net/eth0/operstate"));
        assert_eq!(operstate_path(None), Path::new("/sys/class/net/wlan0/operstate"));
        assert_eq!(operstate_path(Some("")), Path::new("/sys/class/net/wlan0/operstate"));
    }

    #[test]
    fn bluetooth_is_on_while_an_hci_device_exists() {
        let dir = std::env::temp_dir().join(format!("kobo-ha-bt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // Off on the Clara Colour (firmware 6.0): no hci0 at all
        assert!(!bluetooth_on_in(&dir));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!bluetooth_on_in(&dir));
        // On: hci0 with rfkill1/state = 1
        std::fs::create_dir_all(dir.join("hci0/rfkill1")).unwrap();
        std::fs::write(dir.join("hci0/rfkill1/state"), "1\n").unwrap();
        assert!(bluetooth_on_in(&dir));
        // Blocked by rfkill
        std::fs::write(dir.join("hci0/rfkill1/state"), "0\n").unwrap();
        assert!(!bluetooth_on_in(&dir));
        // Without rfkill information: the device is enough
        std::fs::remove_dir_all(dir.join("hci0/rfkill1")).unwrap();
        assert!(bluetooth_on_in(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bluetooth_blocking_suspend_is_recognised() {
        let refused = "PM: suspend entry 2026-11-11 21:32:28 UTC\n\
            dpm_run_callback(): platform_pm_suspend+0x0/0x50 returns -1\n\
            PM: Device 1100c000.btif failed to suspend: error -1\n\
            PM: Some devices failed to suspend, or early wake event detected\n";
        assert!(blocked_by_bluetooth(refused));
        // An older refusal followed by a good suspend doesn't count
        assert!(!blocked_by_bluetooth(&format!("{}PM: suspend entry 2026-11-11 21:40:00 UTC\nPM: suspend exit\n", refused)));
        assert!(!blocked_by_bluetooth("PM: suspend entry\nPM: Device 11230000.mmc failed to suspend: error -16\n"));
    }

    #[test]
    fn wake_source_comes_from_the_last_suspend() {
        let log = [
            "PM: suspend entry 2026-11-11 16:00:00 UTC",
            "-(0)[1:x]wake up by R12_EINT_EVENT_B, timeout = 0, r13 = 0x6060200",
            "PM: suspend exit 2026-11-11 16:05:00 UTC",
            "PM: suspend entry 2026-11-11 16:31:34 UTC",
            "-(0)[560:tokio-rt-worker]wake up by R12_CONN2AP_WAKEUP_B, timeout = 0, r13 = 0x8606e210",
            "PM: Finishing wakeup.",
        ];
        assert_eq!(wake_source(&log).as_deref(), Some("R12_CONN2AP_WAKEUP_B"));
        assert!(!is_power_button_source("R12_CONN2AP_WAKEUP_B"));
        assert_eq!(wake_source(&log[..3]).as_deref(), Some("R12_EINT_EVENT_B"));
        assert!(is_power_button_source("R12_EINT_EVENT_B"));
        // Nothing reported for the last suspend, or "(null)": unknown
        assert_eq!(wake_source(&log[..4]), None);
        assert_eq!(wake_source(&["PM: suspend entry", "wake up by (null), timeout = 2990"]), None);
    }

    #[test]
    fn critical_battery_needs_a_low_reading_and_no_charger() {
        assert!(is_critical(Some(3), false, 3));
        assert!(is_critical(Some(0), false, 3));
        assert!(!is_critical(Some(4), false, 3));
        assert!(!is_critical(Some(0), true, 3), "charging");
        assert!(!is_critical(None, false, 3), "unreadable, as on the Nia's log (issue #4)");
    }

    #[test]
    fn battery_empty_ends_with_a_charger_or_a_clearly_higher_level() {
        assert!(is_recovered(Some(0), true, 3));
        assert!(!is_recovered(Some(3), false, 3));
        assert!(!is_recovered(Some(5), false, 3), "gauge jitter must not wake it");
        assert!(is_recovered(Some(6), false, 3));
        assert!(is_recovered(None, false, 3), "never stuck on a level it can't read");
    }

    #[cfg(unix)]
    #[test]
    fn wifi_busy_while_wifi_sh_runs() {
        let _one_at_a_time = WIFI_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        assert!(!wifi_busy());
        let runner = std::thread::spawn(|| wifi(Path::new("/nonexistent/wifi.sh"), true));
        assert!(!runner.join().unwrap(), "a missing script fails");
        assert!(!wifi_busy(), "done, also after a failure");
        let _run = WifiRun::start();
        assert!(wifi_busy());
    }

    /// Tests that look at the global count of wifi.sh runs.
    static WIFI_TESTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[cfg(unix)]
    #[test]
    fn a_wifi_switch_still_running_is_stopped_on_exit() {
        let _one_at_a_time = WIFI_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("kobo-ha-stop-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // "on" with no known network: waits a long time, child included
        let script = dir.join("wifi.sh");
        std::fs::write(&script, "sleep 60 &\nsleep 60\n").unwrap();
        let runner = std::thread::spawn({
            let script = script.clone();
            move || wifi(&script, true)
        });
        let until = Instant::now() + Duration::from_secs(5);
        while WIFI_GROUPS.lock().unwrap().is_empty() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(wifi_busy());
        let start = Instant::now();
        stop_wifi(Duration::from_secs(5));
        assert!(!wifi_busy(), "still running after stop_wifi");
        assert!(start.elapsed() < Duration::from_secs(3), "{:?}", start.elapsed());
        assert!(!runner.join().unwrap(), "a stopped switch is not a success");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    fn sh(script: &str) -> Command {
        let mut command = Command::new("/bin/sh");
        command.arg("-c").arg(script);
        command
    }

    #[cfg(unix)]
    #[test]
    fn stuck_commands_are_killed_at_the_timeout() {
        let start = Instant::now();
        let out = run_with_timeout(sh("echo started; sleep 30; echo never"), Duration::from_millis(300), || {}).unwrap();
        assert!(out.status.is_none());
        assert_eq!(out.lines, ["started"]);
        assert!(start.elapsed() < Duration::from_secs(5), "{:?}", start.elapsed());
    }

    #[cfg(unix)]
    #[test]
    fn a_daemon_holding_the_pipes_does_not_block() {
        // wpa_supplicant -B and dhcpcd stay behind after wifi.sh on
        let start = Instant::now();
        let out = run_with_timeout(sh("sleep 5 & echo done"), Duration::from_secs(20), || {}).unwrap();
        assert!(out.status.unwrap().success());
        assert_eq!(out.lines, ["done"]);
        assert!(start.elapsed() < Duration::from_secs(3), "{:?}", start.elapsed());
    }

    /// Run deploy/wifi.sh with stub commands that log their arguments; returns
    /// the log and the run directory (marker, lock).
    #[cfg(unix)]
    fn run_wifi_sh(name: &str, arg: &str, module: &str) -> (String, PathBuf) {
        run_wifi_sh_with(name, arg, module, &[])
    }

    /// Same, with extra stubs: (command, script body).
    #[cfg(unix)]
    fn run_wifi_sh_with(name: &str, arg: &str, module: &str, stubs: &[(&str, &str)]) -> (String, PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("kobo-ha-wifi-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let log = dir.join("calls.log");
        for cmd in ["wpa_cli", "ifconfig", "wlarm_le", "killall", "rmmod", "insmod", "usleep"] {
            let stub = bin.join(cmd);
            std::fs::write(&stub, format!("#!/bin/sh\necho \"{} $*\" >> \"{}\"\n", cmd, log.display())).unwrap();
            std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        // No wpa_supplicant running
        let pidof = bin.join("pidof");
        std::fs::write(&pidof, "#!/bin/sh\nexit 1\n").unwrap();
        std::fs::set_permissions(&pidof, std::fs::Permissions::from_mode(0o755)).unwrap();
        for (cmd, body) in stubs {
            let stub = bin.join(cmd);
            std::fs::write(&stub, format!("#!/bin/sh\n{}\n", body)).unwrap();
            std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("deploy/wifi.sh");
        let mut command = Command::new("/bin/sh");
        command
            .arg(&script)
            .arg(arg)
            .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
            .env("KOBO_HA_RUN_DIR", &dir)
            .env("WIFI_MODULE", module)
            .env("INTERFACE", "eth0")
            .env("PLATFORM", "test")
            .env("KOBO_HA_ONBOARD", dir.join("onboard"));
        let out = run_with_timeout(command, Duration::from_secs(20), || {}).unwrap();
        let status = out.status.unwrap_or_else(|| panic!("wifi.sh {} hung: {:?}", arg, out.lines));
        assert!(status.success(), "wifi.sh {} failed: {:?}", arg, out.lines);
        (std::fs::read_to_string(&log).unwrap_or_default(), dir)
    }

    #[cfg(unix)]
    #[test]
    fn wifi_sh_uses_nickels_networks_from_the_user_partition_on_firmware_6() {
        let conf = |name: &str, onboard: Option<&str>| {
            let dir = std::env::temp_dir().join(format!("kobo-ha-wifi-{}-{}", name, std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            if let Some(text) = onboard {
                std::fs::create_dir_all(dir.join("onboard/.kobo")).unwrap();
                std::fs::write(dir.join("onboard/.kobo/wpa_supplicant.conf"), text).unwrap();
            }
            let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("deploy/wifi.sh");
            let out = Command::new("/bin/sh")
                .arg(&script)
                .arg("conf")
                .env("KOBO_HA_RUN_DIR", &dir)
                .env("KOBO_HA_ONBOARD", dir.join("onboard"))
                .env("WIFI_MODULE", "wlan_drv_gen4m")
                .output()
                .unwrap();
            let _ = std::fs::remove_dir_all(&dir);
            String::from_utf8_lossy(&out.stdout).lines().last().unwrap_or_default().to_string()
        };
        // Firmware 6.0: Nickel keeps them on the user partition
        assert!(conf("conf6", Some("ctrl_interface=/var/run/wpa_supplicant\nnetwork={\n}\n")).ends_with("onboard/.kobo/wpa_supplicant.conf"));
        // Firmware 4.x: in /etc; an empty file on the user partition doesn't count
        assert_eq!(conf("conf4", None), "/etc/wpa_supplicant/wpa_supplicant.conf");
        assert_eq!(conf("conf4empty", Some("ctrl_interface=/var/run/wpa_supplicant\n")), "/etc/wpa_supplicant/wpa_supplicant.conf");
    }

    #[cfg(unix)]
    #[test]
    fn wifi_sh_does_not_spin_when_the_lock_cant_be_created() {
        // A stale-lock round used to skip the counter and the wait: with mkdir
        // failing every time the script span forever at full CPU.
        let (calls, dir) = run_wifi_sh_with("nolock", "off", "wlan_drv_gen4m", &[("mkdir", "exit 1")]);
        assert_eq!(calls.matches("usleep 500000").count(), 119, "{}", calls);
        assert!(calls.contains("wpa_cli -i eth0 terminate"), "{}", calls);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn wifi_off_goes_on_when_dhcpcd_hangs() {
        let (calls, dir) = run_wifi_sh_with("dhcpcd", "off", "wlan_drv_gen4m", &[("dhcpcd", "exec sleep 30")]);
        assert!(calls.contains("usleep 10000000"), "no guard for dhcpcd -k:\n{}", calls);
        assert!(calls.contains("wpa_cli -i eth0 terminate"), "{}", calls);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn wifi_off_leaves_the_marker_for_launch_sh() {
        // launch.sh only turns Wi-Fi back on when this marker exists: with Wi-Fi
        // already off in Nickel, loading the driver froze the Glo (issue #1).
        let (calls, dir) = run_wifi_sh("marker", "off", "8189fs");
        assert!(dir.join("kobo-ha-wifi-off").exists(), "marker missing; calls:\n{}", calls);
        assert!(dir.join("kobo-ha-wifi-used").exists(), "used flag missing");
        assert!(!dir.join("kobo-ha-wifi.lock").exists(), "lock left behind");
        assert!(calls.contains("wpa_cli -i eth0 terminate"), "{}", calls);
        assert!(!calls.contains("wlarm_le"), "wlarm_le is for Broadcom only:\n{}", calls);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn launch_sh_hands_wifi_back_off_only_after_the_dashboard_used_it() {
        let launch = include_str!("../deploy/launch.sh");
        let wifi = include_str!("../deploy/wifi.sh");
        assert!(launch.contains(r#"WIFI_USED="/tmp/kobo-ha-wifi-used""#));
        assert!(wifi.contains(r#"USED="$RUN_DIR/kobo-ha-wifi-used""#));
        // Nickel brings Wi-Fi up itself; launch.sh never does.
        assert!(!launch.contains("wifi.sh\" on"), "launch.sh must not switch Wi-Fi on");
        let off = launch.lines().find(|l| l.contains("wifi.sh\" off")).expect("launch.sh runs wifi.sh off");
        let pos = launch.find(off).unwrap();
        let condition = launch[..pos].lines().last().unwrap();
        assert!(condition.contains(r#"[ -e "$WIFI_USED" ]"#), "wifi.sh off without the used check: {}", condition);
    }

    #[cfg(unix)]
    #[test]
    fn wifi_off_switches_broadcom_off_with_wlarm_le() {
        // Broadcom (dhd) chips: wlarm_le down before the interface goes down (issue #2)
        let (calls, dir) = run_wifi_sh("dhd", "off", "dhd");
        let wl = calls.find("wlarm_le -i eth0 down").unwrap_or_else(|| panic!("no wlarm_le down:\n{}", calls));
        let down = calls.find("ifconfig eth0 down").unwrap_or_else(|| panic!("no ifconfig down:\n{}", calls));
        assert!(wl < down, "{}", calls);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
