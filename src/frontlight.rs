//! Kobo frontlight control through sysfs.
//!
//! Natural-light Kobos (Clara 2E/BW/Colour, Libra 2/Colour, Sage, Elipsa 2E…)
//! take a 0-100 brightness in `mxc_msp430.0/brightness` (see KOReader's
//! device definitions); the warmth mixer lives elsewhere and is left alone.
//! Other models fall back to a single-channel backlight if one exists.

use std::path::{Path, PathBuf};

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
        Self::detect_in(Path::new("/sys/class/backlight"))
    }

    fn detect_in(base: &Path) -> Option<Self> {
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
            let Some(current) = read_u32(&brightness) else { continue };
            let max = read_u32(&dir.join("max_brightness")).filter(|m| *m > 0).unwrap_or(100);
            tracing::info!("Frontlight at {:?}: {}/{}", brightness, current, max);
            return Some(Self {
                brightness,
                max,
                original: current,
                // If the light was off when we started, switching it on uses 30%.
                level: if current > 0 { current } else { (max * 3 / 10).max(1) },
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

        let mut light = Frontlight::detect_in(&base).expect("light found");
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
        let mut light = Frontlight::detect_in(&base).unwrap();
        assert!(!light.is_on());
        light.set_on(true);
        assert_eq!(std::fs::read_to_string(dir.join("brightness")).unwrap().trim(), "30");
        let _ = std::fs::remove_dir_all(&base);
    }
}
