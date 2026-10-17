use crate::display::Canvas;
use crate::i18n;
use crate::ha::models::EntityState;
use crate::ui::layout::{InteractiveTarget, Rect, UiAction};
use crate::ui::theme::Theme;
use crate::ui::widgets::light_card::render_toggle_card;

pub fn render_switch_card(
    canvas: &mut Canvas,
    theme: &Theme,
    rect: Rect,
    entity_id: &str,
    state: Option<&EntityState>,
    custom_title: Option<&str>,
    custom_subtitle: Option<&str>,
    custom_icon: Option<&str>,
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
        "state.running"
    } else {
        "state.off"
    });
    let subtitle = match custom_subtitle {
        Some(s) if !unavailable => format!("{} · {}", status, s),
        _ => status.to_string(),
    };

    let default_icon = match entity_id.split('.').next() {
        Some("fan") => "fan",
        Some("lock") => "lock",
        Some("cover") => "door",
        _ => "power",
    };

    render_toggle_card(
        canvas,
        theme,
        rect,
        theme.eco,
        is_on,
        custom_icon.unwrap_or(default_icon),
        &title,
        &subtitle,
        None,
    );

    targets.push(InteractiveTarget {
        rect,
        action: UiAction::ToggleEntity(entity_id.to_string()),
    });
}
