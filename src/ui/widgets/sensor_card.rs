use crate::config::SensorItemConfig;
use crate::display::{Canvas, TextAlign};
use crate::ha::client::HISTORY_ATTR;
use crate::ha::models::EntityState;
use crate::ui::layout::{InteractiveTarget, Rect, UiAction};
use crate::ui::theme::{Theme, Tone};
use crate::ui::widgets::{draw_card, draw_icon_badge, draw_value_unit, format_number, type_scale, PAD};
use std::collections::HashMap;

pub const TILE_H: f32 = 132.0;
pub const TILE_GAP: f32 = 14.0;
pub const SECTION_TITLE_H: f32 = 40.0;

fn tone_for_icon(theme: &Theme, icon: &str) -> Tone {
    match icon {
        "thermometer" | "temperature" | "flame" | "heat" => theme.heat,
        "droplet" | "humidity" | "snowflake" | "cool" => theme.cool,
        "wind" | "fan" | "leaf" => theme.eco,
        "zap" | "energy" | "power" | "plug" | "bolt" | "sun" | "battery" => theme.warm,
        "printer" | "layers" | "clock" | "progress" | "washer" => theme.media,
        _ => theme.accent,
    }
}

fn history(state: Option<&EntityState>) -> Vec<f32> {
    state
        .and_then(|s| s.attributes.get(HISTORY_ATTR))
        .and_then(|v| serde_json::from_value::<Vec<f32>>(v.clone()).ok())
        .unwrap_or_default()
}

/// The largest size up to `max` at which value and unit fit in `max_w`, so a
/// large value shrinks instead of being cut off.
fn fit_value_size(canvas: &Canvas, value: &str, unit: &str, max: f32, max_w: f32) -> f32 {
    let mut size = max;
    while size > type_scale::VALUE {
        let unit_size = (size * 0.5).max(20.0);
        let unit_w = if unit.is_empty() { 0.0 } else { canvas.measure_text(unit, unit_size, false).0 + 6.0 };
        if canvas.measure_text(value, size, true).0 + unit_w <= max_w {
            break;
        }
        size -= 4.0;
    }
    size.max(type_scale::VALUE)
}

pub fn sensor_grid_columns(width: f32, count: usize) -> usize {
    if width >= 900.0 {
        count.clamp(1, 4)
    } else {
        count.clamp(1, 2)
    }
}

pub fn sensor_grid_height(width: f32, count: usize, has_title: bool) -> f32 {
    let cols = sensor_grid_columns(width, count);
    let rows = count.div_ceil(cols).max(1);
    2.0 * PAD + if has_title { SECTION_TITLE_H } else { 0.0 } + rows as f32 * TILE_H + (rows - 1) as f32 * TILE_GAP
}

pub fn render_sensor_card(
    canvas: &mut Canvas,
    theme: &Theme,
    rect: Rect,
    entity_id: &str,
    state: Option<&EntityState>,
    custom_title: Option<&str>,
    custom_unit: Option<&str>,
    custom_icon: Option<&str>,
    show_graph: bool,
    large_value: bool,
    targets: &mut Vec<InteractiveTarget>,
) {
    let title = custom_title
        .map(|s| s.to_string())
        .or_else(|| state.map(|s| s.friendly_name()))
        .unwrap_or_else(|| entity_id.to_string());
    let value = state.map(|s| format_number(&s.state)).unwrap_or_else(|| "--".to_string());
    let unit = custom_unit
        .map(|s| s.to_string())
        .or_else(|| state.and_then(|s| s.unit_of_measurement()))
        .unwrap_or_default();
    let icon = custom_icon.unwrap_or("zap");
    let tone = tone_for_icon(theme, icon);

    draw_card(canvas, theme, rect, None);

    let badge = if large_value { 40.0 } else { 60.0 };
    let bx = rect.x + PAD;
    let by = rect.y + PAD;
    draw_icon_badge(canvas, theme, bx, by, badge, icon, tone, true);
    let tx = bx + badge + 16.0;
    canvas.draw_text_fit(
        &title,
        tx,
        by + badge / 2.0 - 13.0,
        type_scale::BODY,
        true,
        theme.text_muted,
        TextAlign::Left,
        rect.x + rect.w - PAD - tx,
    );

    let values = history(state);
    let graph = show_graph && values.len() >= 2;
    let value_size = if large_value { fit_value_size(canvas, &value, &unit, 84.0, rect.w - 2.0 * PAD) } else { 56.0 };
    let value_y = if graph { by + badge + 12.0 } else { rect.y + rect.h - PAD - value_size };
    draw_value_unit(canvas, theme, bx, value_y, &value, &unit, value_size, rect.w - 2.0 * PAD);

    if graph {
        let gy = value_y + value_size + 12.0;
        let gh = (rect.y + rect.h - PAD - gy).max(24.0);
        canvas.draw_sparkline(bx, gy, rect.w - 2.0 * PAD, gh, &values, tone.solid, 3.0, Some(tone.tint));
    }

    targets.push(InteractiveTarget {
        rect,
        action: UiAction::Refresh,
    });
}

pub fn render_sensor_grid(
    canvas: &mut Canvas,
    theme: &Theme,
    rect: Rect,
    custom_title: Option<&str>,
    sensors: &[SensorItemConfig],
    large_value: bool,
    all_states: &HashMap<String, EntityState>,
    targets: &mut Vec<InteractiveTarget>,
) {
    draw_card(canvas, theme, rect, None);

    let mut y = rect.y + PAD;
    if let Some(title) = custom_title {
        canvas.draw_text(title, rect.x + PAD, y - 2.0, type_scale::BODY, true, theme.text_muted, TextAlign::Left);
        y += SECTION_TITLE_H;
    }

    let cols = sensor_grid_columns(rect.w, sensors.len());
    let col_w = (rect.w - 2.0 * PAD - (cols - 1) as f32 * TILE_GAP) / cols as f32;
    let tile_h = TILE_H.min((rect.y + rect.h - PAD - y - TILE_GAP * (sensors.len().div_ceil(cols) as f32 - 1.0))
        / sensors.len().div_ceil(cols).max(1) as f32);

    for (idx, item) in sensors.iter().enumerate() {
        let (r, c) = (idx / cols, idx % cols);
        let cell = Rect::new(
            rect.x + PAD + c as f32 * (col_w + TILE_GAP),
            y + r as f32 * (tile_h + TILE_GAP),
            col_w,
            tile_h,
        );
        canvas.fill_rounded_rect(cell.x, cell.y, cell.w, cell.h, 16.0, theme.surface_alt);

        let icon = item.icon.as_deref().unwrap_or("zap");
        let state = all_states.get(&item.entity_id);
        let low_battery = icon == "battery"
            && state.and_then(|s| s.state.parse::<f32>().ok()).map(|v| v < 20.0).unwrap_or(false);
        let tone = if low_battery { theme.heat } else { tone_for_icon(theme, icon) };
        let ip = 16.0;
        let badge = if large_value { 36.0 } else { 48.0 };
        draw_icon_badge(canvas, theme, cell.x + ip, cell.y + ip, badge, icon, tone, true);
        canvas.draw_text_fit(
            &item.title,
            cell.x + ip + badge + 12.0,
            cell.y + ip + badge / 2.0 - 12.0,
            type_scale::LABEL,
            false,
            theme.text_muted,
            TextAlign::Left,
            cell.w - 2.0 * ip - badge - 12.0,
        );

        let value = state.map(|s| format_number(&s.state)).unwrap_or_else(|| "--".to_string());
        let unit = item
            .unit
            .clone()
            .or_else(|| state.and_then(|s| s.unit_of_measurement()))
            .unwrap_or_default();
        let value_size = if large_value {
            fit_value_size(canvas, &value, &unit, 60.0, cell.w - 2.0 * ip)
        } else {
            type_scale::VALUE
        };
        draw_value_unit(
            canvas,
            theme,
            cell.x + ip,
            cell.y + cell.h - ip - value_size,
            &value,
            &unit,
            value_size,
            cell.w - 2.0 * ip,
        );
    }

    targets.push(InteractiveTarget {
        rect,
        action: UiAction::Refresh,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_values_shrink_to_fit() {
        let canvas = Canvas::new(400, 200);
        // A short value gets the full size
        assert_eq!(fit_value_size(&canvas, "21,6", "°C", 84.0, 290.0), 84.0);
        // A long one shrinks until it fits, but never below the normal size
        let size = fit_value_size(&canvas, "12.345", "kWh", 84.0, 290.0);
        assert!(size < 84.0 && size >= type_scale::VALUE);
        assert_eq!(fit_value_size(&canvas, "123456789", "kWh", 84.0, 100.0), type_scale::VALUE);
    }
}
