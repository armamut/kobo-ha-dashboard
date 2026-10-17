pub mod climate_card;
pub mod header;
pub mod light_card;
pub mod media_card;
pub mod scene_card;
pub mod sensor_card;
pub mod switch_card;
pub mod weather_card;

pub use climate_card::render_climate_card;
pub use header::render_header;
pub use light_card::render_light_card;
pub use media_card::render_media_card;
pub use scene_card::render_scenes_row;
pub use sensor_card::{render_sensor_card, render_sensor_grid};
pub use switch_card::render_switch_card;
pub use weather_card::render_weather_card;

use crate::display::{Canvas, TextAlign};
use crate::ui::layout::Rect;
use crate::ui::theme::{Theme, Tone};

/// Shared type scale (px at 300 ppi)
pub mod type_scale {
    pub const LABEL: f32 = 22.0;
    pub const BODY: f32 = 24.0;
    pub const TITLE: f32 = 28.0;
    pub const VALUE: f32 = 40.0;
    pub const HERO: f32 = 84.0;
}

pub const PAD: f32 = 24.0;

/// Card surface. Active cards get a tint and a coloured border.
pub fn draw_card(canvas: &mut Canvas, theme: &Theme, rect: Rect, active: Option<Tone>) {
    let r = theme.card_radius;
    match active {
        Some(tone) => {
            canvas.fill_rounded_rect(rect.x, rect.y, rect.w, rect.h, r, tone.tint);
            canvas.stroke_rounded_rect(rect.x + 1.5, rect.y + 1.5, rect.w - 3.0, rect.h - 3.0, r - 1.5, 3.0, tone.solid);
        }
        None => {
            canvas.fill_rounded_rect(rect.x, rect.y, rect.w, rect.h, r, theme.surface);
            canvas.stroke_rounded_rect(rect.x + 1.0, rect.y + 1.0, rect.w - 2.0, rect.h - 2.0, r - 1.0, 2.0, theme.border);
        }
    }
}

/// Round icon badge; filled with the tone when active, neutral otherwise.
pub fn draw_icon_badge(canvas: &mut Canvas, theme: &Theme, x: f32, y: f32, d: f32, icon: &str, tone: Tone, active: bool) {
    let (bg, fg) = if active { (tone.solid, tone.on_solid) } else { (theme.surface_alt, tone.solid) };
    canvas.fill_circle(x + d / 2.0, y + d / 2.0, d / 2.0, bg);
    if !active {
        canvas.stroke_circle(x + d / 2.0, y + d / 2.0, d / 2.0 - 1.0, 2.0, theme.border);
    }
    let icon_size = d * 0.52;
    canvas.draw_icon(icon, x + (d - icon_size) / 2.0, y + (d - icon_size) / 2.0, icon_size, fg, 2.2);
}

/// iOS-style toggle switch drawn at (x, y) with the given width.
pub fn draw_toggle(canvas: &mut Canvas, theme: &Theme, x: f32, y: f32, w: f32, on: bool, tone: Tone) {
    let h = w * 0.56;
    let r = h / 2.0;
    if on {
        canvas.fill_rounded_rect(x, y, w, h, r, tone.solid);
        canvas.fill_circle(x + w - r, y + r, r - 5.0, tone.on_solid);
    } else {
        canvas.fill_rounded_rect(x, y, w, h, r, theme.surface_alt);
        canvas.stroke_rounded_rect(x + 1.0, y + 1.0, w - 2.0, h - 2.0, r - 1.0, 2.0, theme.border_strong);
        canvas.fill_circle(x + r, y + r, r - 7.0, theme.border_strong);
    }
}

/// Small rounded status chip; returns its width.
pub fn draw_chip(canvas: &mut Canvas, theme: &Theme, right_x: f32, y: f32, text: &str, tone: Tone, filled: bool) -> f32 {
    let size = 20.0;
    let (tw, _) = canvas.measure_text(text, size, true);
    let w = tw + 28.0;
    let h = 38.0;
    let x = right_x - w;
    if filled {
        canvas.fill_rounded_rect(x, y, w, h, h / 2.0, tone.solid);
        canvas.draw_text(text, x + w / 2.0, y + 8.0, size, true, tone.on_solid, TextAlign::Center);
    } else {
        canvas.fill_rounded_rect(x, y, w, h, h / 2.0, theme.surface);
        canvas.stroke_rounded_rect(x + 1.0, y + 1.0, w - 2.0, h - 2.0, h / 2.0 - 1.0, 2.0, tone.solid);
        canvas.draw_text(text, x + w / 2.0, y + 8.0, size, true, tone.solid, TextAlign::Center);
    }
    w
}

/// Round push button with an icon; `primary` fills it with the tone.
pub fn draw_round_button(canvas: &mut Canvas, theme: &Theme, rect: Rect, icon: &str, tone: Tone, primary: bool) {
    let d = rect.w.min(rect.h);
    let cx = rect.x + rect.w / 2.0;
    let cy = rect.y + rect.h / 2.0;
    let fg = if primary {
        canvas.fill_circle(cx, cy, d / 2.0, tone.solid);
        tone.on_solid
    } else {
        canvas.fill_circle(cx, cy, d / 2.0, theme.surface);
        canvas.stroke_circle(cx, cy, d / 2.0 - 1.25, 2.5, theme.border_strong);
        theme.text
    };
    let s = d * 0.42;
    canvas.draw_icon(icon, cx - s / 2.0, cy - s / 2.0, s, fg, 2.6);
}

/// Value in big type followed by a smaller unit; returns total width.
pub fn draw_value_unit(
    canvas: &mut Canvas,
    theme: &Theme,
    x: f32,
    y: f32,
    value: &str,
    unit: &str,
    size: f32,
    max_w: f32,
) -> f32 {
    let unit_size = (size * 0.5).max(20.0);
    let unit_w = if unit.is_empty() { 0.0 } else { canvas.measure_text(unit, unit_size, false).0 + 6.0 };
    let vw = canvas.draw_text_fit(value, x, y, size, true, theme.text, TextAlign::Left, (max_w - unit_w).max(10.0));
    if !unit.is_empty() {
        canvas.draw_text(unit, x + vw + 6.0, y + size * 0.85 - unit_size * 0.85, unit_size, false, theme.text_muted, TextAlign::Left);
    }
    vw + unit_w
}

/// Format a HA numeric state compactly ("21.63" -> "21.6", "540.0" -> "540"),
/// translating well-known text states.
pub fn format_number(raw: &str) -> String {
    match raw.parse::<f64>() {
        Ok(v) if v.abs() >= 100.0 || v.fract() == 0.0 => crate::i18n::num(v, 0),
        Ok(v) => crate::i18n::num(v, 1),
        Err(_) => crate::i18n::state_label(raw),
    }
}
