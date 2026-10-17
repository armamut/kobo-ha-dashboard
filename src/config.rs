use serde::{Deserialize, Serialize};
use crate::display::Rotation;
use crate::models;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    /// Config format version, for future migrations
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub device: DeviceConfig,
    #[serde(default)]
    pub home_assistant: HomeAssistantConfig,
    #[serde(default)]
    pub theme: ThemeConfig,
    #[serde(default)]
    pub pages: Vec<PageConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceConfig {
    /// Kobo model key (see `models.rs`) or "auto". On the device the real
    /// resolution is read from the framebuffer; the model drives the preview.
    #[serde(default = "default_auto")]
    pub model: String,

    /// Screen orientation: "portrait" or "landscape"
    #[serde(default = "default_orientation")]
    pub orientation: String,

    /// Custom width override (0 for default based on orientation)
    #[serde(default)]
    pub width: u32,

    /// Custom height override (0 for default based on orientation)
    #[serde(default)]
    pub height: u32,

    /// Touch input device path, or "auto" to detect it
    #[serde(default = "default_touch_device")]
    pub touch_device: String,

    /// Framebuffer device path
    #[serde(default = "default_fb_device")]
    pub fb_device: String,

    /// Full screen e-ink flash refresh interval (in number of updates, 0 = never)
    #[serde(default = "default_full_refresh_cycles")]
    pub full_refresh_cycles: u32,

    /// Swap raw touch X and Y (true on almost every Kobo)
    #[serde(default = "default_true")]
    pub swap_touch_xy: bool,

    /// Mirror touch X after swapping (true on almost every Kobo)
    #[serde(default = "default_true")]
    pub invert_touch_x: bool,

    /// Mirror touch Y after swapping
    #[serde(default)]
    pub invert_touch_y: bool,

    /// Rotation (degrees, clockwise) from the portrait screen to framebuffer
    /// memory. "auto" works out of the box on Kobo; only set 0/90/180/270 if
    /// the picture comes out rotated or upside down.
    #[serde(default = "default_auto")]
    pub fb_rotation: String,

    /// Which way the device is turned in landscape mode: 90 or 270
    #[serde(default = "default_landscape_rotation")]
    pub landscape_rotation: i32,

    /// Use colour waveforms (Kaleido panels such as the Clara Colour)
    #[serde(default = "default_true")]
    pub color_panel: bool,

    /// Switch the frontlight off after this many seconds without a tap
    /// (0 = never). A tap turns it back on.
    #[serde(default)]
    pub frontlight_timeout_secs: u32,
}

fn default_orientation() -> String {
    "portrait".to_string()
}
fn default_touch_device() -> String {
    "auto".to_string()
}
fn default_fb_device() -> String {
    "/dev/fb0".to_string()
}
fn default_full_refresh_cycles() -> u32 {
    10
}
fn default_auto() -> String {
    "auto".to_string()
}
fn default_landscape_rotation() -> i32 {
    90
}

impl Default for DeviceConfig {
    fn default() -> Self {
        Self {
            model: default_auto(),
            orientation: default_orientation(),
            width: 0,
            height: 0,
            touch_device: default_touch_device(),
            fb_device: default_fb_device(),
            full_refresh_cycles: default_full_refresh_cycles(),
            swap_touch_xy: true,
            invert_touch_x: true,
            invert_touch_y: false,
            fb_rotation: default_auto(),
            landscape_rotation: default_landscape_rotation(),
            color_panel: true,
            frontlight_timeout_secs: 0,
        }
    }
}

impl DeviceConfig {
    /// Canvas size for previews: explicit size, else the model's, else Clara.
    pub fn get_dimensions(&self) -> (u32, u32) {
        self.resolve_dimensions(None)
    }

    /// Canvas size; `probed` is the framebuffer resolution on a real device,
    /// which wins over the configured model.
    pub fn resolve_dimensions(&self, probed: Option<(u32, u32)>) -> (u32, u32) {
        if self.width > 0 && self.height > 0 {
            return (self.width, self.height);
        }
        let (pw, ph) = probed
            .map(|(a, b)| (a.min(b), a.max(b)))
            .or_else(|| models::by_key(&self.model).map(|m| (m.width, m.height)))
            .unwrap_or((1072, 1448));
        if self.is_landscape() {
            (ph, pw)
        } else {
            (pw, ph)
        }
    }

    pub fn is_landscape(&self) -> bool {
        self.orientation.to_lowercase() == "landscape"
    }

    /// Rotation from the app canvas to the physical portrait panel
    pub fn canvas_rotation(&self) -> Rotation {
        if self.is_landscape() {
            Rotation::from_degrees(self.landscape_rotation)
        } else {
            Rotation::R0
        }
    }

    pub fn fb_rotation(&self) -> Option<Rotation> {
        self.fb_rotation.trim().parse::<i32>().ok().map(Rotation::from_degrees)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HomeAssistantConfig {
    /// Home Assistant Base URL (e.g. "http://192.168.1.100:8123" or "http://homeassistant.local:8123")
    #[serde(default = "default_ha_url")]
    pub url: String,

    /// Long-Lived Access Token
    #[serde(default)]
    pub token: String,

    /// State polling interval in seconds
    #[serde(default = "default_refresh_secs")]
    pub refresh_interval_secs: u64,

    /// Request timeout in seconds
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,

    /// Enable demo/mock mode if HA is unreachable or token is empty
    #[serde(default = "default_demo_mode")]
    pub demo_mode_fallback: bool,
}

fn default_ha_url() -> String {
    "http://homeassistant.local:8123".to_string()
}
fn default_refresh_secs() -> u64 {
    20
}
fn default_timeout_secs() -> u64 {
    8
}
fn default_demo_mode() -> bool {
    true
}

impl Default for HomeAssistantConfig {
    fn default() -> Self {
        Self {
            url: default_ha_url(),
            token: String::new(),
            refresh_interval_secs: default_refresh_secs(),
            timeout_secs: default_timeout_secs(),
            demo_mode_fallback: default_demo_mode(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeConfig {
    /// Color mode: "kaleido" (rich e-ink colors), "paper_warm", "high_contrast_bw", "dark"
    #[serde(default = "default_color_mode")]
    pub mode: String,

    /// Primary accent color hex (e.g. "#F59E0B" for warm amber, "#3B82F6" for blue)
    #[serde(default = "default_accent_color")]
    pub accent_color: String,

    /// Rounded corner radius for cards (in pixels)
    #[serde(default = "default_card_corner_radius")]
    pub card_corner_radius: f32,

    /// Time format: "24h" or "12h"
    #[serde(default = "default_time_format")]
    pub time_format: String,

    /// Language for date header: "tr" or "en"
    #[serde(default = "default_language")]
    pub language: String,

    /// Fixed UTC offset for the clock, e.g. "+03:00". Kobo keeps its system
    /// clock in UTC (Nickel applies the timezone itself), so set this on device.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub utc_offset: Option<String>,
}

fn default_color_mode() -> String {
    "kaleido".to_string()
}
fn default_accent_color() -> String {
    "#D97706".to_string()
}
fn default_card_corner_radius() -> f32 {
    18.0
}
fn default_time_format() -> String {
    "24h".to_string()
}
fn default_language() -> String {
    "en".to_string()
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            mode: default_color_mode(),
            accent_color: default_accent_color(),
            card_corner_radius: default_card_corner_radius(),
            time_format: default_time_format(),
            language: default_language(),
            utc_offset: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageConfig {
    pub id: String,
    pub title: String,
    #[serde(default = "default_page_icon")]
    pub icon: String,
    #[serde(default)]
    pub cards: Vec<CardConfig>,
}

fn default_page_icon() -> String {
    "home".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum CardConfig {
    #[serde(rename = "weather")]
    Weather {
        entity_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default = "default_forecast_days")]
        forecast_days: usize,
    },
    #[serde(rename = "light")]
    Light {
        entity_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subtitle: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        icon: Option<String>,
        #[serde(default = "default_true")]
        show_brightness: bool,
    },
    #[serde(rename = "switch")]
    Switch {
        entity_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subtitle: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        icon: Option<String>,
    },
    #[serde(rename = "climate")]
    Climate {
        entity_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default = "default_temp_step")]
        step: f32,
    },
    #[serde(rename = "sensor")]
    Sensor {
        entity_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        unit: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        icon: Option<String>,
        #[serde(default)]
        show_graph: bool,
    },
    #[serde(rename = "sensor_grid")]
    SensorGrid {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        sensors: Vec<SensorItemConfig>,
    },
    #[serde(rename = "media_player")]
    MediaPlayer {
        entity_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
    },
    #[serde(rename = "scene")]
    Scene {
        entity_id: String,
        title: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        icon: Option<String>,
    },
    #[serde(rename = "scenes_row")]
    ScenesRow {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        scenes: Vec<SceneItemConfig>,
    },
}

fn default_forecast_days() -> usize {
    4
}
fn default_true() -> bool {
    true
}
fn default_temp_step() -> f32 {
    0.5
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SensorItemConfig {
    pub entity_id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneItemConfig {
    pub entity_id: String,
    pub title: String,
    #[serde(default = "default_scene_icon")]
    pub icon: String,
}

fn default_scene_icon() -> String {
    "play".to_string()
}

pub const CONFIG_VERSION: u32 = 1;

fn default_version() -> u32 {
    CONFIG_VERSION
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            device: DeviceConfig::default(),
            home_assistant: HomeAssistantConfig::default(),
            theme: ThemeConfig::default(),
            pages: default_demo_pages(),
        }
    }
}

pub fn default_demo_pages() -> Vec<PageConfig> {
    vec![
        PageConfig {
            id: "main".to_string(),
            title: "Home".to_string(),
            icon: "home".to_string(),
            cards: vec![
                CardConfig::Weather {
                    entity_id: "weather.home".to_string(),
                    title: Some("Weather".to_string()),
                    forecast_days: 4,
                },
                CardConfig::ScenesRow {
                    title: Some("Scenes".to_string()),
                    scenes: vec![
                        SceneItemConfig {
                            entity_id: "scene.good_night".to_string(),
                            title: "Good night".to_string(),
                            icon: "moon".to_string(),
                        },
                        SceneItemConfig {
                            entity_id: "scene.movie".to_string(),
                            title: "Movie".to_string(),
                            icon: "tv".to_string(),
                        },
                        SceneItemConfig {
                            entity_id: "scene.relax".to_string(),
                            title: "Relax".to_string(),
                            icon: "coffee".to_string(),
                        },
                        SceneItemConfig {
                            entity_id: "scene.all_off".to_string(),
                            title: "All off".to_string(),
                            icon: "power".to_string(),
                        },
                    ],
                },
                CardConfig::Light {
                    entity_id: "light.ceiling".to_string(),
                    title: Some("Ceiling Light".to_string()),
                    subtitle: Some("Living room".to_string()),
                    icon: Some("lightbulb".to_string()),
                    show_brightness: true,
                },
                CardConfig::Light {
                    entity_id: "light.dining_table".to_string(),
                    title: Some("Dining Table".to_string()),
                    subtitle: Some("Living room".to_string()),
                    icon: Some("lightbulb".to_string()),
                    show_brightness: true,
                },
                CardConfig::Climate {
                    entity_id: "climate.living_room".to_string(),
                    title: Some("Living room".to_string()),
                    step: 0.5,
                },
                CardConfig::SensorGrid {
                    title: Some("Climate".to_string()),
                    sensors: vec![
                        SensorItemConfig {
                            entity_id: "sensor.temperature".to_string(),
                            title: "Temperature".to_string(),
                            icon: Some("thermometer".to_string()),
                            unit: Some("°C".to_string()),
                        },
                        SensorItemConfig {
                            entity_id: "sensor.humidity".to_string(),
                            title: "Humidity".to_string(),
                            icon: Some("droplet".to_string()),
                            unit: Some("%".to_string()),
                        },
                        SensorItemConfig {
                            entity_id: "sensor.co2".to_string(),
                            title: "CO₂".to_string(),
                            icon: Some("wind".to_string()),
                            unit: Some("ppm".to_string()),
                        },
                        SensorItemConfig {
                            entity_id: "sensor.power".to_string(),
                            title: "Power".to_string(),
                            icon: Some("zap".to_string()),
                            unit: Some("W".to_string()),
                        },
                    ],
                },
                CardConfig::MediaPlayer {
                    entity_id: "media_player.speaker".to_string(),
                    title: Some("Speaker".to_string()),
                },
            ],
        },
        PageConfig {
            id: "climate".to_string(),
            title: "Climate".to_string(),
            icon: "thermometer".to_string(),
            cards: vec![
                CardConfig::Climate {
                    entity_id: "climate.lounge".to_string(),
                    title: Some("Living room".to_string()),
                    step: 0.5,
                },
                CardConfig::Climate {
                    entity_id: "climate.bedroom".to_string(),
                    title: Some("Bedroom".to_string()),
                    step: 0.5,
                },
                CardConfig::Sensor {
                    entity_id: "sensor.energy_today".to_string(),
                    title: Some("Energy today".to_string()),
                    unit: Some("kWh".to_string()),
                    icon: Some("zap".to_string()),
                    show_graph: true,
                },
                CardConfig::Switch {
                    entity_id: "switch.air_purifier".to_string(),
                    title: Some("Air Purifier".to_string()),
                    subtitle: Some("Auto mode".to_string()),
                    icon: Some("wind".to_string()),
                },
            ],
        },
    ]
}

impl AppConfig {
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self, Box<dyn std::error::Error>> {
        let content = std::fs::read_to_string(path)?;
        let config: AppConfig = serde_yaml::from_str(&content)?;
        Ok(config)
    }

    pub fn load_or_default<P: AsRef<Path>>(path: P) -> Self {
        match Self::load_from_file(path.as_ref()) {
            Ok(cfg) => cfg,
            Err(e) => {
                tracing::warn!(
                    "Config file {:?} could not be loaded ({}), using rich defaults",
                    path.as_ref(),
                    e
                );
                Self::default()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example_config_parses() {
        let cfg: AppConfig = serde_yaml::from_str(include_str!("../config.example.yaml")).expect("config.example.yaml parses");
        assert_eq!(cfg.version, CONFIG_VERSION);
        assert!(!cfg.pages.is_empty());
    }

    #[test]
    fn round_trip_omits_empty_options() {
        let yaml = serde_yaml::to_string(&AppConfig::default()).unwrap();
        assert!(!yaml.contains("null"), "None fields should be skipped:\n{}", yaml);
        let back: AppConfig = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(back.pages.len(), AppConfig::default().pages.len());
    }
}
