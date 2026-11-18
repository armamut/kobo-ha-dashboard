//! How a total splits between entities: a ring with a legend.

use crate::config::SensorItemConfig;
use crate::display::{Canvas, TextAlign};
use crate::ha::models::EntityState;
use crate::ui::layout::{InteractiveTarget, Rect, UiAction};
use crate::ui::theme::{Theme, Tone};
use crate::ui::widgets::{draw_card, format_number, type_scale, PAD};
use std::collections::HashMap;

pub const TITLE_H: f32 = 40.0;
const ROW_H: f32 = 34.0;

/// Colours of the slices, in order; they repeat after these
fn tones(theme: &Theme) -> [Tone; 6] {
    [theme.accent, theme.cool, theme.eco, theme.heat, theme.media, theme.warm]
}

/// Each entity's value (negatives and non-numbers count as nothing)
pub fn shares(items: &[SensorItemConfig], states: &HashMap<String, EntityState>) -> Vec<f64> {
    items
        .iter()
        .map(|i| states.get(&i.entity_id).and_then(|s| s.state.parse::<f64>().ok()).filter(|v| v.is_finite()).unwrap_or(0.0).max(0.0))
        .collect()
}

pub fn render_breakdown_card(
    canvas: &mut Canvas,
    theme: &Theme,
    rect: Rect,
    custom_title: Option<&str>,
    items: &[SensorItemConfig],
    states: &HashMap<String, EntityState>,
    targets: &mut Vec<InteractiveTarget>,
) {
    draw_card(canvas, theme, rect, None);
    let mut top = rect.y + PAD;
    if let Some(title) = custom_title {
        canvas.draw_text_fit(title, rect.x + PAD, top - 2.0, type_scale::BODY, true, theme.text_muted, TextAlign::Left, rect.w - 2.0 * PAD);
        top += TITLE_H;
    }
    let area = Rect::new(rect.x + PAD, top, rect.w - 2.0 * PAD, rect.y + rect.h - PAD - top);
    let values = shares(items, states);
    let total: f64 = values.iter().sum();
    let unit = items
        .iter()
        .find_map(|i| i.unit.clone().or_else(|| states.get(&i.entity_id).and_then(|s| s.unit_of_measurement())))
        .unwrap_or_default();
    let with_unit = |v: f64| {
        let n = format_number(&format!("{:.1}", v));
        if unit.is_empty() { n } else { format!("{} {}", n, unit) }
    };

    // Legend beside the ring on a wide card, below it on a narrow one
    let side = area.w >= area.h * 1.3;
    let d = if side { area.h.min(area.w * 0.45) } else { (area.h - items.len() as f32 * ROW_H - 12.0).min(area.w) }.max(60.0);
    let (cx, cy) = if side { (area.x + d / 2.0, area.y + area.h / 2.0) } else { (area.x + area.w / 2.0, area.y + d / 2.0) };
    let r = d / 2.0;
    let tones = tones(theme);
    if total > 0.0 {
        let mut a = 0.0f32;
        for (k, v) in values.iter().enumerate() {
            let sweep = (*v / total) as f32 * std::f32::consts::TAU;
            if sweep > 0.0 {
                canvas.fill_ring_segment(cx, cy, r, r * 0.6, a, a + sweep, tones[k % tones.len()].solid);
                // A thin gap between slices keeps them apart in grey too
                if values.iter().filter(|v| **v > 0.0).count() > 1 {
                    canvas.fill_ring_segment(cx, cy, r + 1.0, r * 0.6 - 1.0, a - 0.012, a + 0.012, theme.surface);
                }
            }
            a += sweep;
        }
    } else {
        canvas.fill_ring_segment(cx, cy, r, r * 0.6, 0.0, std::f32::consts::TAU, theme.surface_alt);
    }
    // The total in the middle
    let inner = r * 1.1;
    let total_s = with_unit(total);
    let mut size = (r * 0.38).clamp(18.0, type_scale::VALUE);
    while size > 16.0 && canvas.measure_text(&total_s, size, true).0 > inner {
        size -= 2.0;
    }
    canvas.draw_text_fit(&total_s, cx, cy - size / 2.0 - 2.0, size, true, theme.text, TextAlign::Center, inner);

    let (lx, ly, lw) = if side { (area.x + d + 24.0, cy - items.len() as f32 * ROW_H / 2.0, area.w - d - 24.0) } else { (area.x, area.y + d + 12.0, area.w) };
    let size = type_scale::LABEL;
    let pct_of = |v: f64| if total > 0.0 { format!("{:.0}%", v / total * 100.0) } else { "--".into() };
    // Values and shares where every row has room for both, else the shares alone
    let both = values.iter().all(|v| canvas.measure_text(&format!("{}  {}", with_unit(*v), pct_of(*v)), size, true).0 <= lw * 0.5);
    for (k, (item, v)) in items.iter().zip(&values).enumerate() {
        let y = ly + k as f32 * ROW_H;
        if y + ROW_H > rect.y + rect.h - PAD + 6.0 {
            break;
        }
        canvas.fill_rounded_rect(lx, y + 6.0, 16.0, 16.0, 4.0, tones[k % tones.len()].solid);
        let right = if both { format!("{}  {}", with_unit(*v), pct_of(*v)) } else { pct_of(*v) };
        let rw = canvas.draw_text_fit(&right, lx + lw, y + 3.0, size, true, theme.text, TextAlign::Right, lw * 0.5);
        canvas.draw_text_fit(&item.title, lx + 26.0, y + 3.0, size, false, theme.text_muted, TextAlign::Left, lw - 26.0 - rw - 10.0);
    }
    targets.push(InteractiveTarget { rect, action: UiAction::Refresh });
}

/// Natural and smallest height at `width` for `count` entities
pub fn breakdown_heights(width: f32, count: usize, has_title: bool) -> (f32, f32) {
    let t = 2.0 * PAD + if has_title { TITLE_H } else { 0.0 };
    let legend = count.max(1) as f32 * ROW_H;
    if width >= 440.0 {
        // Side by side: the ring as tall as the legend, at least 160
        (t + legend.max(200.0), t + legend.max(130.0))
    } else {
        (t + 200.0 + 12.0 + legend, t + 110.0 + 12.0 + legend)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_that_are_not_amounts_count_as_nothing() {
        let item = |id: &str| SensorItemConfig { entity_id: id.into(), title: id.into(), icon: None, unit: None };
        let mut states = HashMap::new();
        for (id, v) in [("a", "120"), ("b", "-5"), ("c", "unavailable")] {
            states.insert(id.to_string(), EntityState { entity_id: id.into(), state: v.into(), attributes: HashMap::new(), last_changed: None, last_updated: None });
        }
        assert_eq!(shares(&[item("a"), item("b"), item("c"), item("d")], &states), vec![120.0, 0.0, 0.0, 0.0]);
    }
}
