//! How far along something is: a print, a wash, a download.

use crate::display::{Canvas, TextAlign};
use crate::ha::models::EntityState;
use crate::i18n;
use crate::ui::layout::{InteractiveTarget, Rect, UiAction};
use crate::ui::theme::Theme;
use crate::ui::widgets::{draw_card, draw_icon_badge, format_entity_state, type_scale, PAD};

/// Cards shorter than this show one row: icon, title, percentage, bar.
const ROW_BELOW: f32 = 170.0;

/// Fraction done, 0 to 1, or `None` while the state is not a number.
pub fn fraction(state: Option<&EntityState>, max: f32) -> Option<f32> {
    let v = state?.state.parse::<f32>().ok()?;
    Some((v / max.max(f32::EPSILON)).clamp(0.0, 1.0))
}

/// The second entity as text: a time of day for a timestamp (when a print
/// ends), else its value and unit (time left); nothing while unavailable.
pub fn secondary_text(state: Option<&EntityState>, offset: chrono::FixedOffset) -> Option<String> {
    let s = state?;
    if matches!(s.state.as_str(), "unavailable" | "unknown" | "") {
        return None;
    }
    if let Ok(t) = chrono::DateTime::parse_from_rfc3339(&s.state) {
        return Some(t.with_timezone(&offset).format("%H:%M").to_string());
    }
    let unit = s.unit_of_measurement().unwrap_or_default();
    let value = format_entity_state(s);
    Some(if unit.is_empty() { value } else { format!("{} {}", value, unit) })
}

#[allow(clippy::too_many_arguments)]
pub fn render_progress_card(
    canvas: &mut Canvas,
    theme: &Theme,
    rect: Rect,
    entity_id: &str,
    state: Option<&EntityState>,
    secondary: Option<&EntityState>,
    custom_title: Option<&str>,
    custom_icon: Option<&str>,
    max: f32,
    targets: &mut Vec<InteractiveTarget>,
) {
    let title = custom_title
        .map(str::to_string)
        .or_else(|| state.map(|s| s.friendly_name()))
        .unwrap_or_else(|| entity_id.to_string());
    let icon = custom_icon.unwrap_or("progress");
    let frac = fraction(state, max);
    let pct = frac
        .map(|f| i18n::tf("meta.percent", &[("value", &format!("{:.0}", f * 100.0))]))
        .unwrap_or_else(|| "--".into());
    let extra = secondary_text(secondary, theme.utc_offset);
    let tone = theme.accent;
    let running = frac.is_some_and(|f| f > 0.0 && f < 1.0);

    draw_card(canvas, theme, rect, None);
    let bar = |canvas: &mut Canvas, x: f32, y: f32, w: f32, h: f32| {
        canvas.draw_progress_bar(x, y, w, h, h / 2.0, frac.unwrap_or(0.0), theme.surface, tone.solid);
        canvas.stroke_rounded_rect(x, y, w, h, h / 2.0, 1.5, tone.solid);
    };

    if rect.h < ROW_BELOW {
        // Icon | title ........ 42% · 1 h left
        //      | ==========-----------------
        let p = 18.0;
        let icon_size = 28.0;
        canvas.draw_icon(icon, rect.x + p, rect.y + (rect.h - icon_size) / 2.0, icon_size, tone.solid, 2.2);
        let x = rect.x + p + icon_size + 12.0;
        let w = rect.x + rect.w - p - x;
        // The time only where it fits whole, next to the percentage
        let right = match &extra {
            Some(e) if canvas.measure_text(&format!("{} · {}", pct, e), type_scale::BODY, true).0 <= w * 0.6 => format!("{} · {}", pct, e),
            _ => pct.clone(),
        };
        let line_y = rect.y + rect.h / 2.0 - 26.0;
        let rw = canvas.draw_text_fit(
            &right,
            rect.x + rect.w - p,
            line_y,
            type_scale::BODY,
            true,
            theme.text,
            TextAlign::Right,
            w * 0.6,
        );
        canvas.draw_text_fit(
            &title,
            x,
            line_y + 1.0,
            type_scale::LABEL,
            true,
            theme.text_muted,
            TextAlign::Left,
            w - rw - 10.0,
        );
        bar(canvas, x, rect.y + rect.h / 2.0 + 8.0, w, 12.0);
    } else {
        let badge = if rect.w < 240.0 { 40.0 } else { 52.0 };
        let (bx, by) = (rect.x + PAD, rect.y + PAD);
        draw_icon_badge(canvas, theme, bx, by, badge, icon, tone, running);
        let tx = bx + badge + 14.0;
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

        let bar_h = 16.0;
        let bar_y = rect.y + rect.h - PAD - bar_h;
        // The percentage as large as the room above the bar allows
        let room = bar_y - 12.0 - (by + badge + 8.0);
        let size = room.clamp(type_scale::VALUE * 0.8, type_scale::HERO);
        let vy = bar_y - 12.0 - size;
        let w = canvas.draw_text_fit(&pct, bx, vy, size, true, theme.text, TextAlign::Left, rect.w - 2.0 * PAD);
        if let Some(e) = &extra {
            let ex = bx + w + 14.0;
            let ew = rect.x + rect.w - PAD - ex;
            if ew > 40.0 {
                canvas.draw_text_fit(e, ex, vy + size - 30.0, type_scale::BODY, false, theme.text_muted, TextAlign::Left, ew);
            }
        }
        bar(canvas, bx, bar_y, rect.w - 2.0 * PAD, bar_h);
    }
    targets.push(InteractiveTarget {
        rect,
        action: UiAction::Refresh,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn state(value: &str, unit: Option<&str>) -> EntityState {
        let mut attributes = HashMap::new();
        if let Some(u) = unit {
            attributes.insert("unit_of_measurement".to_string(), serde_json::json!(u));
        }
        EntityState {
            entity_id: "sensor.x".into(),
            state: value.into(),
            attributes,
            last_changed: None,
            last_updated: None,
        }
    }

    #[test]
    fn the_fraction_is_clamped_and_numbers_only() {
        assert_eq!(fraction(Some(&state("42", None)), 100.0), Some(0.42));
        assert_eq!(fraction(Some(&state("150", None)), 100.0), Some(1.0));
        assert_eq!(fraction(Some(&state("3", None)), 6.0), Some(0.5));
        assert_eq!(fraction(Some(&state("unavailable", None)), 100.0), None);
        assert_eq!(fraction(None, 100.0), None);
    }

    #[test]
    fn a_timestamp_reads_as_a_time_of_day() {
        let plus3 = chrono::FixedOffset::east_opt(3 * 3600).unwrap();
        assert_eq!(secondary_text(Some(&state("2026-11-18T19:05:00+00:00", None)), plus3).as_deref(), Some("22:05"));
        assert_eq!(secondary_text(Some(&state("unknown", None)), plus3), None);
        assert!(secondary_text(Some(&state("80", Some("min"))), plus3).unwrap().ends_with("min"));
    }
}
