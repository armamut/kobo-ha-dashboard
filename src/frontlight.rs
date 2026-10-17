//! Kobo frontlight control through sysfs.
//!
//! Natural-light Kobos (Clara 2E/BW/Colour, Libra 2/Colour, Sage, Elipsa 2E…)
//! take a 0-100 brightness in `mxc_msp430.0/brightness` (see KOReader's
//! device definitions); the warmth mixer lives elsewhere and is left alone.
//! Other models fall back to a single-channel backlight if one exists.
//!
//! The starting level comes from Nickel's own settings when possible (as
//! KOReader does), because reading the sysfs value back is not reliable on
//! every model; sysfs is only the fallback.

use std::path::{Path, PathBuf};

const NICKEL_CONF: &str = "/mnt/onboard/.kobo/Kobo/Kobo eReader.conf";

/// `FrontLightLevel` (0-100) and `FrontLightState` from Nickel's settings.
fn nickel_light(conf: &Path) -> Option<(u32, Option<bool>)> {
    let text = std::fs::read_to_string(conf).ok()?;
    let mut in_power = false;
    let (mut level, mut state) = (None, None);
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_power = line == "[PowerOptions]";
            continue;
        }
        if !in_power {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            match key.trim() {
                "FrontLightLevel" => level = value.trim().parse::<u32>().ok(),
                "FrontLightState" => state = Some(value.trim().eq_ignore_ascii_case("true")),
                _ => {}
            }
        }
    }
    level.map(|l| (l.min(100), state))
}

pub struct Frontlight {
    brightness: PathBuf,
    max: u32,
    /// Level when the dashboard started, restored on exit
    original: u32,
    /// Level to return to when the light is switched back on
    level: u32,
    on: bool,
}

fn read_u32(path: &Path) -> Option<u32> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

impl Frontlight {
    pub fn detect() -> Option<Self> {
        Self::detect_in(Path::new("/sys/class/backlight"), Path::new(NICKEL_CONF))
    }

    fn detect_in(base: &Path, nickel_conf: &Path) -> Option<Self> {
        let mut candidates = vec![base.join("mxc_msp430.0")];
        if let Ok(rd) = std::fs::read_dir(base) {
            let mut others: Vec<PathBuf> = rd
                .flatten()
                .map(|e| e.path())
                // lm3630a_* / tlc5947_* are colour channels or mixers, not a plain level.
                .filter(|p| {
                    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    name != "mxc_msp430.0" && !name.starts_with("lm3630a") && !name.starts_with("tlc5947")
                })
                .collect();
            others.sort();
            candidates.extend(others);
        }

        for dir in candidates {
            let brightness = dir.join("brightness");
            // The file only has to exist (and be writable); reading it back may not work.
            if !brightness.exists() {
                continue;
            }
            let max = read_u32(&dir.join("max_brightness")).filter(|m| *m > 0).unwrap_or(100);
            let sysfs = read_u32(&brightness);
            let nickel = nickel_light(nickel_conf);

            // (current level, level to use when switching on)
            let (current, remembered) = match nickel {
                Some((level, state)) => {
                    let scaled = level * max / 100;
                    let on = state.unwrap_or(level > 0) && scaled > 0;
                    (if on { scaled } else { 0 }, scaled)
                }
                None => {
                    let v = sysfs.unwrap_or(0);
                    (v, v)
                }
            };
            tracing::info!(
                "Frontlight at {:?}: max {}, sysfs reads {:?}, Nickel settings {:?} -> currently {}",
                brightness,
                max,
                sysfs,
                nickel,
                current
            );
            return Some(Self {
                brightness,
                max,
                original: current,
                // With no known level, switching the light on uses 30%.
                level: if remembered > 0 { remembered } else { (max * 3 / 10).max(1) },
                on: current > 0,
            });
        }
        tracing::info!("No controllable frontlight found");
        None
    }

    fn write(&self, value: u32) {
        if let Err(e) = std::fs::write(&self.brightness, value.min(self.max).to_string()) {
            tracing::warn!("Could not set frontlight: {}", e);
        }
    }

    pub fn is_on(&self) -> bool {
        self.on
    }

    pub fn set_on(&mut self, on: bool) {
        if on == self.on {
            return;
        }
        if !on {
            // Remember a level the user may have changed from elsewhere.
            if let Some(v) = read_u32(&self.brightness).filter(|v| *v > 0) {
                self.level = v;
            }
        }
        self.write(if on { self.level } else { 0 });
        self.on = on;
    }

    /// Put the light back the way the Kobo had it.
    pub fn restore(&mut self) {
        self.write(self.original);
        self.on = self.original > 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggles_remembers_and_restores() {
        let base = std::env::temp_dir().join(format!("kobo-ha-light-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let dir = base.join("mxc_msp430.0");
        std::fs::create_dir_all(&dir).unwrap();
        // A colour mixer next to it must not be picked up
        std::fs::create_dir_all(base.join("lm3630a_led")).unwrap();
        std::fs::write(base.join("lm3630a_led/brightness"), "5").unwrap();
        std::fs::write(dir.join("brightness"), "40").unwrap();
        std::fs::write(dir.join("max_brightness"), "100").unwrap();
        let level = || std::fs::read_to_string(dir.join("brightness")).unwrap().trim().to_string();

        let mut light = Frontlight::detect_in(&base, &base.join("no-such.conf")).expect("light found");
        assert!(light.is_on());
        light.set_on(false);
        assert_eq!(level(), "0");
        light.set_on(true);
        assert_eq!(level(), "40");

        // Changed elsewhere while on: switching off and on keeps the new level
        std::fs::write(dir.join("brightness"), "70").unwrap();
        light.set_on(false);
        light.set_on(true);
        assert_eq!(level(), "70");

        light.restore();
        assert_eq!(level(), "40");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn light_off_at_start_turns_on_at_30_percent() {
        let base = std::env::temp_dir().join(format!("kobo-ha-light-off-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let dir = base.join("mxc_msp430.0");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("brightness"), "0").unwrap();
        let mut light = Frontlight::detect_in(&base, &base.join("no-such.conf")).unwrap();
        assert!(!light.is_on());
        light.set_on(true);
        assert_eq!(std::fs::read_to_string(dir.join("brightness")).unwrap().trim(), "30");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn nickel_settings_win_over_sysfs() {
        let base = std::env::temp_dir().join(format!("kobo-ha-light-nickel-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let dir = base.join("mxc_msp430.0");
        std::fs::create_dir_all(&dir).unwrap();
        // Some models read back 0 even with the light on
        std::fs::write(dir.join("brightness"), "0").unwrap();
        let conf = base.join("Kobo eReader.conf");
        std::fs::write(&conf, "[Reading]\nFrontLightLevel=5\n\n[PowerOptions]\nFrontLightLevel=42\nFrontLightState=true\n").unwrap();

        let mut light = Frontlight::detect_in(&base, &conf).unwrap();
        assert!(light.is_on(), "light is on according to Nickel");
        light.set_on(false);
        assert_eq!(std::fs::read_to_string(dir.join("brightness")).unwrap().trim(), "0");
        light.set_on(true);
        assert_eq!(std::fs::read_to_string(dir.join("brightness")).unwrap().trim(), "42");

        // Light toggled off in Nickel: starts off, switching on uses the saved level
        std::fs::write(&conf, "[PowerOptions]\nFrontLightLevel=42\nFrontLightState=false\n").unwrap();
        let mut light = Frontlight::detect_in(&base, &conf).unwrap();
        assert!(!light.is_on());
        light.set_on(true);
        assert_eq!(std::fs::read_to_string(dir.join("brightness")).unwrap().trim(), "42");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn unreadable_brightness_file_still_counts() {
        let base = std::env::temp_dir().join(format!("kobo-ha-light-wo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let dir = base.join("mxc_msp430.0");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("brightness"), "not a number").unwrap();
        let conf = base.join("Kobo eReader.conf");
        std::fs::write(&conf, "[PowerOptions]\nFrontLightLevel=20\n").unwrap();
        let light = Frontlight::detect_in(&base, &conf).expect("found despite unreadable value");
        assert!(light.is_on());
        let _ = std::fs::remove_dir_all(&base);
    }
}
