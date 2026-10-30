//! Known Kobo (and Kobo-based Tolino) e-readers.
//!
//! `model_id` is the last three digits of the product UUID at the end of
//! `.kobo/version` (as used by calibre's Kobo driver). Devices without one can
//! still be picked by hand.

use serde::Serialize;

#[derive(Debug, Clone, Copy, Serialize)]
pub struct KoboModel {
    pub key: &'static str,
    pub name: &'static str,
    pub model_id: Option<&'static str>,
    /// Portrait resolution
    pub width: u32,
    pub height: u32,
    pub color: bool,
    /// Confirmed working with this dashboard
    pub tested: bool,
}

const fn m(key: &'static str, name: &'static str, model_id: Option<&'static str>, width: u32, height: u32, color: bool, tested: bool) -> KoboModel {
    KoboModel { key, name, model_id, width, height, color, tested }
}

pub const MODELS: &[KoboModel] = &[
    m("clara_colour", "Kobo Clara Colour", Some("393"), 1072, 1448, true, true),
    m("clara_bw", "Kobo Clara BW", Some("391"), 1072, 1448, false, false),
    m("libra_colour", "Kobo Libra Colour", Some("390"), 1264, 1680, true, false),
    m("elipsa_2e", "Kobo Elipsa 2E", None, 1404, 1872, false, false),
    m("clara_2e", "Kobo Clara 2E", None, 1072, 1448, false, false),
    m("libra_2", "Kobo Libra 2", None, 1264, 1680, false, false),
    m("sage", "Kobo Sage", None, 1440, 1920, false, false),
    m("elipsa", "Kobo Elipsa", None, 1404, 1872, false, false),
    m("clara_hd", "Kobo Clara HD", None, 1072, 1448, false, false),
    m("libra_h2o", "Kobo Libra H2O", None, 1264, 1680, false, false),
    m("forma", "Kobo Forma", None, 1440, 1920, false, false),
    m("tolino_shine_color", "Tolino Shine Color", Some("693"), 1072, 1448, true, false),
    m("tolino_vision_color", "Tolino Vision Color", Some("690"), 1264, 1680, true, false),
    m("tolino_shine_5", "Tolino Shine 5", Some("691"), 1072, 1448, false, false),
    // Older Kobos (issue #1); their product ids are the ones FBInk/KOReader use
    m("glo", "Kobo Glo", Some("330"), 758, 1024, false, true),
    m("aura", "Kobo Aura", Some("360"), 758, 1024, false, true),
    m("aura_2", "Kobo Aura (2nd edition)", Some("375"), 758, 1024, false, false),
    m("glo_hd", "Kobo Glo HD", Some("371"), 1072, 1448, false, false),
    m("aura_hd", "Kobo Aura HD", Some("350"), 1080, 1440, false, false),
    m("aura_h2o", "Kobo Aura H2O", Some("370"), 1080, 1430, false, false),
    m("touch", "Kobo Touch", Some("310"), 600, 800, false, false),
    m("mini", "Kobo Mini", Some("340"), 600, 800, false, false),
];

pub fn by_key(key: &str) -> Option<&'static KoboModel> {
    MODELS.iter().find(|m| m.key == key)
}

/// Match the product UUID (or its last digits) from `.kobo/version`.
pub fn by_model_id(id: &str) -> Option<&'static KoboModel> {
    let tail = id.trim().get(id.trim().len().saturating_sub(3)..)?;
    MODELS.iter().find(|m| m.model_id == Some(tail))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookups() {
        assert_eq!(by_model_id("00000000-0000-0000-0000-000000000393").map(|m| m.key), Some("clara_colour"));
        assert_eq!(by_model_id("390").map(|m| m.width), Some(1264));
        assert!(by_model_id("00000000-0000-0000-0000-000000000376").is_none());
        assert!(by_key("sage").is_some());
        // Glo (issue #1): previews at its real size instead of the Clara's
        assert_eq!(by_model_id("00000000-0000-0000-0000-000000000330").map(|m| (m.key, m.width, m.height)), Some(("glo", 758, 1024)));
    }
}
