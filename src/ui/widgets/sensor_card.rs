use crate::config::SensorItemConfig;
use crate::display::{Canvas, TextAlign};
use crate::ha::client::HISTORY_ATTR;
use crate::ha::models::EntityState;
use crate::ui::layout::{InteractiveTarget, Rect, UiAction};
use crate::ui::theme::{Theme, Tone};
use crate::ui::widgets::{draw_card, draw_icon_badge, draw_value_unit, format_entity_state, type_scale, value_unit_width, PAD};
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
        let unit_w = if unit.is_empty() {
            0.0
        } else {
            canvas.measure_text(unit, unit_size, false).0 + 6.0
        };
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
    } else if width >= 360.0 {
        count.clamp(1, 2)
    } else {
        1
    }
}

/// Shortest tile: a small icon and the title above the value
pub const COMPACT_TILE_H: f32 = 84.0;

/// Smallest height of a sensor grid at `width`, with compact tiles.
pub fn sensor_grid_min_height(width: f32, count: usize, has_title: bool) -> f32 {
    let rows = count.div_ceil(sensor_grid_columns(width, count)).max(1);
    2.0 * PAD + if has_title { SECTION_TITLE_H } else { 0.0 } + rows as f32 * COMPACT_TILE_H + (rows - 1) as f32 * TILE_GAP
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
    let value = state.map(format_entity_state).unwrap_or_else(|| "--".to_string());
    let unit = custom_unit
        .map(|s| s.to_string())
        .or_else(|| state.and_then(|s| s.unit_of_measurement()))
        .unwrap_or_default();
    let icon = custom_icon.unwrap_or("zap");
    let tone = tone_for_icon(theme, icon);

    draw_card(canvas, theme, rect, None);

    if rect.h < ROW_CARD_H {
        render_sensor_row(canvas, theme, rect, &title, &value, &unit, icon, tone);
    } else if rect.h < TILE_CARD_H {
        draw_compact_tile(
            canvas,
            theme,
            rect,
            18.0,
            icon,
            tone,
            &title,
            &value,
            &unit,
            if large_value { 60.0 } else { 52.0 },
        );
    } else {
        let badge = if large_value || rect.w < 240.0 { 44.0 } else { 60.0 };
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

        // The graph needs some height; on a short card the value is enough.
        let values = history(state);
        let graph_room = |size: f32| rect.y + rect.h - PAD - (by + badge + 12.0 + size + 12.0);
        let graph = show_graph && values.len() >= 2 && graph_room(56.0) >= 40.0;
        // A tall card without a graph shows a larger value.
        let max_size = if large_value || (!graph && rect.h >= 220.0) { type_scale::HERO } else { 56.0 };
        let value_size = fit_value_size(canvas, &value, &unit, max_size, rect.w - 2.0 * PAD);
        let value_y = if graph { by + badge + 12.0 } else { rect.y + rect.h - PAD - value_size };
        draw_value_unit(canvas, theme, bx, value_y, &value, &unit, value_size, rect.w - 2.0 * PAD);

        if graph {
            let gy = value_y + value_size + 12.0;
            let gh = (rect.y + rect.h - PAD - gy).max(24.0);
            canvas.draw_sparkline(bx, gy, rect.w - 2.0 * PAD, gh, &values, tone.solid, 3.0, Some(tone.tint));
        }
    }

    targets.push(InteractiveTarget {
        rect,
        action: UiAction::Refresh,
    });
}

/// Cards shorter than this show the compact tile: a small icon and the title
/// above the value.
pub const TILE_CARD_H: f32 = 140.0;
/// Cards shorter than this show a single row: icon, title, value.
pub const ROW_CARD_H: f32 = 96.0;

/// A small icon beside the title, the value below: a short card, or a tile of
/// a sensor grid with little room.
#[allow(clippy::too_many_arguments)]
fn draw_compact_tile(canvas: &mut Canvas, theme: &Theme, cell: Rect, ip: f32, icon: &str, tone: Tone, title: &str, value: &str, unit: &str, max_value: f32) {
    let icon_size = 26.0;
    canvas.draw_icon(icon, cell.x + ip, cell.y + ip, icon_size, tone.solid, 2.2);
    canvas.draw_text_fit(
        title,
        cell.x + ip + icon_size + 8.0,
        cell.y + ip + 1.0,
        type_scale::LABEL,
        false,
        theme.text_muted,
        TextAlign::Left,
        cell.w - 2.0 * ip - icon_size - 8.0,
    );
    let room = cell.h - 2.0 * ip - icon_size - 4.0;
    let mut size = max_value.min(room).max(24.0);
    while size > 24.0 && value_unit_width(canvas, value, unit, size) > cell.w - 2.0 * ip {
        size -= 2.0;
    }
    draw_value_unit(canvas, theme, cell.x + ip, cell.y + cell.h - ip - size, value, unit, size, cell.w - 2.0 * ip);
}

/// One row: icon, title and the value on the right. The title goes first
/// when room runs out, then the icon; the value always stays.
#[allow(clippy::too_many_arguments)]
fn render_sensor_row(canvas: &mut Canvas, theme: &Theme, rect: Rect, title: &str, value: &str, unit: &str, icon: &str, tone: Tone) {
    let pad = 18.0;
    let cy = rect.y + rect.h / 2.0;
    let inner = rect.w - 2.0 * pad;
    let badge = (rect.h - 24.0).clamp(28.0, 48.0);
    let mut size = type_scale::VALUE.min(rect.h - 28.0).max(24.0);
    while size > 24.0 && value_unit_width(canvas, value, unit, size) > inner {
        size -= 2.0;
    }
    let vw = value_unit_width(canvas, value, unit, size).min(inner);
    let with_badge = badge + 12.0 + vw <= inner;
    let vx = rect.x + rect.w - pad - vw;
    draw_value_unit(canvas, theme, vx, cy - size * 0.5, value, unit, size, vw);
    if with_badge {
        let bx = rect.x + pad;
        draw_icon_badge(canvas, theme, bx, cy - badge / 2.0, badge, icon, tone, true);
        let tx = bx + badge + 12.0;
        let tw = vx - 12.0 - tx;
        if tw >= 48.0 {
            canvas.draw_text_fit(title, tx, cy - 12.0, type_scale::LABEL, true, theme.text_muted, TextAlign::Left, tw);
        }
    }
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
    // The tiles share the card's height: a card the grid made taller than
    // the tiles need leaves no empty band at the bottom
    let tile_h = (rect.y + rect.h - PAD - y - TILE_GAP * (sensors.len().div_ceil(cols) as f32 - 1.0)) / sensors.len().div_ceil(cols).max(1) as f32;

    for (idx, item) in sensors.iter().enumerate() {
        let (r, c) = (idx / cols, idx % cols);
        let cell = Rect::new(rect.x + PAD + c as f32 * (col_w + TILE_GAP), y + r as f32 * (tile_h + TILE_GAP), col_w, tile_h);
        canvas.fill_rounded_rect(cell.x, cell.y, cell.w, cell.h, 16.0, theme.surface_alt);

        let icon = item.icon.as_deref().unwrap_or("zap");
        let state = all_states.get(&item.entity_id);
        let low_battery = icon == "battery" && state.and_then(|s| s.state.parse::<f32>().ok()).map(|v| v < 20.0).unwrap_or(false);
        let tone = if low_battery { theme.heat } else { tone_for_icon(theme, icon) };
        let value = state.map(format_entity_state).unwrap_or_else(|| "--".to_string());
        let unit = item.unit.clone().or_else(|| state.and_then(|s| s.unit_of_measurement())).unwrap_or_default();

        // A short or narrow tile: a small icon beside the title, no badge
        if tile_h < 112.0 || col_w < 170.0 {
            let max = if large_value { 60.0 } else { type_scale::VALUE };
            draw_compact_tile(canvas, theme, cell, 12.0, icon, tone, &item.title, &value, &unit, max);
            continue;
        }

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

        // A tall tile gets a larger value
        let value_size = if large_value || tile_h >= 170.0 {
            fit_value_size(canvas, &value, &unit, if large_value { 60.0 } else { 56.0 }, cell.w - 2.0 * ip)
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
