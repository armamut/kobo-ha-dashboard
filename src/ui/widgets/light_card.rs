use crate::display::{Canvas, TextAlign};
use crate::i18n;
use crate::ha::models::EntityState;
use crate::ui::layout::{InteractiveTarget, Rect, UiAction};
use crate::ui::theme::Theme;
use crate::ui::widgets::{draw_card, draw_icon_badge, draw_toggle, type_scale, PAD};

pub fn render_light_card(
    canvas: &mut Canvas,
    theme: &Theme,
    rect: Rect,
    entity_id: &str,
    state: Option<&EntityState>,
    custom_title: Option<&str>,
    custom_subtitle: Option<&str>,
    custom_icon: Option<&str>,
    show_brightness: bool,
    targets: &mut Vec<InteractiveTarget>,
) {
    let is_on = state.map(|s| s.is_on()).unwrap_or(false);
    let unavailable = state.map(|s| s.state == "unavailable").unwrap_or(true);
    let title = custom_title
        .map(|s| s.to_string())
        .or_else(|| state.map(|s| s.friendly_name()))
        .unwrap_or_else(|| entity_id.to_string());

    let status = i18n::t(if unavailable {
        "state.unavailable"
    } else if is_on {
        "state.on"
    } else {
        "state.off"
    });
    // The toggle already shows on/off; only spell the state out when there is
    // no custom subtitle or the device is unreachable.
    let subtitle = match custom_subtitle {
        Some(s) if !unavailable => s.to_string(),
        _ => status.to_string(),
    };

    let icon_name = match custom_icon {
        Some("lightbulb") | None if is_on => "lightbulb_on",
        Some(icon) => icon,
        None => "lightbulb",
    };

    let tone = theme.warm;
    render_toggle_card(
        canvas,
        theme,
        rect,
        tone,
        is_on,
        icon_name,
        &title,
        &subtitle,
        if show_brightness && is_on { state.and_then(|s| s.brightness_percent()) } else { None },
    );

    targets.push(InteractiveTarget {
        rect,
        action: UiAction::ToggleEntity(entity_id.to_string()),
    });
}

/// Shared body for light and switch cards.
pub(crate) fn render_toggle_card(
    canvas: &mut Canvas,
    theme: &Theme,
    rect: Rect,
    tone: crate::ui::theme::Tone,
    is_on: bool,
    icon: &str,
    title: &str,
    subtitle: &str,
    level_pct: Option<u8>,
) {
    draw_card(canvas, theme, rect, is_on.then_some(tone));

    let badge = 68.0;
    let toggle_w = 76.0;
    let bx = rect.x + PAD;
    let bar_h = 14.0;
    // With a brightness bar the header sits at the top; otherwise it is centred.
    let by = if level_pct.is_some() { rect.y + PAD } else { rect.y + (rect.h - badge) / 2.0 };
    draw_icon_badge(canvas, theme, bx, by, badge, icon, if is_on { tone } else { theme.idle }, is_on);

    let toggle_h = toggle_w * 0.56;
    draw_toggle(canvas, theme, rect.x + rect.w - PAD - toggle_w, by + (badge - toggle_h) / 2.0, toggle_w, is_on, tone);

    let tx = bx + badge + 18.0;
    let text_w = rect.x + rect.w - PAD - toggle_w - 14.0 - tx;
    // Step down one size before resorting to an ellipsis.
    let title_size = if canvas.measure_text(title, type_scale::TITLE, true).0 <= text_w { type_scale::TITLE } else { 24.0 };
    canvas.draw_text_fit(title, tx, by + 2.0 + (type_scale::TITLE - title_size) * 0.6, title_size, true, theme.text, TextAlign::Left, text_w);
    let sub_color = if is_on { theme.text } else { theme.text_muted };
    canvas.draw_text_fit(subtitle, tx, by + 40.0, type_scale::LABEL, false, sub_color, TextAlign::Left, text_w);

    if let Some(pct) = level_pct {
        let label_w = 72.0;
        let bar_y = rect.y + rect.h - PAD - bar_h - 2.0;
        let bar_w = rect.w - 2.0 * PAD - label_w;
        canvas.draw_progress_bar(bx, bar_y, bar_w, bar_h, bar_h / 2.0, pct as f32 / 100.0, theme.surface, tone.solid);
        canvas.stroke_rounded_rect(bx, bar_y, bar_w, bar_h, bar_h / 2.0, 1.5, tone.solid);
        canvas.draw_text(
            &i18n::tf("meta.percent", &[("value", &pct.to_string())]),
            rect.x + rect.w - PAD,
            bar_y + bar_h / 2.0 - 12.0,
            type_scale::BODY,
            true,
            theme.text,
            TextAlign::Right,
        );
    }
}
