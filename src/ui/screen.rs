use crate::config::{AppConfig, CardConfig, SceneItemConfig};
use crate::display::Canvas;
use crate::ha::models::EntityState;
use crate::ui::layout::{InteractiveTarget, Rect, UiAction};
use crate::ui::theme::Theme;
use crate::ui::widgets::climate_card::CLIMATE_WIDE;
use crate::ui::widgets::header::{read_device_status, HeaderStatus, SIDE_PAD};
use crate::ui::widgets::scene_card::SCENES_TITLE_H;
use crate::ui::widgets::sensor_card::{sensor_grid_columns, sensor_grid_height, TILE_GAP};
use crate::ui::widgets::*;
use chrono::{DateTime, Utc};
use std::collections::HashMap;

const GAP: f32 = 18.0;
const BOTTOM_PAD: f32 = 28.0;
/// Space kept at the bottom for the sleep-mode note
const SLEEP_NOTE_H: f32 = 124.0;
/// Width of the left column in landscape (weather + sensor grid)
const LANDSCAPE_LEFT_W: f32 = 440.0;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Span {
    Full,
    Half,
}

fn span_of(card: &CardConfig) -> Span {
    match card {
        CardConfig::Weather { .. }
        | CardConfig::ScenesRow { .. }
        | CardConfig::SensorGrid { .. }
        | CardConfig::MediaPlayer { .. } => Span::Full,
        _ => Span::Half,
    }
}

/// (natural, minimum) height of a card rendered at width `w`.
fn heights(card: &CardConfig, w: f32) -> (f32, f32) {
    match card {
        CardConfig::Weather { .. } => {
            if w >= 640.0 {
                (312.0, 300.0)
            } else {
                (318.0, 306.0)
            }
        }
        CardConfig::ScenesRow { title, .. } => {
            let t = if title.is_some() { SCENES_TITLE_H } else { 0.0 };
            (t + 124.0, t + 78.0)
        }
        CardConfig::Light { show_brightness, .. } => {
            if *show_brightness {
                (178.0, 150.0)
            } else {
                (150.0, 118.0)
            }
        }
        CardConfig::Switch { .. } => (150.0, 118.0),
        CardConfig::Climate { .. } => {
            if w >= CLIMATE_WIDE {
                (160.0, 136.0)
            } else {
                (236.0, 206.0)
            }
        }
        CardConfig::Sensor { show_graph, .. } => {
            if *show_graph {
                (250.0, 214.0)
            } else {
                (176.0, 150.0)
            }
        }
        CardConfig::SensorGrid { title, sensors } => {
            let nat = sensor_grid_height(w, sensors.len(), title.is_some());
            let rows = sensors.len().div_ceil(sensor_grid_columns(w, sensors.len())).max(1);
            (nat, nat - rows as f32 * 10.0)
        }
        CardConfig::MediaPlayer { .. } => (180.0, 140.0),
        CardConfig::Scene { .. } => (112.0, 86.0),
    }
}

/// Flow cards into rows of one full-width or two half-width cards, shrinking
/// rows (down to their minimum) to fit `avail_h`. A half card without a
/// partner is stretched to full width. Rows that still don't fit are dropped
/// rather than drawn on top of each other.
fn layout_zone(cards: &[&CardConfig], x: f32, y: f32, w: f32, avail_h: f32) -> Vec<(usize, Rect)> {
    let mut rows: Vec<Vec<usize>> = Vec::new();
    let mut pending: Option<usize> = None;
    for (i, card) in cards.iter().enumerate() {
        match span_of(card) {
            Span::Full => {
                if let Some(p) = pending.take() {
                    rows.push(vec![p]);
                }
                rows.push(vec![i]);
            }
            Span::Half => match pending.take() {
                Some(p) => rows.push(vec![p, i]),
                None => pending = Some(i),
            },
        }
    }
    if let Some(p) = pending {
        rows.push(vec![p]);
    }

    let half_w = (w - GAP) / 2.0;
    let row_heights: Vec<(f32, f32)> = rows
        .iter()
        .map(|row| {
            let item_w = if row.len() == 2 { half_w } else { w };
            row.iter()
                .map(|&i| heights(cards[i], item_w))
                .fold((0.0f32, 0.0f32), |(n, m), (cn, cm)| (n.max(cn), m.max(cm)))
        })
        .collect();

    let gaps = GAP * rows.len().saturating_sub(1) as f32;
    let natural: f32 = row_heights.iter().map(|h| h.0).sum::<f32>() + gaps;
    let slack: f32 = row_heights.iter().map(|h| h.0 - h.1).sum();
    let k = if natural > avail_h && slack > 0.0 { ((natural - avail_h) / slack).min(1.0) } else { 0.0 };

    let mut out = Vec::new();
    let mut shown = 0;
    let mut cur_y = y;
    for (row, (nat, min)) in rows.iter().zip(row_heights) {
        let h = nat - k * (nat - min);
        if cur_y + h > y + avail_h + 1.0 {
            let hidden: usize = rows.iter().skip(shown).map(|r| r.len()).sum();
            tracing::debug!("Page content does not fit the screen; {} card(s) hidden", hidden);
            break;
        }
        shown += 1;
        if row.len() == 2 {
            out.push((row[0], Rect::new(x, cur_y, half_w, h)));
            out.push((row[1], Rect::new(x + half_w + GAP, cur_y, half_w, h)));
        } else {
            out.push((row[0], Rect::new(x, cur_y, w, h)));
        }
        cur_y += h + GAP;
    }
    out
}

/// Header height when page tabs are shown (see `render_header`)
pub const HEADER_WITH_TABS: f32 = 206.0;
pub const HEADER_WITHOUT_TABS: f32 = 122.0;

fn zones_for<'a>(cards: &'a [CardConfig], cw: f32, ch: f32, top: f32) -> Vec<(&'a CardConfig, Rect)> {
    let avail_h = ch - top - BOTTOM_PAD;
    let content_w = cw - 2.0 * SIDE_PAD;
    let all: Vec<&CardConfig> = cards.iter().collect();
    if cw > ch {
        let (left, right): (Vec<&CardConfig>, Vec<&CardConfig>) =
            cards.iter().partition(|c| matches!(c, CardConfig::Weather { .. } | CardConfig::SensorGrid { .. }));
        if !left.is_empty() && !right.is_empty() {
            let right_x = SIDE_PAD + LANDSCAPE_LEFT_W + GAP;
            let right_w = content_w - LANDSCAPE_LEFT_W - GAP;
            let mut v: Vec<(&CardConfig, Rect)> = layout_zone(&left, SIDE_PAD, top, LANDSCAPE_LEFT_W, avail_h)
                .into_iter()
                .map(|(i, r)| (left[i], r))
                .collect();
            v.extend(layout_zone(&right, right_x, top, right_w, avail_h).into_iter().map(|(i, r)| (right[i], r)));
            return v;
        }
    }
    layout_zone(&all, SIDE_PAD, top, content_w, avail_h)
        .into_iter()
        .map(|(i, r)| (all[i], r))
        .collect()
}

/// How many of `cards` would not fit on a `w`x`h` screen.
pub fn hidden_card_count(cards: &[CardConfig], w: u32, h: u32, with_tabs: bool) -> usize {
    let top = if with_tabs { HEADER_WITH_TABS } else { HEADER_WITHOUT_TABS };
    cards.len() - zones_for(cards, w as f32, h as f32, top).len()
}

pub struct ScreenRenderer {
    pub theme: Theme,
    pub targets: Vec<InteractiveTarget>,
    /// Cards on the last rendered page that did not fit
    pub hidden_cards: usize,
    /// Time of the last successful Home Assistant refresh
    pub last_update: Option<DateTime<Utc>>,
    /// Frontlight state for the header button (None hides the button)
    pub frontlight: Option<bool>,
    /// While in sleep mode: the time of the next timed refresh, shown in a
    /// note at the bottom of the screen
    pub sleep_until: Option<String>,
}

impl ScreenRenderer {
    pub fn new(config: &AppConfig) -> Self {
        Self {
            theme: Theme::from_config(&config.theme),
            targets: Vec::new(),
            hidden_cards: 0,
            last_update: None,
            frontlight: None,
            sleep_until: None,
        }
    }

    pub fn render(
        &mut self,
        canvas: &mut Canvas,
        config: &AppConfig,
        active_page_idx: usize,
        states: &HashMap<String, EntityState>,
        is_live: bool,
    ) {
        self.targets.clear();
        self.hidden_cards = 0;
        canvas.clear(self.theme.bg);

        let (battery, charging, wifi_up) = read_device_status();
        let wifi_up = crate::ui::widgets::header::shown_wifi(wifi_up, self.sleep_until.is_some(), crate::power::wifi_switched_off());
        let status = HeaderStatus {
            is_live,
            is_demo: config.home_assistant.token.trim().is_empty(),
            last_update: self.last_update,
            battery,
            charging,
            wifi_up,
            frontlight: self.frontlight,
        };

        let header_bottom = render_header(
            canvas,
            &self.theme,
            &config.theme,
            &config.pages,
            active_page_idx,
            &status,
            &mut self.targets,
        );

        let Some(page) = config.pages.get(active_page_idx) else { return };

        // The sleep note replaces the page tabs (useless while asleep); with a
        // single page there are no tabs, so room is made at the bottom instead.
        let has_tabs = config.pages.len() > 1;
        let reserve = if self.sleep_until.is_some() && !has_tabs { SLEEP_NOTE_H } else { 0.0 };
        let placements = zones_for(&page.cards, canvas.width as f32, canvas.height as f32 - reserve, header_bottom);
        self.hidden_cards = page.cards.len() - placements.len();

        for (card, rect) in placements {
            self.render_card(canvas, card, rect, states);
        }

        if let Some(next) = self.sleep_until.clone() {
            let w = canvas.width as f32;
            let bar = if has_tabs {
                // Same place and size as the tab row drawn by the header
                Rect::new(SIDE_PAD, HEADER_WITH_TABS - 84.0, w - 2.0 * SIDE_PAD, 64.0)
            } else {
                let h = canvas.height as f32;
                Rect::new(SIDE_PAD, h - SLEEP_NOTE_H + 4.0, w - 2.0 * SIDE_PAD, SLEEP_NOTE_H - 28.0)
            };
            self.render_sleep_note(canvas, bar, &next);
            // Nothing is tappable while the device sleeps.
            self.targets.clear();
        }
    }

    fn render_sleep_note(&self, canvas: &mut Canvas, bar: Rect, next: &str) {
        use crate::display::TextAlign;
        use crate::i18n;
        let theme = &self.theme;
        canvas.fill_rounded_rect(bar.x, bar.y, bar.w, bar.h, bar.h / 2.0, theme.text);
        let icon = (bar.h * 0.5).min(40.0);
        let ix = bar.x + bar.h * 0.4;
        canvas.draw_icon("moon", ix, bar.y + (bar.h - icon) / 2.0, icon, theme.surface, 2.2);
        let tx = ix + icon + 18.0;
        let max_w = bar.x + bar.w - bar.h * 0.4 - tx;
        if bar.h < 80.0 {
            // One line: "Sleep mode · Next update 22:56" then the hint, muted
            let (line, size) = sleep_line(canvas, next, max_w, 24.0);
            let tw = canvas.draw_text_fit(&line, tx, bar.y + bar.h / 2.0 - size / 2.0, size, true, theme.surface, TextAlign::Left, max_w);
            let hint_x = tx + tw + 18.0;
            if hint_x + 120.0 < tx + max_w {
                let muted = theme.surface.mix(theme.text, 0.35);
                canvas.draw_text_fit(&i18n::t("sleep.wake_hint"), hint_x, bar.y + bar.h / 2.0 - 11.0, 22.0, false, muted, TextAlign::Left, tx + max_w - hint_x);
            }
        } else {
            let (line, size) = sleep_line(canvas, next, max_w, 26.0);
            canvas.draw_text_fit(&line, tx, bar.y + 16.0 + (26.0 - size) / 2.0, size, true, theme.surface, TextAlign::Left, max_w);
            canvas.draw_text_fit(&i18n::t("sleep.wake_hint"), tx, bar.y + 52.0, 22.0, false, theme.surface, TextAlign::Left, max_w);
        }
    }

    fn render_card(&mut self, canvas: &mut Canvas, card: &CardConfig, rect: Rect, states: &HashMap<String, EntityState>) {
        let theme = &self.theme;
        let targets = &mut self.targets;
        match card {
            CardConfig::Weather { entity_id, title, forecast_days } => {
                render_weather_card(canvas, theme, rect, entity_id, states.get(entity_id), title.as_deref(), *forecast_days, targets)
            }
            CardConfig::ScenesRow { title, scenes } => render_scenes_row(canvas, theme, rect, title.as_deref(), scenes, targets),
            CardConfig::SensorGrid { title, sensors } => {
                render_sensor_grid(canvas, theme, rect, title.as_deref(), sensors, states, targets)
            }
            CardConfig::MediaPlayer { entity_id, title } => {
                render_media_card(canvas, theme, rect, entity_id, states.get(entity_id), title.as_deref(), targets)
            }
            CardConfig::Climate { entity_id, title, step } => {
                render_climate_card(canvas, theme, rect, entity_id, states.get(entity_id), title.as_deref(), *step, targets)
            }
            CardConfig::Light { entity_id, title, subtitle, icon, show_brightness } => render_light_card(
                canvas,
                theme,
                rect,
                entity_id,
                states.get(entity_id),
                title.as_deref(),
                subtitle.as_deref(),
                icon.as_deref(),
                *show_brightness,
                targets,
            ),
            CardConfig::Switch { entity_id, title, subtitle, icon } => render_switch_card(
                canvas,
                theme,
                rect,
                entity_id,
                states.get(entity_id),
                title.as_deref(),
                subtitle.as_deref(),
                icon.as_deref(),
                targets,
            ),
            CardConfig::Sensor { entity_id, title, unit, icon, show_graph } => render_sensor_card(
                canvas,
                theme,
                rect,
                entity_id,
                states.get(entity_id),
                title.as_deref(),
                unit.as_deref(),
                icon.as_deref(),
                *show_graph,
                targets,
            ),
            CardConfig::Scene { entity_id, title, icon } => {
                let item = [SceneItemConfig {
                    entity_id: entity_id.clone(),
                    title: title.clone(),
                    icon: icon.clone().unwrap_or_else(|| "play".to_string()),
                }];
                render_scenes_row(canvas, theme, rect, None, &item, targets)
            }
        }
    }

    pub fn handle_touch(&self, x: f32, y: f32) -> Option<UiAction> {
        // Exact hit first (topmost wins), then the nearest target within a
        // small radius to forgive slightly-off finger taps.
        if let Some(t) = self.targets.iter().rev().find(|t| t.rect.contains(x, y)) {
            return Some(t.action.clone());
        }
        const SLOP: f32 = 24.0;
        self.targets
            .iter()
            .filter_map(|t| {
                let dx = (t.rect.x - x).max(x - (t.rect.x + t.rect.w)).max(0.0);
                let dy = (t.rect.y - y).max(y - (t.rect.y + t.rect.h)).max(0.0);
                let d = (dx * dx + dy * dy).sqrt();
                (d <= SLOP).then_some((d, t))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, t)| t.action.clone())
    }
}

/// The one-line sleep note: "Sleep mode · Next update 22:56", smaller if
/// needed, and on narrow screens (Kobo Aura, Glo) just "Next update 22:56";
/// the moon icon already says it's sleeping. The time is never cut off.
fn sleep_line(canvas: &Canvas, time: &str, max_w: f32, size: f32) -> (String, f32) {
    use crate::i18n;
    let small = size * 0.875;
    let next = i18n::tf("sleep.next", &[("time", time)]);
    let full = format!("{} · {}", i18n::t("sleep.title"), next);
    for (text, size) in [(&full, size), (&full, small), (&next, size), (&next, small)] {
        if canvas.measure_text(text, size, true).0 <= max_w {
            return (text.clone(), size);
        }
    }
    // Still too wide: shorten the words but keep the time, "Next upd… 22:56"
    let Some(at) = next.find(time) else { return (next, small) };
    let (label, rest) = next.split_at(at);
    let mut chars: Vec<char> = label.trim_end().chars().collect();
    while !chars.is_empty() {
        chars.pop();
        let text = format!("{}… {}", chars.iter().collect::<String>().trim_end(), rest);
        if canvas.measure_text(&text, small, true).0 <= max_w {
            return (text, small);
        }
    }
    (rest.to_string(), small)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sleep_note_shows_the_next_update_time_on_narrow_screens() {
        // Glo/Aura width with page tabs: "Modalità sospensione · Prossimo
        // aggiorname…" lost the time (issue #2).
        let _guard = crate::i18n::TEST_LANGUAGE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let canvas = Canvas::new(758, 200);
        // Same geometry as render_sleep_note: the one-line bar in place of the
        // tabs, and the two-line bar at the bottom of a single page
        let bar_w = 758.0 - 2.0 * SIDE_PAD;
        for (bar_h, size) in [(64.0f32, 24.0), (SLEEP_NOTE_H - 28.0, 26.0)] {
            let icon = (bar_h * 0.5).min(40.0);
            let max_w = bar_w - bar_h * 0.4 - (bar_h * 0.4 + icon + 18.0);
            for (code, _) in crate::i18n::BUILTIN {
                crate::i18n::set_language(code);
                let (line, size) = sleep_line(&canvas, "22:56", max_w, size);
                assert!(line.contains("22:56"), "{}: {}", code, line);
                assert!(canvas.measure_text(&line, size, true).0 <= max_w, "{}: {}", code, line);
            }
        }
        crate::i18n::set_language("en");
        // Wide screens keep the full line
        assert_eq!(sleep_line(&canvas, "22:56", 1000.0, 24.0), ("Sleep mode · Next update 22:56".to_string(), 24.0));
        // Very narrow: the words are shortened, the time stays
        let (line, _) = sleep_line(&canvas, "22:56", 150.0, 24.0);
        assert!(line.ends_with("… 22:56"), "{}", line);
    }

    fn overlaps(a: &Rect, b: &Rect) -> bool {
        a.x < b.x + b.w - 0.5 && b.x < a.x + a.w - 0.5 && a.y < b.y + b.h - 0.5 && b.y < a.y + a.h - 0.5
    }

    #[test]
    fn demo_pages_fit_without_overlap() {
        let config = AppConfig::default();
        for landscape in [false, true] {
            let (w, h) = if landscape { (1448, 1072) } else { (1072, 1448) };
            let mut canvas = Canvas::new(w, h);
            let mut renderer = ScreenRenderer::new(&config);
            let states = crate::ha::mock::generate_mock_states();
            for page in 0..config.pages.len() {
                renderer.render(&mut canvas, &config, page, &states, false);
                let rects: Vec<Rect> = renderer.targets.iter().map(|t| t.rect).collect();
                for r in &rects {
                    assert!(r.y + r.h <= h as f32 + 12.0, "target below screen: {:?}", r);
                    assert!(r.x + r.w <= w as f32 + 12.0, "target right of screen: {:?}", r);
                }
            }
            if landscape {
                continue; // landscape splits into two zones; covered by the render pass above
            }
            let all: Vec<&CardConfig> = config.pages[0].cards.iter().collect();
            let placed = layout_zone(&all, 32.0, 200.0, w as f32 - 64.0, h as f32 - 230.0);
            assert_eq!(placed.len(), all.len(), "a card was dropped (landscape={})", landscape);
            for (i, (_, a)) in placed.iter().enumerate() {
                for (_, b) in placed.iter().skip(i + 1) {
                    assert!(!overlaps(a, b), "{:?} overlaps {:?}", a, b);
                }
            }
        }
    }
}
