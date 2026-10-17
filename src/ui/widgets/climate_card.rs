use crate::display::{Canvas, TextAlign};
use crate::i18n;
use crate::ha::models::EntityState;
use crate::ui::layout::{InteractiveTarget, Rect, UiAction};
use crate::ui::theme::Theme;
use crate::ui::widgets::{draw_card, draw_icon_badge, draw_round_button, type_scale, PAD};

/// Width above which the card uses its single-row layout
pub const CLIMATE_WIDE: f32 = 760.0;

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
    let title = custom_title
        .map(|s| s.to_string())
        .or_else(|| state.map(|s| s.friendly_name()))
        .unwrap_or_else(|| i18n::t("climate.default_title"));

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
    let badge = 68.0;
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
    } else {
        // Two rows: header, then temperature + buttons
        draw_icon_badge(canvas, theme, bx, by, badge, icon, tone, active);
        let tx = bx + badge + 18.0;
        let tw = rect.x + rect.w - PAD - tx;
        canvas.draw_text_fit(&title, tx, by + 2.0, type_scale::TITLE, true, theme.text, TextAlign::Left, tw);
        canvas.draw_text_fit(&subtitle, tx, by + 40.0, type_scale::LABEL, false, theme.text_muted, TextAlign::Left, tw);

        let btn = 84.0f32.min(rect.h - badge - 3.0 * PAD).max(56.0);
        let row_y = rect.y + rect.h - PAD - btn;
        let plus = Rect::new(rect.x + rect.w - PAD - btn, row_y, btn, btn);
        let minus = Rect::new(plus.x - 14.0 - btn, row_y, btn, btn);

        let temp_size = 68.0;
        let temp_y = row_y + btn / 2.0 - temp_size * 0.5;
        let w = canvas.draw_text_fit(&temp_str, bx, temp_y, temp_size, true, theme.text, TextAlign::Left, minus.x - bx - 50.0);
        canvas.draw_text("°C", bx + w + 4.0, temp_y + 6.0, 26.0, false, theme.text_muted, TextAlign::Left);
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
