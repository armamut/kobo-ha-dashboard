//! Battery, Wi-Fi and suspend-to-RAM helpers for the Kobo.
//!
//! The suspend sequence follows what Nickel and KOReader do on Kobo:
//! flag `/sys/power/state-extended`, wait, sync, write `mem` to
//! `/sys/power/state` (returns after wakeup), then unflag. Two rules from
//! KOReader's notes are enforced by the caller: never suspend with Wi-Fi up
//! (it can crash the kernel), and never suspend while plugged in on MediaTek
//! devices (the kernel hangs).

use std::path::{Path, PathBuf};
use std::time::Duration;

fn battery_dir() -> Option<PathBuf> {
    std::fs::read_dir("/sys/class/power_supply")
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| std::fs::read_to_string(p.join("type")).map(|t| t.trim() == "Battery").unwrap_or(false))
}

/// `operstate` of the Wi-Fi interface (`INTERFACE` from Nickel: eth0 on
/// older Kobos such as the Glo, wlan0 on newer ones).
pub fn wifi_operstate_path() -> PathBuf {
    let iface = std::env::var("INTERFACE").ok().filter(|i| !i.is_empty()).unwrap_or_else(|| "wlan0".into());
    PathBuf::from(format!("/sys/class/net/{}/operstate", iface))
}

fn read_trim(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

/// One line in the log with charge level, charging state and current draw.
pub fn log_battery() {
    let Some(dir) = battery_dir() else { return };
    let capacity = read_trim(&dir.join("capacity")).unwrap_or_else(|| "?".into());
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

/// Run `wifi.sh on|off`; true on success.
pub fn wifi(script: &Path, on: bool) -> bool {
    let arg = if on { "on" } else { "off" };
    tracing::info!("Wi-Fi {}…", arg);
    match std::process::Command::new("/bin/sh").arg(script).arg(arg).output() {
        Ok(out) => {
            for line in String::from_utf8_lossy(&out.stdout).lines().chain(String::from_utf8_lossy(&out.stderr).lines()) {
                tracing::info!("  wifi.sh: {}", line);
            }
            let ok = out.status.success();
            if !ok {
                tracing::warn!("wifi.sh {} failed ({})", arg, out.status);
            }
            ok
        }
        Err(e) => {
            tracing::warn!("Could not run {}: {}", script.display(), e);
            false
        }
    }
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
pub fn log_wakeup_reason() {
    if let Ok(out) = std::process::Command::new("dmesg").output() {
        let text = String::from_utf8_lossy(&out.stdout);
        let lines: Vec<&str> = text.lines().collect();
        for line in lines[lines.len().saturating_sub(150)..]
            .iter()
            .filter(|l| l.contains("wake") || l.contains("Wake") || l.contains("PM: suspend") || l.contains("pwrkey") || l.contains("rtc"))
        {
            tracing::info!("  dmesg: {}", line);
        }
    }
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
    tracing::info!("Suspending…");
    // Flush the log (and anything else) to the FAT partition before sleeping.
    #[cfg(unix)]
    unsafe {
        libc::sync();
    }
    let result = std::fs::write("/sys/power/state", "mem").map_err(|e| format!("suspend refused: {}", e));
    if has_extended {
        let _ = std::fs::write(extended, "0");
        std::thread::sleep(Duration::from_millis(100));
    }
    result
}
