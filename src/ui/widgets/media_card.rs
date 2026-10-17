use crate::display::{Canvas, TextAlign};
use crate::i18n;
use crate::ha::models::EntityState;
use crate::ui::layout::{InteractiveTarget, Rect, UiAction};
use crate::ui::theme::Theme;
use crate::ui::widgets::{draw_card, draw_round_button, type_scale, PAD};

pub fn render_media_card(
    canvas: &mut Canvas,
    theme: &Theme,
    rect: Rect,
    entity_id: &str,
    state: Option<&EntityState>,
    custom_title: Option<&str>,
    targets: &mut Vec<InteractiveTarget>,
) {
    let title = custom_title
        .map(|s| s.to_string())
        .or_else(|| state.map(|s| s.friendly_name()))
        .unwrap_or_else(|| i18n::t("media.default_title"));
    let player_state = state.map(|s| s.state.as_str()).unwrap_or("unavailable");
    let is_playing = player_state == "playing";
    let has_media = matches!(player_state, "playing" | "paused" | "buffering");

    let track = state
        .and_then(|s| s.get_attr_str("media_title"))
        .filter(|_| has_media)
        .map(str::to_string)
        .unwrap_or_else(|| {
            i18n::t(match player_state {
                "unavailable" => "state.unavailable",
                "off" => "state.off",
                _ => "state.not_playing",
            })
        });
    let artist = state
        .and_then(|s| s.get_attr_str("media_artist").or_else(|| s.get_attr_str("app_name")))
        .filter(|_| has_media)
        .unwrap_or("");

    let tone = theme.media;
    draw_card(canvas, theme, rect, is_playing.then_some(tone));

    // Artwork placeholder tile
    let art = (rect.h - 2.0 * PAD).min(124.0);
    let ax = rect.x + PAD;
    let ay = rect.y + (rect.h - art) / 2.0;
    let (art_bg, art_fg) = if is_playing { (tone.solid, tone.on_solid) } else { (theme.surface_alt, tone.solid) };
    canvas.fill_rounded_rect(ax, ay, art, art, 18.0, art_bg);
    let icon = art * 0.46;
    canvas.draw_icon("music", ax + (art - icon) / 2.0, ay + (art - icon) / 2.0, icon, art_fg, 2.2);

    // Controls on the right, vertically centred
    let small = 76.0f32.min(rect.h - 2.0 * PAD);
    let big = 92.0f32.min(rect.h - 2.0 * PAD);
    let cy = rect.y + rect.h / 2.0;
    let right = rect.x + rect.w - PAD;
    let next_rect = Rect::new(right - small, cy - small / 2.0, small, small);
    let play_rect = Rect::new(next_rect.x - 16.0 - big, cy - big / 2.0, big, big);
    let prev_rect = Rect::new(play_rect.x - 16.0 - small, cy - small / 2.0, small, small);

    draw_round_button(canvas, theme, prev_rect, "skip_previous", tone, false);
    draw_round_button(canvas, theme, play_rect, if is_playing { "pause" } else { "play" }, tone, true);
    draw_round_button(canvas, theme, next_rect, "skip_next", tone, false);

    // Text block between artwork and controls
    let tx = ax + art + 22.0;
    let tw = prev_rect.x - 20.0 - tx;
    let block_h = 22.0 + 8.0 + 30.0 + if artist.is_empty() { 0.0 } else { 8.0 + 24.0 };
    let mut ty = cy - block_h / 2.0 - 2.0;
    canvas.draw_text_fit(&title, tx, ty, type_scale::LABEL, false, theme.text_muted, TextAlign::Left, tw);
    ty += 30.0;
    canvas.draw_text_fit(&track, tx, ty, 30.0, true, theme.text, TextAlign::Left, tw);
    if !artist.is_empty() {
        ty += 38.0;
        canvas.draw_text_fit(artist, tx, ty, type_scale::BODY, false, theme.text_muted, TextAlign::Left, tw);
    }

    for (r, action) in [
        (prev_rect, UiAction::MediaPrevious(entity_id.to_string())),
        (play_rect, UiAction::MediaPlayPause(entity_id.to_string())),
        (next_rect, UiAction::MediaNext(entity_id.to_string())),
    ] {
        targets.push(InteractiveTarget { rect: r.inset(-6.0), action });
    }
}
