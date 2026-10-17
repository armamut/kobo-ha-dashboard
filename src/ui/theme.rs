use crate::config::ThemeConfig;
use crate::display::color::Color;
use crate::display::icons::WeatherInk;

/// A semantic colour: `solid` for icons/fills, `tint` for large active
/// surfaces, `on_solid` for content drawn on top of `solid`.
#[derive(Debug, Clone, Copy)]
pub struct Tone {
    pub solid: Color,
    pub tint: Color,
    pub on_solid: Color,
}

impl Tone {
    const fn new(solid: Color, tint: Color, on_solid: Color) -> Self {
        Self { solid, tint, on_solid }
    }
}

#[derive(Debug, Clone)]
pub struct Theme {
    pub bg: Color,
    pub surface: Color,
    pub surface_alt: Color,
    pub border: Color,
    pub border_strong: Color,
    pub text: Color,
    pub text_muted: Color,
    pub text_faint: Color,
    pub card_radius: f32,
    pub is_dark: bool,

    pub accent: Tone,
    /// Lights on / warm
    pub warm: Tone,
    /// Heating, alerts
    pub heat: Tone,
    /// Cooling, water, humidity
    pub cool: Tone,
    /// Switches on, eco, OK
    pub eco: Tone,
    /// Media
    pub media: Tone,
    /// Neutral "off" state
    pub idle: Tone,

    pub weather: WeatherInk,
}

const WHITE: Color = Color::rgb(255, 255, 255);

impl Theme {
    pub fn from_config(config: &ThemeConfig) -> Self {
        let radius = config.card_corner_radius;
        let accent_solid = Color::from_hex(&config.accent_color);

        match config.mode.to_lowercase().as_str() {
            "dark" => {
                let surface = Color::rgb(30, 32, 36);
                let tone = |c: Color| Tone::new(c, c.mix(surface, 0.72), WHITE);
                Self {
                    bg: Color::rgb(14, 15, 17),
                    surface,
                    surface_alt: Color::rgb(44, 47, 53),
                    border: Color::rgb(64, 68, 76),
                    border_strong: Color::rgb(110, 116, 128),
                    text: Color::rgb(245, 246, 248),
                    text_muted: Color::rgb(176, 182, 192),
                    text_faint: Color::rgb(120, 126, 136),
                    card_radius: radius,
                    is_dark: true,
                    accent: tone(accent_solid),
                    warm: tone(Color::rgb(245, 166, 35)),
                    heat: tone(Color::rgb(239, 83, 80)),
                    cool: tone(Color::rgb(66, 165, 245)),
                    eco: tone(Color::rgb(76, 175, 80)),
                    media: tone(Color::rgb(171, 130, 255)),
                    idle: Tone::new(Color::rgb(176, 182, 192), Color::rgb(44, 47, 53), surface),
                    weather: WeatherInk {
                        line: Color::rgb(220, 224, 230),
                        sun: Color::rgb(250, 190, 40),
                        cloud: Color::rgb(70, 75, 84),
                        rain: Color::rgb(66, 165, 245),
                    },
                }
            }
            "high_contrast_bw" => {
                let ink = Color::rgb(0, 0, 0);
                let tone = Tone::new(ink, Color::rgb(222, 222, 222), WHITE);
                Self {
                    bg: WHITE,
                    surface: WHITE,
                    surface_alt: Color::rgb(238, 238, 238),
                    border: ink,
                    border_strong: ink,
                    text: ink,
                    text_muted: Color::rgb(50, 50, 50),
                    text_faint: Color::rgb(100, 100, 100),
                    card_radius: radius,
                    is_dark: false,
                    accent: tone,
                    warm: tone,
                    heat: tone,
                    cool: tone,
                    eco: tone,
                    media: tone,
                    idle: Tone::new(Color::rgb(60, 60, 60), Color::rgb(238, 238, 238), WHITE),
                    weather: WeatherInk {
                        line: ink,
                        sun: Color::rgb(160, 160, 160),
                        cloud: WHITE,
                        rain: ink,
                    },
                }
            }
            "paper_warm" => {
                let surface = Color::rgb(255, 253, 248);
                let tone = |c: Color| Tone::new(c, c.mix(surface, 0.74), WHITE);
                Self {
                    bg: Color::rgb(238, 232, 220),
                    surface,
                    surface_alt: Color::rgb(244, 239, 229),
                    border: Color::rgb(212, 202, 184),
                    border_strong: Color::rgb(150, 138, 118),
                    text: Color::rgb(33, 27, 20),
                    text_muted: Color::rgb(96, 84, 70),
                    text_faint: Color::rgb(150, 138, 122),
                    card_radius: radius,
                    is_dark: false,
                    accent: tone(accent_solid),
                    warm: tone(Color::rgb(214, 125, 0)),
                    heat: tone(Color::rgb(196, 48, 43)),
                    cool: tone(Color::rgb(30, 104, 150)),
                    eco: tone(Color::rgb(46, 125, 50)),
                    media: tone(Color::rgb(111, 66, 193)),
                    idle: Tone::new(Color::rgb(96, 84, 70), Color::rgb(244, 239, 229), surface),
                    weather: WeatherInk {
                        line: Color::rgb(60, 50, 40),
                        sun: Color::rgb(232, 160, 20),
                        cloud: WHITE,
                        rain: Color::rgb(30, 104, 150),
                    },
                }
            }
            _ => {
                // Kaleido 3: the colour filter washes out pastels, so state is
                // carried by saturated solids and fairly strong tints, while
                // text stays near-black for 300 ppi crispness.
                let surface = WHITE;
                let tone = |c: Color| Tone::new(c, c.mix(surface, 0.70), WHITE);
                Self {
                    bg: Color::rgb(232, 235, 239),
                    surface,
                    surface_alt: Color::rgb(241, 243, 246),
                    border: Color::rgb(200, 205, 213),
                    border_strong: Color::rgb(120, 128, 140),
                    text: Color::rgb(12, 16, 24),
                    text_muted: Color::rgb(72, 80, 94),
                    text_faint: Color::rgb(128, 136, 148),
                    card_radius: radius,
                    is_dark: false,
                    accent: tone(accent_solid),
                    warm: tone(Color::rgb(240, 140, 0)),
                    heat: tone(Color::rgb(220, 38, 38)),
                    cool: tone(Color::rgb(2, 116, 190)),
                    eco: tone(Color::rgb(22, 140, 64)),
                    media: tone(Color::rgb(124, 58, 237)),
                    idle: Tone::new(Color::rgb(72, 80, 94), Color::rgb(236, 239, 243), surface),
                    weather: WeatherInk {
                        line: Color::rgb(40, 48, 60),
                        sun: Color::rgb(250, 176, 5),
                        cloud: WHITE,
                        rain: Color::rgb(2, 116, 190),
                    },
                }
            }
        }
    }
}
