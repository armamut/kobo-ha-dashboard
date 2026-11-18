//! Blinds, shutters, a garage door: open, stop and close, and how far open.

use crate::display::{Canvas, TextAlign};
use crate::ha::models::EntityState;
use crate::i18n;
use crate::ui::layout::{InteractiveTarget, Rect, UiAction};
use crate::ui::theme::Theme;
use crate::ui::widgets::{draw_card, draw_icon_badge, draw_round_button, type_scale, PAD};

/// Narrower than this, the buttons go on a row of their own below the name
pub const COVER_WIDE: f32 = 440.0;

const BUTTONS: [(&str, &str); 3] = [("chevron_up", "open_cover"), ("stop", "stop_cover"), ("chevron_down", "close_cover")];

/// The icon for a cover without one of its own, from its device class
pub fn cover_icon(state: Option<&EntityState>) -> &'static str {
    match state.and_then(|s| s.get_attr_str("device_class")) {
        Some("garage") => "garage",
        Some("door") | Some("gate") => "door",
        Some("window") => "window",
        _ => "blinds",
    }
}

#[allow(clippy::too_many_arguments)]
pub fn render_cover_card(
    canvas: &mut Canvas,
    theme: &Theme,
    rect: Rect,
    entity_id: &str,
    state: Option<&EntityState>,
    custom_title: Option<&str>,
    custom_icon: Option<&str>,
    targets: &mut Vec<InteractiveTarget>,
) {
    let title = custom_title
        .map(str::to_string)
        .or_else(|| state.map(|s| s.friendly_name()))
        .unwrap_or_else(|| entity_id.to_string());
    let raw = state.map(|s| s.state.as_str()).unwrap_or("unavailable");
    let open = matches!(raw, "open" | "opening");
    let position = state.and_then(|s| s.get_attr_f32("current_position"));
    let label = if raw == "unavailable" {
        i18n::t("state.unavailable")
    } else {
        i18n::entity_state_label("cover", state.and_then(|s| s.get_attr_str("device_class")), raw)
    };
    let subtitle = match position {
        Some(p) if raw != "unavailable" => format!("{} · {}", label, i18n::tf("meta.percent", &[("value", &format!("{:.0}", p))])),
        _ => label,
    };
    let icon = custom_icon.unwrap_or_else(|| cover_icon(state));
    let tone = theme.cool;
    draw_card(canvas, theme, rect, None);

    let gap = 12.0;
    let stacked = rect.w < COVER_WIDE && rect.h >= 150.0;
    let p = if rect.h < 110.0 { 18.0 } else { PAD };
    let buttons: Vec<Rect>;
    if stacked {
        // Name on top, the three buttons along the bottom
        let badge = 52.0;
        draw_icon_badge(canvas, theme, rect.x + p, rect.y + p, badge, icon, tone, open);
        let tx = rect.x + p + badge + 14.0;
        let tw = rect.x + rect.w - p - tx;
        canvas.draw_text_fit(&title, tx, rect.y + p + 1.0, type_scale::BODY, true, theme.text, TextAlign::Left, tw);
        canvas.draw_text_fit(
            &subtitle,
            tx,
            rect.y + p + 30.0,
            type_scale::LABEL,
            false,
            theme.text_muted,
            TextAlign::Left,
            tw,
        );
        let btn = ((rect.w - 2.0 * p - 2.0 * gap) / 3.0).min(64.0).min(rect.h - 2.0 * p - badge - 16.0).max(36.0);
        let total = 3.0 * btn + 2.0 * gap;
        let x0 = rect.x + (rect.w - total) / 2.0;
        let y = rect.y + rect.h - p - btn;
        buttons = (0..3).map(|i| Rect::new(x0 + i as f32 * (btn + gap), y, btn, btn)).collect();
        if let Some(pos) = position.filter(|_| rect.h >= 220.0) {
            let by = rect.y + p + badge + (y - (rect.y + p + badge)) / 2.0 - 6.0;
            canvas.draw_progress_bar(rect.x + p, by, rect.w - 2.0 * p, 12.0, 6.0, pos / 100.0, theme.surface, tone.solid);
            canvas.stroke_rounded_rect(rect.x + p, by, rect.w - 2.0 * p, 12.0, 6.0, 1.5, tone.solid);
        }
    } else {
        // [badge] name ........ (^) (■) (v), the position below if there is room
        let with_bar = position.is_some() && rect.h >= 150.0;
        let row_h = if with_bar {
            (rect.h - 2.0 * p - 26.0).min(72.0)
        } else {
            (rect.h - 2.0 * p).min(72.0)
        };
        let btn = row_h.clamp(36.0, 64.0);
        let total = 3.0 * btn + 2.0 * gap;
        // On a tall card the row and the bar sit in the middle, together
        let block = row_h + if with_bar { 26.0 } else { 0.0 };
        let top = rect.y + (rect.h - block) / 2.0;
        let cy = top + row_h / 2.0;
        let x0 = rect.x + rect.w - p - total;
        let fits = x0 - (rect.x + p) >= 0.0;
        let x0 = if fits { x0 } else { rect.x + (rect.w - total) / 2.0 };
        buttons = (0..3).map(|i| Rect::new(x0 + i as f32 * (btn + gap), cy - btn / 2.0, btn, btn)).collect();
        let room = x0 - 16.0 - (rect.x + p);
        let badge = row_h.min(56.0);
        if room >= badge {
            draw_icon_badge(canvas, theme, rect.x + p, cy - badge / 2.0, badge, icon, tone, open);
            let tx = rect.x + p + badge + 14.0;
            let tw = x0 - 16.0 - tx;
            if tw >= 60.0 {
                let with_sub = row_h >= 60.0;
                let ty = if with_sub { cy - 28.0 } else { cy - 14.0 };
                canvas.draw_text_fit(&title, tx, ty, type_scale::BODY, true, theme.text, TextAlign::Left, tw);
                if with_sub {
                    canvas.draw_text_fit(&subtitle, tx, ty + 32.0, type_scale::LABEL, false, theme.text_muted, TextAlign::Left, tw);
                }
            }
        }
        if let (true, Some(pos)) = (with_bar, position) {
            let by = top + row_h + 14.0;
            canvas.draw_progress_bar(rect.x + p, by, rect.w - 2.0 * p, 12.0, 6.0, pos / 100.0, theme.surface, tone.solid);
            canvas.stroke_rounded_rect(rect.x + p, by, rect.w - 2.0 * p, 12.0, 6.0, 1.5, tone.solid);
        }
    }

    for (r, (icon, service)) in buttons.iter().zip(BUTTONS) {
        draw_round_button(canvas, theme, *r, icon, tone, false);
        targets.push(InteractiveTarget {
            rect: r.inset(-6.0),
            action: UiAction::CallService("cover".into(), service.into(), entity_id.to_string()),
        });
    }
}
