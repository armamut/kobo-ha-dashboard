//! A row of entities at a glance: icon, name and state for each.

use crate::config::SensorItemConfig;
use crate::display::{Canvas, TextAlign};
use crate::ha::models::EntityState;
use crate::ui::layout::{InteractiveTarget, Rect, UiAction};
use crate::ui::theme::Theme;
use crate::ui::widgets::{draw_card, format_entity_state, type_scale, PAD};
use std::collections::HashMap;

pub const GLANCE_TITLE_H: f32 = 40.0;
/// Narrowest a cell gets before the row wraps
const CELL_MIN_W: f32 = 110.0;
pub const CELL_H: f32 = 112.0;
pub const CELL_MIN_H: f32 = 76.0;

pub fn glance_columns(width: f32, count: usize) -> usize {
    (((width - 2.0 * PAD) / CELL_MIN_W).floor() as usize).clamp(1, count.max(1))
}

/// Height of a glance card at `width` with cells `cell_h` tall
pub fn glance_height(width: f32, count: usize, has_title: bool, cell_h: f32) -> f32 {
    let rows = count.max(1).div_ceil(glance_columns(width, count));
    2.0 * PAD + if has_title { GLANCE_TITLE_H } else { 0.0 } + rows as f32 * cell_h
}

/// States that mean "something is on or open", drawn in colour
fn active(state: &str) -> bool {
    matches!(state, "on" | "open" | "opening" | "unlocked" | "home" | "playing" | "detected" | "problem")
}

fn default_icon(state: Option<&EntityState>, entity_id: &str) -> &'static str {
    let class = state.and_then(|s| s.get_attr_str("device_class"));
    match (entity_id.split('.').next().unwrap_or(""), class) {
        (_, Some("door") | Some("garage_door") | Some("opening")) => "door",
        (_, Some("window")) => "window",
        (_, Some("motion") | Some("occupancy") | Some("presence")) => "motion",
        (_, Some("lock")) | ("lock", _) => "lock",
        (_, Some("temperature")) => "thermometer",
        (_, Some("humidity") | Some("moisture")) => "droplet",
        (_, Some("battery")) => "battery",
        ("light", _) => "lightbulb",
        ("person", _) | ("device_tracker", _) => "person",
        ("cover", _) => "blinds",
        ("climate", _) => "thermometer",
        ("media_player", _) => "music",
        _ => "zap",
    }
}

pub fn render_glance_card(
    canvas: &mut Canvas,
    theme: &Theme,
    rect: Rect,
    custom_title: Option<&str>,
    items: &[SensorItemConfig],
    states: &HashMap<String, EntityState>,
    targets: &mut Vec<InteractiveTarget>,
) {
    draw_card(canvas, theme, rect, None);
    let mut y = rect.y + PAD;
    if let Some(title) = custom_title {
        canvas.draw_text_fit(
            title,
            rect.x + PAD,
            y - 2.0,
            type_scale::BODY,
            true,
            theme.text_muted,
            TextAlign::Left,
            rect.w - 2.0 * PAD,
        );
        y += GLANCE_TITLE_H;
    }
    if !items.is_empty() {
        let cols = glance_columns(rect.w, items.len());
        let rows = items.len().div_ceil(cols);
        let cell_w = (rect.w - 2.0 * PAD) / cols as f32;
        let cell_h = (rect.y + rect.h - PAD - y) / rows as f32;
        // A short cell leaves the name out
        let with_name = cell_h >= 96.0;
        let icon_size = (cell_h * 0.3).clamp(22.0, 34.0);
        for (i, item) in items.iter().enumerate() {
            let (r, c) = (i / cols, i % cols);
            let cx = rect.x + PAD + c as f32 * cell_w + cell_w / 2.0;
            let state = states.get(&item.entity_id);
            let raw = state.map(|s| s.state.as_str()).unwrap_or("unavailable");
            let icon = item.icon.as_deref().unwrap_or_else(|| default_icon(state, &item.entity_id));
            let color = if active(raw) { theme.accent.solid } else { theme.text_muted };
            let value = state.map(format_entity_state).unwrap_or_else(|| "--".into());
            let unit = item.unit.clone().or_else(|| state.and_then(|s| s.unit_of_measurement())).unwrap_or_default();
            // A unit only after a number, not after "Open" or "Home"
            let numeric = state.is_some_and(|s| s.state.parse::<f64>().is_ok());
            let value = if numeric && !unit.is_empty() { format!("{} {}", value, unit) } else { value };
            let block = icon_size + 8.0 + type_scale::BODY + if with_name { 6.0 + type_scale::LABEL } else { 0.0 };
            let top = y + r as f32 * cell_h + (cell_h - block) / 2.0;
            canvas.draw_icon(icon, cx - icon_size / 2.0, top, icon_size, color, 2.2);
            let mut ty = top + icon_size + 8.0;
            if with_name {
                canvas.draw_text_fit(
                    &item.title,
                    cx,
                    ty,
                    type_scale::LABEL,
                    false,
                    theme.text_muted,
                    TextAlign::Center,
                    cell_w - 12.0,
                );
                ty += type_scale::LABEL + 6.0;
            }
            canvas.draw_text_fit(&value, cx, ty, type_scale::BODY, true, theme.text, TextAlign::Center, cell_w - 12.0);
        }
    }
    targets.push(InteractiveTarget {
        rect,
        action: UiAction::Refresh,
    });
}
