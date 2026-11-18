use crate::config::ChartKind;
use crate::display::{Canvas, TextAlign};
use crate::ha::models::EntityState;
use crate::ui::layout::{InteractiveTarget, Rect, UiAction};
use crate::ui::theme::Theme;
use crate::ui::widgets::{draw_card, format_number, type_scale, PAD};

/// Attribute holding the statistics series of an entity, keyed per chart:
/// `{"day:14:change": [[start_ms, value], ...]}`. The Home Assistant client
/// fills it, so the widget needs no network access of its own.
pub const STATS_ATTR: &str = "__stats";

/// Key of one chart in [`STATS_ATTR`], for a range of `hours`.
pub fn stat_key(period: &str, hours: u32, stat: &str) -> String {
    format!("{}:{}h:{}", period, hours, stat)
}

/// The range of a chart in hours: `hours` when given, else `days`, and
/// never more than [`window_days`] allows.
pub fn window_hours(days: u32, hours: Option<u32>) -> u32 {
    match hours {
        Some(h) => h.clamp(1, window_days(u32::MAX) * 24),
        None => window_days(days) * 24,
    }
}

/// A chart never looks further back than ten years: beyond that the window is
/// meaningless, and a huge value makes the date arithmetic overflow.
pub fn window_days(days: u32) -> u32 {
    days.clamp(1, 3660)
}

fn series(state: Option<&EntityState>, key: &str) -> Vec<(f64, f64)> {
    state
        .and_then(|s| s.attributes.get(STATS_ATTR))
        .and_then(|v| v.get(key))
        .and_then(|v| serde_json::from_value::<Vec<(f64, f64)>>(v.clone()).ok())
        .unwrap_or_default()
}

/// A period starts at local midnight in Home Assistant, so the label has to be
/// read in the configured timezone too; otherwise every date is a day early.
fn day_label(ms: f64, offset: chrono::FixedOffset) -> String {
    time_label(ms, offset, "%d/%m")
}

fn time_label(ms: f64, offset: chrono::FixedOffset, format: &str) -> String {
    chrono::DateTime::from_timestamp_millis(ms as i64)
        .map(|d| d.with_timezone(&offset).format(format).to_string())
        .unwrap_or_default()
}

/// The label of a period's start: a time for periods shorter than a day
fn period_label(ms: f64, offset: chrono::FixedOffset, period: &str) -> String {
    match period {
        "5minute" | "hour" => time_label(ms, offset, "%H:%M"),
        _ => day_label(ms, offset),
    }
}

/// Chart of Home Assistant statistics (a Lovelace `statistics-graph`): bars,
/// horizontal bars or a line.
#[allow(clippy::too_many_arguments)]
pub fn render_statistics_card(
    canvas: &mut Canvas,
    theme: &Theme,
    rect: Rect,
    entity_id: &str,
    state: Option<&EntityState>,
    custom_title: Option<&str>,
    period: &str,
    hours: u32,
    stat: &str,
    custom_unit: Option<&str>,
    chart_kind: ChartKind,
    targets: &mut Vec<InteractiveTarget>,
) {
    draw_card(canvas, theme, rect, None);

    let title = custom_title
        .map(str::to_string)
        .or_else(|| state.map(|s| s.friendly_name()))
        .unwrap_or_else(|| entity_id.to_string());
    let unit = custom_unit
        .map(str::to_string)
        .or_else(|| state.and_then(|s| s.unit_of_measurement()))
        .unwrap_or_default();
    let points = series(state, &stat_key(period, hours, stat));
    let with_unit = |v: f64| format!("{}{}", format_number(&format!("{:.1}", v)), if unit.is_empty() { String::new() } else { format!(" {}", unit) });

    let inner_w = rect.w - 2.0 * PAD;
    canvas.draw_text_fit(&title, rect.x + PAD, rect.y + PAD, type_scale::BODY, true, theme.text_muted, TextAlign::Left, inner_w);

    let labels_h = if chart_kind == ChartKind::Hbar { 0.0 } else { 34.0 };
    let chart_top = rect.y + PAD + 44.0;
    let chart_h = (rect.y + rect.h - PAD - labels_h - chart_top).max(24.0);
    let chart = Rect::new(rect.x + PAD, chart_top, inner_w, chart_h);

    if points.is_empty() {
        canvas.draw_text_fit(
            "--",
            chart.x + chart.w / 2.0,
            chart.y + chart.h / 2.0 - type_scale::VALUE / 2.0,
            type_scale::VALUE,
            true,
            theme.text_muted,
            TextAlign::Center,
            chart.w,
        );
        targets.push(InteractiveTarget { rect, action: UiAction::Refresh });
        return;
    }

    // The scale spans zero: a drop can make a period negative (a meter reset),
    // and then the bar goes below the axis instead of disappearing.
    let lo = points.iter().map(|(_, v)| *v).fold(0.0f64, f64::min);
    let hi = points.iter().map(|(_, v)| *v).fold(0.0f64, f64::max);
    let span = (hi - lo).max(1e-9);
    let offset = theme.utc_offset;
    let n = points.len();

    if chart_kind == ChartKind::Hbar {
        // One row per period, the newest at the bottom; as many as fit
        let rows = ((chart.h / 22.0).floor() as usize).clamp(1, n);
        let shown = &points[n - rows..];
        let row_h = chart.h / rows as f32;
        let size = (row_h * 0.7).clamp(14.0, 20.0);
        let label_w = shown.iter().map(|(t, _)| canvas.measure_text(&period_label(*t, offset, period), size, false).0).fold(0.0f32, f32::max) + 10.0;
        let value_w = shown.iter().map(|(_, v)| canvas.measure_text(&with_unit(*v), size, true).0).fold(0.0f32, f32::max) + 10.0;
        let bars_x = chart.x + label_w;
        let bars_w = (chart.w - label_w - value_w).max(10.0);
        let zero_x = bars_x + bars_w * (-lo / span) as f32;
        canvas.fill_rect(zero_x, chart.y, 2.0, chart.h, theme.border);
        for (k, (t, v)) in shown.iter().enumerate() {
            let y = chart.y + k as f32 * row_h;
            let bar_h = (row_h * 0.62).max(4.0);
            let w = (bars_w * (v.abs() / span) as f32).max(2.0);
            let x = if *v >= 0.0 { zero_x } else { zero_x - w };
            canvas.fill_rounded_rect(x, y + (row_h - bar_h) / 2.0, w, bar_h, (bar_h / 4.0).min(6.0), theme.accent.solid);
            let ty = y + (row_h - size) / 2.0 - 1.0;
            canvas.draw_text(&period_label(*t, offset, period), chart.x, ty, size, false, theme.text_muted, TextAlign::Left);
            canvas.draw_text(&with_unit(*v), chart.x + chart.w, ty, size, true, theme.text, TextAlign::Right);
        }
        targets.push(InteractiveTarget { rect, action: UiAction::Refresh });
        return;
    }

    // The largest value, in the corner the title leaves free.
    let extreme = if hi.abs() >= lo.abs() { hi } else { lo };
    canvas.draw_text_fit(&with_unit(extreme), rect.x + rect.w - PAD, rect.y + PAD + 4.0, type_scale::LABEL, false, theme.text_muted, TextAlign::Right, inner_w / 2.0);

    let zero_y = chart.y + chart.h * (hi / span) as f32;
    if chart_kind == ChartKind::Line {
        // The line spans its own lowest to highest value
        let values: Vec<f32> = points.iter().map(|(_, v)| *v as f32).collect();
        if values.len() >= 2 {
            canvas.draw_sparkline(chart.x, chart.y, chart.w, chart.h, &values, theme.accent.solid, 3.0, Some(theme.accent.tint));
        } else {
            let y = chart.y + chart.h * ((hi - values[0] as f64) / span) as f32;
            canvas.fill_circle(chart.x + chart.w / 2.0, y, 5.0, theme.accent.solid);
        }
    } else {
        canvas.fill_rect(chart.x, zero_y, chart.w, 2.0, theme.border);
        // One bar per period. The step follows from the width, so the bars can
        // never grow past the chart however many periods there are.
        let step = chart.w / n as f32;
        let gap = (step * 0.25).min(4.0);
        let bar_w = (step - gap).max(0.5);
        let radius = (bar_w / 4.0).min(6.0);
        for (i, (_, value)) in points.iter().enumerate() {
            let h = (chart.h * (value.abs() / span) as f32).max(2.0);
            let y = if *value >= 0.0 { zero_y - h } else { zero_y };
            let x = chart.x + i as f32 * step;
            canvas.fill_rounded_rect(x, y, bar_w, h, radius, theme.accent.solid);
        }
    }

    // Only the ends are labelled: the title says what the range is.
    let label_y = chart.y + chart.h + 8.0;
    canvas.draw_text(&period_label(points[0].0, offset, period), chart.x, label_y, 20.0, false, theme.text_muted, TextAlign::Left);
    canvas.draw_text(&period_label(points[n - 1].0, offset, period), chart.x + chart.w, label_y, 20.0, false, theme.text_muted, TextAlign::Right);

    targets.push(InteractiveTarget { rect, action: UiAction::Refresh });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ThemeConfig;

    fn state_with(points: &[(f64, f64)]) -> EntityState {
        let mut attrs = std::collections::HashMap::new();
        attrs.insert(
            STATS_ATTR.to_string(),
            serde_json::json!({
                stat_key("day", 14 * 24, "change"): points,
                stat_key("day", 365 * 24, "change"): points,
                stat_key("day", 400 * 24, "change"): points,
                stat_key("week", 60 * 24, "change"): [[0.0, 1.0]],
            }),
        );
        EntityState {
            entity_id: "sensor.gas".into(),
            state: "1234.5".into(),
            attributes: attrs,
            last_changed: None,
            last_updated: None,
        }
    }

    #[test]
    fn reads_the_series_of_its_own_chart() {
        let state = state_with(&[(0.0, 1.0), (86_400_000.0, 2.5)]);
        assert_eq!(series(Some(&state), &stat_key("day", 336, "change")), vec![(0.0, 1.0), (86_400_000.0, 2.5)]);
        assert_eq!(series(Some(&state), &stat_key("week", 1440, "change")), vec![(0.0, 1.0)]);
        assert!(series(Some(&state), &stat_key("month", 8760, "mean")).is_empty());
        assert!(series(None, &stat_key("day", 336, "change")).is_empty());
        assert_eq!(stat_key("hour", 4, "mean"), "hour:4h:mean");
    }

    #[test]
    fn dates_are_read_in_the_configured_timezone() {
        let midnight = chrono::FixedOffset::east_opt(0).unwrap();
        let amsterdam = chrono::FixedOffset::east_opt(3600).unwrap();
        // 2025-01-01T23:30:00Z is still 1 January in UTC, 2 January at +01:00
        let ms = 1_735_774_200_000.0;
        assert_eq!(day_label(ms, midnight), "01/01");
        assert_eq!(day_label(ms, amsterdam), "02/01");
    }

    #[test]
    fn a_huge_range_is_clamped() {
        // The key is built the same way on both sides, so clamping it here keeps
        // the fetched series and the widget in agreement.
        assert_eq!(window_hours(4_000_000_000, None), 3660 * 24);
        assert_eq!(window_hours(0, None), 24);
        assert_eq!(window_hours(14, Some(4)), 4);
        assert_eq!(window_hours(14, Some(0)), 1);
        assert_eq!(window_hours(14, Some(u32::MAX)), 3660 * 24);
        assert_eq!(window_hours(14, None), 336);
    }

    #[test]
    fn a_long_series_stays_inside_the_card() {
        let theme = Theme::from_config(&ThemeConfig::default());
        let (w, h) = (700u32, 400u32);
        let rect = Rect::new(20.0, 20.0, 660.0, 360.0);
        let mut targets = Vec::new();
        let many: Vec<(f64, f64)> = (0..400).map(|i| (i as f64 * 86_400_000.0, (i % 9) as f64)).collect();
        let kinds = [ChartKind::Bar, ChartKind::Hbar, ChartKind::Line];
        for (points, kind) in [many.clone(), many.iter().map(|(t, v)| (*t, -v)).collect()].into_iter().flat_map(|p| kinds.map(|k| (p.clone(), k))) {
            let state = state_with(&points);
            let mut canvas = Canvas::new(w, h);
            render_statistics_card(&mut canvas, &theme, rect, "sensor.gas", Some(&state), Some("Gas"), "day", 400 * 24, "change", Some("m³"), kind, &mut targets);
            assert!(canvas.raw_rgba_data().iter().any(|b| *b != 0));
            let data = canvas.raw_rgba_data();
            let mut outside = 0;
            for y in 0..h {
                for x in 0..w {
                    let inside = (x as f32) >= rect.x
                        && (y as f32) >= rect.y
                        && (x as f32) < rect.x + rect.w
                        && (y as f32) < rect.y + rect.h;
                    if inside {
                        continue;
                    }
                    let i = ((y * w + x) * 4) as usize;
                    if data[i..i + 4].iter().any(|b| *b != 0) {
                        outside += 1;
                    }
                }
            }
            assert_eq!(outside, 0, "{:?}: {} pixels outside the card", kind, outside);
        }
    }

    #[test]
    fn draws_bars_without_panicking_on_extremes() {
        let theme = Theme::from_config(&ThemeConfig::default());
        let mut canvas = Canvas::new(700, 400);
        let rect = Rect::new(20.0, 20.0, 660.0, 360.0);
        let mut targets = Vec::new();
        // Empty, a single point, zeroes and a long series all render
        for points in [vec![], vec![(0.0, 0.0)], vec![(0.0, 0.0), (86_400_000.0, 0.0)]] {
            let state = state_with(&points);
            for kind in [ChartKind::Bar, ChartKind::Hbar, ChartKind::Line] {
                render_statistics_card(&mut canvas, &theme, rect, "sensor.gas", Some(&state), Some("Gas"), "day", 14 * 24, "change", None, kind, &mut targets);
            }
        }
        let many: Vec<(f64, f64)> = (0..365).map(|i| (i as f64 * 86_400_000.0, (i % 7) as f64)).collect();
        let state = state_with(&many);
        render_statistics_card(&mut canvas, &theme, rect, "sensor.gas", Some(&state), None, "day", 365 * 24, "change", Some("m³"), ChartKind::Bar, &mut targets);
        // Nothing tappable but the refresh target, and the canvas is not blank
        assert_eq!(targets.len(), 10);
        assert!(canvas.raw_rgba_data().iter().any(|b| *b != 0));
    }
}