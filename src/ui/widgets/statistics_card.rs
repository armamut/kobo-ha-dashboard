use crate::display::{Canvas, TextAlign};
use crate::ha::models::EntityState;
use crate::ui::layout::{InteractiveTarget, Rect, UiAction};
use crate::ui::theme::Theme;
use crate::ui::widgets::{draw_card, format_number, type_scale, PAD};

/// Attribute holding the statistics series of an entity, keyed per chart:
/// `{"day:14:change": [[start_ms, value], ...]}`. The Home Assistant client
/// fills it, so the widget needs no network access of its own.
pub const STATS_ATTR: &str = "__stats";

/// Key of one chart in [`STATS_ATTR`].
pub fn stat_key(period: &str, days: u32, stat: &str) -> String {
    format!("{}:{}:{}", period, window_days(days), stat)
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
    chrono::DateTime::from_timestamp_millis(ms as i64)
        .map(|d| d.with_timezone(&offset).format("%d/%m").to_string())
        .unwrap_or_default()
}

/// Bar chart of Home Assistant statistics (a Lovelace `statistics-graph`).
pub fn render_statistics_card(
    canvas: &mut Canvas,
    theme: &Theme,
    rect: Rect,
    entity_id: &str,
    state: Option<&EntityState>,
    custom_title: Option<&str>,
    period: &str,
    days: u32,
    stat: &str,
    custom_unit: Option<&str>,
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
    let points = series(state, &stat_key(period, days, stat));

    let inner_w = rect.w - 2.0 * PAD;
    canvas.draw_text_fit(&title, rect.x + PAD, rect.y + PAD, type_scale::BODY, true, theme.text_muted, TextAlign::Left, inner_w);

    let chart_top = rect.y + PAD + 44.0;
    let chart_h = (rect.y + rect.h - PAD - 34.0 - chart_top).max(24.0);
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
    let zero_y = chart.y + chart.h * (hi / span) as f32;
    // The largest value, in the corner the title leaves free.
    let extreme = if hi.abs() >= lo.abs() { hi } else { lo };
    let peak = format!("{}{}", format_number(&format!("{:.1}", extreme)), if unit.is_empty() { String::new() } else { format!(" {}", unit) });
    canvas.draw_text_fit(&peak, rect.x + rect.w - PAD, rect.y + PAD + 4.0, type_scale::LABEL, false, theme.text_muted, TextAlign::Right, inner_w / 2.0);

    // Axis at zero, then one bar per period. The step follows from the width,
    // so the bars can never grow past the chart however many periods there are.
    canvas.fill_rect(chart.x, zero_y, chart.w, 2.0, theme.border);
    let n = points.len();
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

    // Only the ends are labelled: the title says what the range is.
    let label_y = chart.y + chart.h + 8.0;
    let offset = theme.utc_offset;
    canvas.draw_text(&day_label(points[0].0, offset), chart.x, label_y, 20.0, false, theme.text_muted, TextAlign::Left);
    canvas.draw_text(&day_label(points[n - 1].0, offset), chart.x + chart.w, label_y, 20.0, false, theme.text_muted, TextAlign::Right);

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
            serde_json::json!({ "day:14:change": points, "week:60:change": [[0.0, 1.0]] }),
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
        assert_eq!(series(Some(&state), "day:14:change"), vec![(0.0, 1.0), (86_400_000.0, 2.5)]);
        assert_eq!(series(Some(&state), "week:60:change"), vec![(0.0, 1.0)]);
        assert!(series(Some(&state), "month:365:mean").is_empty());
        assert!(series(None, "day:14:change").is_empty());
        assert_eq!(stat_key("day", 14, "change"), "day:14:change");
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
        assert_eq!(stat_key("day", 4_000_000_000, "change"), "day:3660:change");
        assert_eq!(stat_key("day", 0, "change"), "day:1:change");
        assert_eq!(window_days(14), 14);
    }

    #[test]
    fn a_long_series_stays_inside_the_card() {
        let theme = Theme::from_config(&ThemeConfig::default());
        let (w, h) = (700u32, 400u32);
        let rect = Rect::new(20.0, 20.0, 660.0, 360.0);
        let mut targets = Vec::new();
        let many: Vec<(f64, f64)> = (0..400).map(|i| (i as f64 * 86_400_000.0, (i % 9) as f64)).collect();
        for points in [many.clone(), many.iter().map(|(t, v)| (*t, -v)).collect()] {
            let state = state_with(&points);
            let mut canvas = Canvas::new(w, h);
            render_statistics_card(&mut canvas, &theme, rect, "sensor.gas", Some(&state), Some("Gas"), "day", 400, "change", Some("m³"), &mut targets);
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
            assert_eq!(outside, 0, "{} pixels outside the card", outside);
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
            render_statistics_card(&mut canvas, &theme, rect, "sensor.gas", Some(&state), Some("Gas"), "day", 14, "change", None, &mut targets);
        }
        let many: Vec<(f64, f64)> = (0..365).map(|i| (i as f64 * 86_400_000.0, (i % 7) as f64)).collect();
        let state = state_with(&many);
        render_statistics_card(&mut canvas, &theme, rect, "sensor.gas", Some(&state), None, "day", 365, "change", Some("m³"), &mut targets);
        // Nothing tappable but the refresh target, and the canvas is not blank
        assert_eq!(targets.len(), 4);
        assert!(canvas.raw_rgba_data().iter().any(|b| *b != 0));
    }
}