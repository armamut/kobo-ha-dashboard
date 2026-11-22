use crate::display::Rotation;
use crate::models;
use serde::{Deserialize, Serialize};
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
    #[serde(default)]
    pub updates: UpdatesConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdatesConfig {
    /// Look once a day for a new version on GitHub
    #[serde(default = "default_true")]
    pub check: bool,
}

impl Default for UpdatesConfig {
    fn default() -> Self {
        Self { check: true }
    }
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

    /// Sleep mode: suspend the Kobo after this many seconds without a tap
    /// (0 = never). The screen keeps showing the dashboard with a note.
    #[serde(default)]
    pub sleep_after_secs: u32,

    /// The header's keep-awake (eye) button: "auto" shows it where the top
    /// bar has room (not on 6" screens held upright), "show" or "hide".
    #[serde(default = "default_auto")]
    pub keep_awake_button: String,

    /// While asleep, wake up this often to refresh the screen.
    #[serde(default = "default_sleep_wake_secs")]
    pub sleep_wake_secs: u32,

    /// At or below this battery level (without a charger) show "battery
    /// empty" and sleep until the Kobo is charged (0 = never).
    #[serde(default = "default_battery_critical_percent")]
    pub battery_critical_percent: u32,
}

fn default_sleep_wake_secs() -> u32 {
    900
}
fn default_battery_critical_percent() -> u32 {
    3
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
            sleep_after_secs: 0,
            keep_awake_button: default_auto(),
            sleep_wake_secs: default_sleep_wake_secs(),
            battery_critical_percent: default_battery_critical_percent(),
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

    /// Touch axes (swap, invert x, invert y) for this device. The Libra 2
    /// (`PRODUCT=io`) has its panel's touch Y the other way round from other
    /// Kobos, so the generic defaults are replaced for it; a config that
    /// already changed them is kept as it is.
    pub fn touch_axes(&self, product: &str) -> (bool, bool, bool) {
        let configured = (self.swap_touch_xy, self.invert_touch_x, self.invert_touch_y);
        let generic = (true, true, false);
        let libra_2 = product == "io" || self.model == "libra_2";
        if libra_2 && configured == generic {
            (true, false, false)
        } else {
            configured
        }
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

    /// How often (seconds) the dashboard updates: the screen is redrawn with
    /// the latest states, and states are fetched when live updates are down.
    /// Between updates nothing is drawn, which saves battery. Default 5 minutes.
    #[serde(default = "default_update_secs")]
    pub update_interval_secs: u64,

    /// Former polling interval, replaced by `update_interval_secs`. Still read
    /// so older config files load, but no longer used or written.
    #[serde(default, skip_serializing)]
    pub refresh_interval_secs: Option<u64>,

    /// Request timeout in seconds
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,

    /// Enable demo/mock mode if HA is unreachable or token is empty
    #[serde(default = "default_demo_mode")]
    pub demo_mode_fallback: bool,

    /// Send the Kobo's battery level to Home Assistant as a sensor
    #[serde(default)]
    pub report_battery: bool,

    /// Entity the battery level is written to
    #[serde(default = "default_battery_entity")]
    pub battery_entity: String,
}

fn default_ha_url() -> String {
    "http://homeassistant.local:8123".to_string()
}
pub fn default_update_secs() -> u64 {
    300
}
/// Shortest update interval taken from the config
pub const MIN_UPDATE_SECS: u64 = 30;
fn default_timeout_secs() -> u64 {
    8
}
fn default_demo_mode() -> bool {
    true
}
pub fn default_battery_entity() -> String {
    "sensor.kobo_battery".to_string()
}

impl Default for HomeAssistantConfig {
    fn default() -> Self {
        Self {
            url: default_ha_url(),
            token: String::new(),
            update_interval_secs: default_update_secs(),
            refresh_interval_secs: None,
            timeout_secs: default_timeout_secs(),
            demo_mode_fallback: default_demo_mode(),
            report_battery: false,
            battery_entity: default_battery_entity(),
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

    /// Show the clock in the top bar. Off by default: a clock means a screen
    /// update every minute, which costs battery on e-ink.
    #[serde(default)]
    pub show_clock: bool,

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
            show_clock: false,
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
    /// Cells the page is divided into. Without it (configs from before the
    /// grid), cards flow into rows by themselves.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grid: Option<GridSize>,
    #[serde(default, deserialize_with = "known_cards")]
    pub cards: Vec<Card>,
    /// Where the page came from, e.g. `lovelace:energie`. An import only
    /// replaces and removes pages with its own mark, so your own pages survive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imported_from: Option<String>,
}

fn default_page_icon() -> String {
    "home".to_string()
}

/// Largest number of columns or rows a page grid may have
pub const MAX_GRID: u8 = 24;

/// A page grid: the area below the header split into equal cells. Cells are
/// a share of the screen, not pixels, so a page fits every Kobo model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GridSize {
    pub columns: u8,
    pub rows: u8,
}

impl GridSize {
    /// Within 1..=MAX_GRID, whatever the config says
    pub fn clamped(self) -> Self {
        Self {
            columns: self.columns.clamp(1, MAX_GRID),
            rows: self.rows.clamp(1, MAX_GRID),
        }
    }
}

/// Where a card sits on its page grid: first column and row, and how many of
/// each it covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GridPos {
    pub x: u8,
    pub y: u8,
    pub w: u8,
    pub h: u8,
}

/// A card on a page: what it shows, and where it sits on the grid.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Card {
    #[serde(flatten)]
    pub kind: CardConfig,
    /// Without it the card is placed in the first free spot
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pos: Option<GridPos>,
    /// Put aside in the editor: kept, but not on the page
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub parked: bool,
}

/// A page's cards, leaving out those of a type this version does not know
/// (a config from a newer setup tool), instead of rejecting the whole
/// config and falling back to the demo pages.
fn known_cards<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<Card>, D::Error> {
    let raw = Vec::<serde_json::Value>::deserialize(d)?;
    let mut out = Vec::with_capacity(raw.len());
    for v in raw {
        match serde_json::from_value::<Card>(v.clone()) {
            Ok(card) => out.push(card),
            Err(e) if e.to_string().contains("unknown variant") => {
                tracing::warn!(
                    "Card of type {} left out: this version does not know it",
                    v.get("type").unwrap_or(&serde_json::Value::Null)
                );
            }
            Err(e) => return Err(serde::de::Error::custom(e)),
        }
    }
    Ok(out)
}

impl From<CardConfig> for Card {
    fn from(kind: CardConfig) -> Self {
        Self {
            kind,
            pos: None,
            parked: false,
        }
    }
}

impl std::ops::Deref for Card {
    type Target = CardConfig;
    fn deref(&self) -> &CardConfig {
        &self.kind
    }
}

impl std::ops::DerefMut for Card {
    fn deref_mut(&mut self) -> &mut CardConfig {
        &mut self.kind
    }
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
        /// A graph below the controls: room temperature, target and when it heated
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        show_graph: bool,
        /// How far back the graph goes; 24 hours when not given
        #[serde(default, skip_serializing_if = "Option::is_none")]
        graph_hours: Option<u32>,
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
        /// Bigger value with a smaller icon, for reading from a distance
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        large_value: bool,
    },
    #[serde(rename = "sensor_grid")]
    SensorGrid {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        sensors: Vec<SensorItemConfig>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        large_value: bool,
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
    /// Section title, as a Lovelace `heading` card
    #[serde(rename = "heading")]
    Heading {
        #[serde(default)]
        title: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        icon: Option<String>,
    },
    /// Bar chart of Home Assistant statistics, as a Lovelace `statistics-graph`
    #[serde(rename = "statistics")]
    Statistics {
        entity_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        /// "day", "week" or "month"
        #[serde(default = "default_stat_period")]
        period: String,
        /// How far back the chart goes
        #[serde(default = "default_stat_days")]
        days: u32,
        /// A range in hours instead, for a chart of the last hours
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hours: Option<u32>,
        /// Vertical bars, horizontal bars or a line
        #[serde(default, skip_serializing_if = "ChartKind::is_bar")]
        chart: ChartKind,
        /// "change" (usage per period) or "mean" (average per period)
        #[serde(default = "default_stat_type")]
        stat: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        unit: Option<String>,
    },
    /// How far along something is (a print, a wash), as a bar and a
    /// percentage, with an optional second entity for the time left or the
    /// time it ends
    #[serde(rename = "progress")]
    Progress {
        entity_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        icon: Option<String>,
        /// Shown next to the percentage, e.g. the time left
        #[serde(default, skip_serializing_if = "Option::is_none")]
        secondary_entity: Option<String>,
        /// The value that means done; 100 when not given
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<f32>,
    },
    /// Blinds, shutters, a garage door: open, stop and close buttons and how
    /// far open it is
    #[serde(rename = "cover")]
    Cover {
        entity_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        icon: Option<String>,
    },
    /// A row of entities, each with its icon, name and state, as a Lovelace
    /// `glance` card
    #[serde(rename = "glance")]
    Glance {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        entities: Vec<SensorItemConfig>,
    },
    /// How a total splits between entities (devices' power, rooms' energy),
    /// as a ring with a legend
    #[serde(rename = "breakdown")]
    Breakdown {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        entities: Vec<SensorItemConfig>,
    },
    /// Picture from a Home Assistant `image` entity, as a Lovelace `picture`
    /// card with `image_entity` (or an `image` card).
    #[serde(rename = "image")]
    Image {
        entity_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        /// The picture alone, without the card around it
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        frameless: bool,
        /// How the picture fills its box
        #[serde(default, skip_serializing_if = "ImageFit::is_contain")]
        fit: ImageFit,
        /// A picture of your own instead of an entity's: a file in the
        /// `pictures` folder next to the config
        #[serde(default, skip_serializing_if = "Option::is_none")]
        file: Option<String>,
    },
}

/// How a statistics chart is drawn
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChartKind {
    /// A vertical bar per period
    #[default]
    Bar,
    /// A horizontal bar per period, one row each, newest at the bottom
    Hbar,
    Line,
}

impl ChartKind {
    pub fn is_bar(&self) -> bool {
        *self == ChartKind::Bar
    }
}

/// How a picture fills the card: whole and as large as fits (`contain`),
/// filling the card and cut at the edges (`cover`), or stretched to the
/// card's shape (`stretch`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageFit {
    #[default]
    Contain,
    Cover,
    Stretch,
}

impl ImageFit {
    pub fn is_contain(&self) -> bool {
        *self == ImageFit::Contain
    }
}

fn default_stat_period() -> String {
    "day".to_string()
}
fn default_stat_days() -> u32 {
    14
}
fn default_stat_type() -> String {
    // `mean` exists for every statistics sensor; `change` only for meters.
    "mean".to_string()
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
            updates: UpdatesConfig::default(),
        }
    }
}

pub fn default_demo_pages() -> Vec<PageConfig> {
    vec![
        PageConfig {
            id: "main".to_string(),
            title: "Home".to_string(),
            icon: "home".to_string(),
            imported_from: None,
            grid: None,
            cards: into_cards(vec![
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
                    show_graph: false,
                    graph_hours: None,
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
                    large_value: false,
                },
                CardConfig::MediaPlayer {
                    entity_id: "media_player.speaker".to_string(),
                    title: Some("Speaker".to_string()),
                },
            ]),
        },
        PageConfig {
            id: "climate".to_string(),
            title: "Climate".to_string(),
            icon: "thermometer".to_string(),
            imported_from: None,
            grid: None,
            cards: into_cards(vec![
                CardConfig::Climate {
                    entity_id: "climate.lounge".to_string(),
                    title: Some("Living room".to_string()),
                    step: 0.5,
                    show_graph: false,
                    graph_hours: None,
                },
                CardConfig::Climate {
                    entity_id: "climate.bedroom".to_string(),
                    title: Some("Bedroom".to_string()),
                    step: 0.5,
                    show_graph: false,
                    graph_hours: None,
                },
                CardConfig::Sensor {
                    entity_id: "sensor.energy_today".to_string(),
                    title: Some("Energy today".to_string()),
                    unit: Some("kWh".to_string()),
                    icon: Some("zap".to_string()),
                    show_graph: true,
                    large_value: false,
                },
                CardConfig::Switch {
                    entity_id: "switch.air_purifier".to_string(),
                    title: Some("Air Purifier".to_string()),
                    subtitle: Some("Auto mode".to_string()),
                    icon: Some("wind".to_string()),
                },
            ]),
        },
    ]
}

/// Cards without a place on the grid
pub fn into_cards(cards: Vec<CardConfig>) -> Vec<Card> {
    cards.into_iter().map(Card::from).collect()
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
                tracing::warn!("Config file {:?} could not be loaded ({}), using rich defaults", path.as_ref(), e);
                Self::default()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_card_of_an_unknown_type_is_left_out_not_the_config() {
        let yaml = r#"
pages:
  - id: home
    title: Home
    cards:
      - { type: light, entity_id: light.a }
      - { type: hologram, entity_id: light.b }
      - { type: switch, entity_id: switch.c }
"#;
        let config: AppConfig = serde_yaml::from_str(yaml).expect("the config still loads");
        assert_eq!(config.pages[0].cards.len(), 2);
        // A known type with a broken field is still an error
        let broken = "pages:\n  - { id: h, title: H, cards: [{ type: light }] }\n";
        assert!(serde_yaml::from_str::<AppConfig>(broken).is_err());
    }

    #[test]
    fn libra_2_touch_axes() {
        let dev = DeviceConfig::default();
        assert_eq!(dev.touch_axes(""), (true, true, false), "other Kobos keep the generic axes");
        assert_eq!(dev.touch_axes("io"), (true, false, false));
        let libra = DeviceConfig {
            model: "libra_2".into(),
            ..DeviceConfig::default()
        };
        assert_eq!(libra.touch_axes(""), (true, false, false));
        // A config that already sets its own axes is left alone
        let custom = DeviceConfig {
            invert_touch_y: true,
            ..DeviceConfig::default()
        };
        assert_eq!(custom.touch_axes("io"), (true, true, true));

        // Real tap from a Libra 2 in landscape: the "+" of a climate card
        let (swap_xy, invert_x, invert_y) = DeviceConfig::default().touch_axes("io");
        let mapping = crate::input::TouchMapping {
            portrait_w: 1264,
            portrait_h: 1680,
            swap_xy,
            invert_x,
            invert_y,
            canvas_rotation: Rotation::R90,
        };
        let (x, y) = mapping.to_canvas(973.0 / 1680.0, 788.0 / 1264.0);
        assert_eq!((x.round(), y.round()), (973.0, 475.0));
    }

    #[test]
    fn example_config_parses() {
        let cfg: AppConfig = serde_yaml::from_str(include_str!("../config.example.yaml")).expect("config.example.yaml parses");
        assert_eq!(cfg.version, CONFIG_VERSION);
        assert!(!cfg.pages.is_empty());
    }

    #[test]
    fn a_page_without_an_import_mark_still_loads() {
        let plain: PageConfig = serde_yaml::from_str("id: home\ntitle: Home\ncards: []").expect("older config parses");
        assert_eq!(plain.imported_from, None);
        let marked: PageConfig = serde_yaml::from_str("id: energy\ntitle: Energy\nimported_from: lovelace:energie\ncards: []").expect("marked page parses");
        assert_eq!(marked.imported_from.as_deref(), Some("lovelace:energie"));
        let yaml = serde_yaml::to_string(&plain).unwrap();
        assert!(!yaml.contains("imported_from"), "an unmarked page writes no empty field");
    }

    #[test]
    fn round_trip_omits_empty_options() {
        let yaml = serde_yaml::to_string(&AppConfig::default()).unwrap();
        assert!(!yaml.contains("null"), "None fields should be skipped:\n{}", yaml);
        let back: AppConfig = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(back.pages.len(), AppConfig::default().pages.len());
    }

    #[test]
    fn grid_places_round_trip_and_older_pages_have_none() {
        let yaml = "id: home\ntitle: Home\ngrid: { columns: 6, rows: 9 }\ncards:\n  - type: switch\n    entity_id: switch.a\n    pos: { x: 4, y: 7, w: 2, h: 2 }\n  - type: switch\n    entity_id: switch.b\n";
        let page: PageConfig = serde_yaml::from_str(yaml).expect("grid page parses");
        assert_eq!(page.grid, Some(GridSize { columns: 6, rows: 9 }));
        assert_eq!(page.cards[0].pos, Some(GridPos { x: 4, y: 7, w: 2, h: 2 }));
        assert_eq!(page.cards[1].pos, None);
        assert!(matches!(&page.cards[0].kind, CardConfig::Switch { entity_id, .. } if entity_id == "switch.a"));

        let out = serde_yaml::to_string(&page).unwrap();
        let back: PageConfig = serde_yaml::from_str(&out).unwrap();
        assert_eq!(back.cards[0].pos, page.cards[0].pos);
        assert_eq!(out.matches("pos:").count(), 1, "a card without a place writes none:\n{}", out);
        // The card's type still comes first, as in hand-written configs
        assert!(out.contains("- type: switch"), "{}", out);

        let old: PageConfig = serde_yaml::from_str("id: home\ntitle: Home\ncards:\n  - type: switch\n    entity_id: switch.a\n").unwrap();
        assert_eq!(old.grid, None);
        assert!(!serde_yaml::to_string(&old).unwrap().contains("grid"));
        // The setup tool talks JSON: the place is a field of the card there too
        let json = serde_json::to_value(&page).unwrap();
        assert_eq!(json["cards"][0]["pos"]["x"], 4);
        assert_eq!(json["cards"][0]["type"], "switch");
    }
}
