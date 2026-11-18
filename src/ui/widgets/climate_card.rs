use crate::display::{Canvas, TextAlign};
use crate::i18n;
use crate::ha::models::EntityState;
use crate::ui::layout::{InteractiveTarget, Rect, UiAction};
use crate::ui::theme::Theme;
use crate::ui::widgets::{draw_card, draw_icon_badge, draw_round_button, type_scale, PAD};

/// Width above which the card uses its single-row layout
pub const CLIMATE_WIDE: f32 = 760.0;

/// Size of the icon badge in the top left corner.
const BADGE: f32 = 68.0;

/// Button size and the room left for the temperature on a card narrower than
/// `CLIMATE_WIDE`. The buttons shrink with the card: a number with no room left
/// used to be cut off to dots.
fn narrow_budget(canvas: &Canvas, rect: Rect) -> (f32, f32) {
    let room = rect.h - BADGE - 3.0 * PAD;
    let btn = (rect.w * 0.16).clamp(48.0, 64.0).min(room.max(48.0));
    let unit_w = canvas.measure_text("°C", 26.0, false).0;
    // Padding on both sides, two buttons and their gap, 12 px before the unit
    // and 8 px after the number.
    let avail = rect.w - 2.0 * PAD - 2.0 * btn - 12.0 - unit_w - 8.0;
    (btn, avail)
}

pub fn render_climate_card(
    canvas: &mut Canvas,
    theme: &Theme,
    rect: Rect,
    entity_id: &str,
    state: Option<&EntityState>,
    custom_title: Option<&str>,
    step: f32,
    targets: &mut Vec<InteractiveTarget>,
) {
    let title = title_text(custom_title, state);

    let target_temp = state.and_then(|s| s.get_attr_f32("temperature"));
    let current_temp = state.and_then(|s| s.get_attr_f32("current_temperature"));
    let mode = state.map(|s| s.state.as_str()).unwrap_or("unavailable");
    let action = state.and_then(|s| s.get_attr_str("hvac_action")).unwrap_or("");

    let (tone, active, status_key) = match (action, mode) {
        ("heating", _) => (theme.heat, true, "state.heating"),
        ("cooling", _) => (theme.cool, true, "state.cooling"),
        ("drying", _) => (theme.cool, true, "state.drying"),
        ("fan", _) => (theme.cool, true, "state.fan"),
        (_, "off") => (theme.idle, false, "state.off"),
        (_, "unavailable") => (theme.idle, false, "state.unavailable"),
        (_, "heat") => (theme.heat, false, "state.idle"),
        (_, "cool") => (theme.cool, false, "state.idle"),
        _ => (theme.idle, false, "state.idle"),
    };
    let status = i18n::t(status_key);
    let icon = match (action, mode) {
        ("cooling", _) | (_, "cool") => "snowflake",
        ("heating", _) => "flame",
        _ => "thermometer",
    };

    draw_card(canvas, theme, rect, active.then_some(tone));

    let subtitle = match current_temp {
        Some(t) => format!("{} · {}", status, i18n::tf("climate.room", &[("temp", &i18n::num(t as f64, 1))])),
        None => status,
    };
    let temp_str = target_temp.map(|t| i18n::num(t as f64, 1)).unwrap_or_else(|| "--".to_string());
    let badge = BADGE;
    let bx = rect.x + PAD;
    let by = rect.y + PAD;

    let (minus_rect, plus_rect) = if rect.w >= CLIMATE_WIDE {
        // [badge] title/subtitle ........ 22.5°C  (-) (+)
        let btn = (rect.h - 2.0 * PAD).min(100.0);
        let cy = rect.y + rect.h / 2.0;
        let plus = Rect::new(rect.x + rect.w - PAD - btn, cy - btn / 2.0, btn, btn);
        let minus = Rect::new(plus.x - 18.0 - btn, plus.y, btn, btn);

        let temp_size = type_scale::HERO;
        let unit_w = canvas.measure_text("°C", 30.0, false).0;
        let temp_w = canvas.measure_text(&temp_str, temp_size, true).0;
        let temp_right = minus.x - 28.0;
        let temp_x = temp_right - unit_w - 4.0 - temp_w;
        let temp_y = cy - temp_size * 0.5;
        canvas.draw_text(&temp_str, temp_x, temp_y, temp_size, true, theme.text, TextAlign::Left);
        canvas.draw_text("°C", temp_right - unit_w, temp_y + 8.0, 30.0, false, theme.text_muted, TextAlign::Left);

        draw_icon_badge(canvas, theme, bx, cy - badge / 2.0, badge, icon, tone, active);
        let tx = bx + badge + 18.0;
        let tw = temp_x - 20.0 - tx;
        canvas.draw_text_fit(&title, tx, cy - 36.0, type_scale::TITLE, true, theme.text, TextAlign::Left, tw);
        canvas.draw_text_fit(&subtitle, tx, cy + 4.0, type_scale::LABEL, false, theme.text_muted, TextAlign::Left, tw);
        (minus, plus)
    } else if rect.h < COMPACT_BELOW {
        // Short: a small icon and the title, then temperature + buttons. The
        // tint already says whether it is heating.
        let p = 18.0;
        let icon_size = 28.0;
        canvas.draw_icon(icon, rect.x + p, rect.y + p, icon_size, tone.solid, 2.2);
        let tx = rect.x + p + icon_size + 10.0;
        canvas.draw_text_fit(&title, tx, rect.y + p + 1.0, type_scale::BODY, true, theme.text, TextAlign::Left, rect.x + rect.w - p - tx);

        let btn = (rect.h - 2.0 * p - icon_size - 12.0).min(rect.w * 0.2).clamp(40.0, 64.0);
        let row_y = rect.y + rect.h - p - btn;
        let plus = Rect::new(rect.x + rect.w - p - btn, row_y, btn, btn);
        let minus = Rect::new(plus.x - 10.0 - btn, row_y, btn, btn);
        draw_temperature(canvas, theme, rect.x + p, row_y + btn / 2.0, &temp_str, minus.x - 8.0 - (rect.x + p));
        (minus, plus)
    } else {
        // Two rows: header, then temperature + buttons
        draw_icon_badge(canvas, theme, bx, by, badge, icon, tone, active);
        let tx = bx + badge + 18.0;
        let tw = rect.x + rect.w - PAD - tx;
        canvas.draw_text_fit(&title, tx, by + 2.0, type_scale::TITLE, true, theme.text, TextAlign::Left, tw);
        canvas.draw_text_fit(&subtitle, tx, by + 40.0, type_scale::LABEL, false, theme.text_muted, TextAlign::Left, tw);

        // Two buttons that shrink with the card leave room for the number; with
        // a fixed 84 px and a fixed 68 px number it was cut off to dots.
        let (btn, avail) = narrow_budget(canvas, rect);
        let row_y = rect.y + rect.h - PAD - btn;
        let plus = Rect::new(rect.x + rect.w - PAD - btn, row_y, btn, btn);
        let minus = Rect::new(plus.x - 10.0 - btn, row_y, btn, btn);

        // The temperature is the point of the card: shrink it until it fits.
        let unit_w = canvas.measure_text("°C", 26.0, false).0;
        draw_temperature(canvas, theme, bx, row_y + btn / 2.0, &temp_str, avail + 12.0 + unit_w);
        (minus, plus)
    };

    draw_round_button(canvas, theme, minus_rect, "minus", tone, false);
    draw_round_button(canvas, theme, plus_rect, "plus", tone, false);

    // Generous hit areas around the buttons
    targets.push(InteractiveTarget {
        rect: minus_rect.inset(-10.0),
        action: UiAction::SetClimateTemp(entity_id.to_string(), -step),
    });
    targets.push(InteractiveTarget {
        rect: plus_rect.inset(-10.0),
        action: UiAction::SetClimateTemp(entity_id.to_string(), step),
    });
}

/// Cards shorter than this (and narrower than `CLIMATE_WIDE`) drop the badge
/// and the subtitle.
const COMPACT_BELOW: f32 = 190.0;

/// The temperature and its unit in `room` px, centred on `cy`. On a very
/// narrow card the unit goes first, so the number is never cut off.
fn draw_temperature(canvas: &mut Canvas, theme: &Theme, x: f32, cy: f32, temp: &str, room: f32) {
    let unit_w = canvas.measure_text("°C", 26.0, false).0;
    let with_unit = canvas.measure_text(temp, 40.0, true).0 <= room - 12.0 - unit_w;
    let avail = if with_unit { room - 12.0 - unit_w } else { room };
    let size = temp_font_size(canvas, temp, avail);
    let y = cy - size * 0.5;
    let w = canvas.draw_text_fit(temp, x, y, size, true, theme.text, TextAlign::Left, avail);
    if with_unit {
        canvas.draw_text("°C", x + w + 4.0, y + 6.0, 26.0, false, theme.text_muted, TextAlign::Left);
    }
}

/// The size the temperature is drawn at: the largest that fits next to the unit
/// and the buttons, so a long value shrinks instead of being cut off to dots.
fn temp_font_size(canvas: &Canvas, text: &str, avail: f32) -> f32 {
    [68.0f32, 60.0, 52.0, 46.0, 40.0]
        .into_iter()
        .find(|size| canvas.measure_text(text, *size, true).0 <= avail)
        .unwrap_or(40.0)
}

/// The card's heading: the configured title, else Home Assistant's name, with
/// underscores read as spaces (Home Assistant names often carry them, and a
/// trimmed "thermostaat_z…" said nothing about the room).
fn title_text(custom_title: Option<&str>, state: Option<&EntityState>) -> String {
    custom_title
        .map(|s| s.to_string())
        .or_else(|| state.map(|s| s.friendly_name()))
        .unwrap_or_else(|| i18n::t("climate.default_title"))
        .replace('_', " ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn state(entity_id: &str, name: Option<&str>) -> EntityState {
        let mut attributes = HashMap::new();
        if let Some(name) = name {
            attributes.insert("friendly_name".to_string(), serde_json::Value::String(name.to_string()));
        }
        EntityState {
            entity_id: entity_id.to_string(),
            state: "heat".to_string(),
            attributes,
            last_changed: None,
            last_updated: None,
        }
    }

    #[test]
    fn a_name_with_underscores_reads_as_words() {
        let s = state("climate.thermostaat_zone", Some("thermostaat_zone_gang"));
        assert_eq!(title_text(None, Some(&s)), "thermostaat zone gang");
        assert_eq!(title_text(Some("Thermostaat_woonkamer"), Some(&s)), "Thermostaat woonkamer");
    }

    #[test]
    fn an_entity_without_a_name_falls_back_to_its_id_as_words() {
        let s = state("climate.living_room", None);
        assert_eq!(s.friendly_name(), "living room");
        assert_eq!(title_text(None, Some(&s)), "living room");
    }

    #[test]
    fn without_a_state_the_card_shows_its_own_title() {
        assert_eq!(title_text(None, None), i18n::t("climate.default_title"));
    }

    #[test]
    fn the_temperature_fits_every_card_the_grid_makes() {
        let canvas = Canvas::new(400, 200);
        // Half-width cards run from the smallest screens to just below the wide
        // layout; the number must never be cut off on any of them.
        for w in [300.0f32, 340.0, 380.0, 410.0, 480.0, 560.0, 700.0, 759.0] {
            let rect = crate::ui::layout::Rect::new(0.0, 0.0, w, 206.0);
            let (_, avail) = narrow_budget(&canvas, rect);
            for value in ["8", "16.5", "21.5", "-12.5"] {
                let size = temp_font_size(&canvas, value, avail);
                assert!(size >= 40.0, "{value} has no room on a {w} px card");
                assert!(
                    canvas.measure_text(value, size, true).0 <= avail,
                    "{value} is still cut off on a {w} px card"
                );
            }
        }
        assert_eq!(temp_font_size(&canvas, "8", 400.0), 68.0);
    }
}
