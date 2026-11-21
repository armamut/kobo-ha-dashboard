//! Robot vacuum: current state, battery level, and action buttons (start/pause, dock, stop).

use crate::display::{Canvas, TextAlign};
use crate::ha::models::EntityState;
use crate::i18n;
use crate::ui::layout::{InteractiveTarget, Rect, UiAction};
use crate::ui::theme::Theme;
use crate::ui::widgets::{draw_card, draw_icon_badge, draw_round_button, type_scale, PAD};

/// Narrower than this, the buttons go on a row of their own below the name.
pub const VACUUM_WIDE: f32 = 440.0;

const DEFAULT_COMMANDS: &[&str] = &["start_pause", "return_home"];

/// Map a configured vacuum command name to `(icon, ha_service)`.
pub fn command_button(cmd: &str, cleaning: bool) -> Option<(&'static str, &'static str)> {
    match cmd {
        "start_pause" => Some((if cleaning { "pause" } else { "play" }, "start_pause")),
        "start" => Some(("play", "start")),
        "pause" => Some(("pause", "pause")),
        "stop" => Some(("stop", "stop")),
        "return_home" | "return_to_base" => Some(("home", "return_to_base")),
        "locate" => Some(("bell", "locate")),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
pub fn render_vacuum_card(
    canvas: &mut Canvas,
    theme: &Theme,
    rect: Rect,
    entity_id: &str,
    state: Option<&EntityState>,
    custom_title: Option<&str>,
    custom_icon: Option<&str>,
    commands: &[String],
    targets: &mut Vec<InteractiveTarget>,
) {
    let title = custom_title
        .map(str::to_string)
        .or_else(|| state.map(|s| s.friendly_name()))
        .unwrap_or_else(|| entity_id.to_string());
    let raw = state.map(|s| s.state.as_str()).unwrap_or("unavailable");
    let cleaning = raw == "cleaning";
    let active = matches!(raw, "cleaning" | "returning");
    let battery = state.and_then(|s| s.get_attr_f32("battery_level"));
    let label = if raw == "unavailable" {
        i18n::t("state.unavailable")
    } else {
        i18n::entity_state_label("vacuum", None, raw)
    };
    let subtitle = match battery {
        Some(b) if raw != "unavailable" => {
            format!("{} · {}", label, i18n::tf("meta.percent", &[("value", &format!("{:.0}", b))]))
        }
        _ => label,
    };
    let icon = custom_icon.unwrap_or("vacuum");
    let tone = theme.eco;
    draw_card(canvas, theme, rect, None);

    let resolved: Vec<(&'static str, &'static str)> = if commands.is_empty() {
        DEFAULT_COMMANDS.iter().filter_map(|c| command_button(c, cleaning)).collect()
    } else {
        commands.iter().filter_map(|c| command_button(c.as_str(), cleaning)).collect()
    };
    let n = resolved.len().max(1) as f32;
    let gap = 12.0;
    let stacked = rect.w < VACUUM_WIDE && rect.h >= 150.0;
    let p = if rect.h < 110.0 { 18.0 } else { PAD };

    let buttons: Vec<Rect>;
    if stacked {
        let badge = 52.0;
        draw_icon_badge(canvas, theme, rect.x + p, rect.y + p, badge, icon, tone, active);
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
        let btn = ((rect.w - 2.0 * p - (n - 1.0) * gap) / n)
            .min(64.0)
            .min(rect.h - 2.0 * p - badge - 16.0)
            .max(36.0);
        let total = n * btn + (n - 1.0) * gap;
        let x0 = rect.x + (rect.w - total) / 2.0;
        let y = rect.y + rect.h - p - btn;
        buttons = (0..resolved.len())
            .map(|i| Rect::new(x0 + i as f32 * (btn + gap), y, btn, btn))
            .collect();
    } else {
        let row_h = (rect.h - 2.0 * p).min(72.0);
        let btn = row_h.clamp(36.0, 64.0);
        let total = n * btn + (n - 1.0) * gap;
        let cy = rect.y + rect.h / 2.0;
        let x0 = rect.x + rect.w - p - total;
        let fits = x0 - (rect.x + p) >= 0.0;
        let x0 = if fits { x0 } else { rect.x + (rect.w - total) / 2.0 };
        buttons = (0..resolved.len())
            .map(|i| Rect::new(x0 + i as f32 * (btn + gap), cy - btn / 2.0, btn, btn))
            .collect();
        let room = x0 - 16.0 - (rect.x + p);
        let badge = row_h.min(56.0);
        if room >= badge {
            draw_icon_badge(canvas, theme, rect.x + p, cy - badge / 2.0, badge, icon, tone, active);
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
    }

    for (r, (btn_icon, service)) in buttons.iter().zip(resolved) {
        let primary = service == "start_pause" && !cleaning;
        draw_round_button(canvas, theme, *r, btn_icon, tone, primary);
        targets.push(InteractiveTarget {
            rect: r.inset(-6.0),
            action: UiAction::CallService("vacuum".into(), service.into(), serde_json::json!({ "entity_id": entity_id })),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ThemeConfig;
    use crate::ui::theme::Theme;

    #[test]
    fn command_button_maps_start_pause_and_return_home() {
        assert_eq!(command_button("start_pause", false), Some(("play", "start_pause")));
        assert_eq!(command_button("start_pause", true), Some(("pause", "start_pause")));
        assert_eq!(command_button("return_home", false), Some(("home", "return_to_base")));
        assert_eq!(command_button("stop", false), Some(("stop", "stop")));
        assert_eq!(command_button("unknown_cmd", false), None);
    }

    #[test]
    fn vacuum_card_emits_vacuum_service_targets() {
        let mut canvas = Canvas::new(600, 200);
        let theme = Theme::from_config(&ThemeConfig::default());
        let mut targets = Vec::new();
        let state = EntityState {
            entity_id: "vacuum.roborock".into(),
            state: "docked".into(),
            attributes: [("battery_level".into(), serde_json::json!(98))].into_iter().collect(),
            last_changed: None,
            last_updated: None,
        };
        render_vacuum_card(
            &mut canvas,
            &theme,
            Rect::new(0.0, 0.0, 500.0, 150.0),
            "vacuum.roborock",
            Some(&state),
            Some("Roborock V"),
            None,
            &[],
            &mut targets,
        );
        assert_eq!(targets.len(), 2);
        match &targets[0].action {
            UiAction::CallService(d, s, data) => {
                assert_eq!(d, "vacuum");
                assert_eq!(s, "start_pause");
                assert_eq!(data["entity_id"], "vacuum.roborock");
            }
            other => panic!("expected CallService, got {:?}", other),
        }
        match &targets[1].action {
            UiAction::CallService(d, s, data) => {
                assert_eq!(d, "vacuum");
                assert_eq!(s, "return_to_base");
                assert_eq!(data["entity_id"], "vacuum.roborock");
            }
            other => panic!("expected CallService, got {:?}", other),
        }
    }
}
