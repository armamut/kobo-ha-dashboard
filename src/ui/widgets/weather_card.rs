use crate::display::{Canvas, TextAlign};
use crate::i18n;
use crate::ha::models::{EntityState, ForecastItem};
use crate::ui::layout::{InteractiveTarget, Rect, UiAction};
use crate::ui::theme::Theme;
use crate::ui::widgets::{draw_card, type_scale, PAD};
use chrono::Datelike;

pub fn condition_label(condition: &str) -> String {
    if condition == "unavailable" {
        return i18n::t("state.unavailable");
    }
    i18n::try_t(&format!("weather.condition.{}", condition)).unwrap_or_else(|| condition.to_string())
}

fn day_label(datetime: &str) -> String {
    match datetime.get(..10).and_then(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok()) {
        Some(date) if date == chrono::Local::now().date_naive() => i18n::t("date.today"),
        Some(date) => i18n::nth("date.weekdays_short", date.weekday().num_days_from_monday() as usize),
        None => datetime.to_string(),
    }
}

/// Cards at least this tall show the forecast strip.
pub const FORECAST_FROM: f32 = 280.0;

pub fn render_weather_card(
    canvas: &mut Canvas,
    theme: &Theme,
    rect: Rect,
    _entity_id: &str,
    state: Option<&EntityState>,
    custom_title: Option<&str>,
    forecast_days: usize,
    targets: &mut Vec<InteractiveTarget>,
) {
    draw_card(canvas, theme, rect, None);

    let condition = state.map(|s| s.state.as_str()).unwrap_or("unavailable");
    let temp = state.and_then(|s| s.get_attr_f32("temperature"));
    let unit = state.and_then(|s| s.get_attr_str("temperature_unit")).unwrap_or("°C");
    let humidity = state.and_then(|s| s.get_attr_f32("humidity"));
    let wind = state.and_then(|s| s.get_attr_f32("wind_speed"));
    let wind_unit = match state.and_then(|s| s.get_attr_str("wind_speed_unit")) {
        Some("km/h") | None => i18n::t("weather.wind_kmh"),
        Some(other) => other.to_string(),
    };

    let forecast: Vec<ForecastItem> = state
        .and_then(|s| s.attributes.get("forecast"))
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    // The forecast strip needs a tall card; a short one shows today only,
    // centred in the card.
    let strip_h = 22.0 + 6.0 + 46.0 + 6.0 + 28.0;
    let with_strip = rect.h >= FORECAST_FROM;
    let head_h = if with_strip { rect.h - strip_h - PAD - 16.0 } else { rect.h };

    let wide = rect.w >= 640.0;
    let x0 = rect.x + PAD;
    let art = (if wide { 116.0 } else { 96.0f32 }).min(head_h - 2.0 * PAD).min((rect.w - 2.0 * PAD) * 0.4);
    let y0 = if with_strip { rect.y + PAD } else { rect.y + (rect.h - art) / 2.0 };

    canvas.draw_weather_art(condition, x0, y0, art, theme.weather);

    let tx = x0 + art + if art < 80.0 { 14.0 } else { 22.0 };
    let max_w = rect.x + rect.w - PAD - tx;
    let temp_str = temp.map(|t| format!("{:.0}°", t)).unwrap_or_else(|| "--".to_string());
    let unit_w = if unit != "°C" { canvas.measure_text(unit, 26.0, false).0 + 4.0 } else { 0.0 };
    let mut temp_size = if wide { type_scale::HERO } else { 72.0f32 }.min(art * 0.8).max(40.0);
    while temp_size > 40.0 && canvas.measure_text(&temp_str, temp_size, true).0 + unit_w > max_w {
        temp_size -= 4.0;
    }
    // Lines below the temperature (condition, then humidity and wind), as
    // many as fit. Without the strip, the block is centred in the card.
    let max_lines = if wide { 1 } else { 2 };
    let block = |lines: usize| temp_size * 0.92 + lines as f32 * 30.0;
    let (temp_y, lines) = if with_strip {
        (y0 - 6.0, max_lines)
    } else {
        let lines = (0..=max_lines).rev().find(|&n| block(n) <= rect.h - 24.0).unwrap_or(0);
        (rect.y + (rect.h - block(lines)) / 2.0 - 6.0, lines)
    };
    let bottom = if with_strip { rect.y + head_h } else { rect.y + rect.h - 12.0 };
    let text_top = temp_y + 6.0 + temp_size * 0.92;
    canvas.draw_text(&temp_str, tx, temp_y, temp_size, true, theme.text, TextAlign::Left);
    let temp_w = canvas.measure_text(&temp_str, temp_size, true).0;
    if unit != "°C" {
        canvas.draw_text(unit, tx + temp_w + 4.0, temp_y + 12.0, 26.0, false, theme.text_muted, TextAlign::Left);
    }

    let mut metrics: Vec<(&str, String)> = Vec::new();
    if let Some(h) = humidity {
        metrics.push(("droplet", i18n::tf("meta.percent", &[("value", &format!("{:.0}", h))])));
    }
    if let Some(w) = wind {
        metrics.push(("wind", format!("{:.0} {}", w, wind_unit)));
    }

    let _ = custom_title;
    let cond_text = condition_label(condition);
    let cond_text = cond_text.as_str();

    if wide {
        let metrics_w = 200.0;
        if lines >= 1 {
            canvas.draw_text_fit(cond_text, tx, text_top + 4.0, type_scale::TITLE, true, theme.text, TextAlign::Left, max_w - metrics_w);
        }
        // Metrics column, right aligned, as far as it fits
        let mut my = rect.y + (if with_strip { PAD } else { (rect.h - 44.0 * metrics.len() as f32) / 2.0 }) + 6.0;
        for (icon, text) in &metrics {
            if my + 30.0 > bottom {
                break;
            }
            let right = rect.x + rect.w - PAD;
            let tw = canvas.measure_text(text, type_scale::BODY, true).0;
            canvas.draw_text(text, right, my, type_scale::BODY, true, theme.text, TextAlign::Right);
            canvas.draw_icon(icon, right - tw - 38.0, my - 1.0, 28.0, theme.text_muted, 2.0);
            my += 44.0;
        }
    } else {
        if lines >= 1 {
            canvas.draw_text_fit(cond_text, tx, text_top, 24.0, true, theme.text, TextAlign::Left, max_w);
        }
        if lines >= 2 {
            let line = metrics.iter().map(|(_, t)| t.as_str()).collect::<Vec<_>>().join("  ·  ");
            canvas.draw_text_fit(&line, tx, text_top + 32.0, type_scale::LABEL, false, theme.text_muted, TextAlign::Left, max_w);
        }
    }

    if with_strip {
        // Forecast strip anchored to the bottom of the card
        let strip_y = rect.y + rect.h - PAD - strip_h;
        let divider_y = strip_y - 16.0;
        canvas.draw_line(x0, divider_y, rect.x + rect.w - PAD, divider_y, 2.0, theme.border);

        if forecast.is_empty() {
            canvas.draw_text(
                &i18n::t("weather.no_forecast"),
                rect.x + rect.w / 2.0,
                strip_y + strip_h / 2.0 - 12.0,
                type_scale::BODY,
                false,
                theme.text_faint,
                TextAlign::Center,
            );
        } else {
            // As many days as fit, each at least 76 px wide
            let fit = ((rect.w - 2.0 * PAD) / 76.0).floor().max(1.0) as usize;
            let count = forecast.len().min(forecast_days.max(1)).min(fit);
            let col_w = (rect.w - 2.0 * PAD) / count as f32;
            for (i, item) in forecast.iter().take(count).enumerate() {
                let cx = x0 + col_w * (i as f32 + 0.5);
                let day = day_label(&item.datetime);
                let day_color = if i == 0 { theme.text } else { theme.text_muted };
                canvas.draw_text(&day, cx, strip_y, type_scale::LABEL, true, day_color, TextAlign::Center);
                canvas.draw_weather_art(&item.condition, cx - 23.0, strip_y + 28.0, 46.0, theme.weather);

                let (hi_size, lo_size, sep) = if col_w < 130.0 { (22.0, 20.0, 5.0) } else { (26.0, 24.0, 10.0) };
                let hi = format!("{:.0}°", item.temperature);
                let lo = item.templow.map(|l| format!("{:.0}°", l)).filter(|_| col_w >= 96.0);
                let hi_w = canvas.measure_text(&hi, hi_size, true).0;
                let lo_w = lo.as_ref().map(|l| canvas.measure_text(l, lo_size, false).0 + sep).unwrap_or(0.0);
                let start = cx - (hi_w + lo_w) / 2.0;
                let ty = strip_y + 80.0 + (26.0 - hi_size) * 0.6;
                canvas.draw_text(&hi, start, ty, hi_size, true, theme.text, TextAlign::Left);
                if let Some(lo) = lo {
                    canvas.draw_text(&lo, start + hi_w + sep, ty + 1.0, lo_size, false, theme.text_faint, TextAlign::Left);
                }
            }
        }
    }

    // Tapping weather card triggers refresh
    targets.push(InteractiveTarget {
        rect,
        action: UiAction::Refresh,
    });
}
