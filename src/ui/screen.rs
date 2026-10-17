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
            (nat, nat - rows as f32 * 22.0)
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
}

impl ScreenRenderer {
    pub fn new(config: &AppConfig) -> Self {
        Self {
            theme: Theme::from_config(&config.theme),
            targets: Vec::new(),
            hidden_cards: 0,
            last_update: None,
            frontlight: None,
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

        let placements = zones_for(&page.cards, canvas.width as f32, canvas.height as f32, header_bottom);
        self.hidden_cards = page.cards.len() - placements.len();

        for (card, rect) in placements {
            self.render_card(canvas, card, rect, states);
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

#[cfg(test)]
mod tests {
    use super::*;

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
