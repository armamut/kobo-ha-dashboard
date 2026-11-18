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

    // Artwork and the skip buttons need width; play/pause always stays.
    let inner_h = rect.h - 2.0 * PAD;
    // Skip buttons only while the track keeps 160 px beside them
    let with_skip = rect.w - 2.0 * PAD - (76.0f32.min(inner_h) * 2.0 + 32.0 + 92.0f32.min(inner_h)) - 20.0 >= 160.0;
    let art = inner_h.min(124.0);
    // Room the controls leave; the artwork only when the track still fits
    let controls_w = if with_skip { 76.0f32.min(inner_h) * 2.0 + 32.0 } else { 0.0 } + 92.0f32.min(inner_h);
    let with_art = rect.w - 2.0 * PAD - controls_w - 20.0 - art - 22.0 >= 140.0;
    let ax = rect.x + PAD;
    if with_art {
        let ay = rect.y + (rect.h - art) / 2.0;
        let (art_bg, art_fg) = if is_playing { (tone.solid, tone.on_solid) } else { (theme.surface_alt, tone.solid) };
        canvas.fill_rounded_rect(ax, ay, art, art, 18.0, art_bg);
        let icon = art * 0.46;
        canvas.draw_icon("music", ax + (art - icon) / 2.0, ay + (art - icon) / 2.0, icon, art_fg, 2.2);
    }

    // Controls on the right, vertically centred
    let small = 76.0f32.min(inner_h);
    let big = 92.0f32.min(inner_h);
    let cy = rect.y + rect.h / 2.0;
    let right = rect.x + rect.w - PAD;
    let next_rect = Rect::new(right - small, cy - small / 2.0, small, small);
    let play_rect = if with_skip {
        Rect::new(next_rect.x - 16.0 - big, cy - big / 2.0, big, big)
    } else {
        Rect::new(right - big, cy - big / 2.0, big, big)
    };
    let prev_rect = Rect::new(play_rect.x - 16.0 - small, cy - small / 2.0, small, small);

    draw_round_button(canvas, theme, play_rect, if is_playing { "pause" } else { "play" }, tone, true);
    if with_skip {
        draw_round_button(canvas, theme, prev_rect, "skip_previous", tone, false);
        draw_round_button(canvas, theme, next_rect, "skip_next", tone, false);
    }

    // Text block between artwork and controls: the track always, the
    // player's name and the artist when there is height for them
    let tx = if with_art { ax + art + 22.0 } else { ax };
    let tw = if with_skip { prev_rect.x } else { play_rect.x } - 20.0 - tx;
    if tw >= 60.0 {
        let with_title = inner_h >= 64.0;
        let with_artist = !artist.is_empty() && inner_h >= 100.0;
        let block_h = 30.0 + if with_title { 30.0 } else { 0.0 } + if with_artist { 38.0 } else { 0.0 };
        let mut ty = cy - block_h / 2.0 - 2.0;
        if with_title {
            canvas.draw_text_fit(&title, tx, ty, type_scale::LABEL, false, theme.text_muted, TextAlign::Left, tw);
            ty += 30.0;
        }
        canvas.draw_text_fit(&track, tx, ty, 30.0, true, theme.text, TextAlign::Left, tw);
        if with_artist {
            ty += 38.0;
            canvas.draw_text_fit(artist, tx, ty, type_scale::BODY, false, theme.text_muted, TextAlign::Left, tw);
        }
    }

    let mut buttons = vec![(play_rect, UiAction::MediaPlayPause(entity_id.to_string()))];
    if with_skip {
        buttons.push((prev_rect, UiAction::MediaPrevious(entity_id.to_string())));
        buttons.push((next_rect, UiAction::MediaNext(entity_id.to_string())));
    }
    for (r, action) in buttons {
        targets.push(InteractiveTarget { rect: r.inset(-6.0), action });
    }
}
