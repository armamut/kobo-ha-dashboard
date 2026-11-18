use crate::config::{AppConfig, Card, CardConfig, GridPos, GridSize, ImageFit, PageConfig, SceneItemConfig};
use crate::display::canvas::ui_size;
use crate::display::Canvas;
use crate::ha::models::EntityState;
use crate::ui::layout::{InteractiveTarget, Rect, UiAction};
use crate::ui::theme::Theme;
use crate::ui::widgets::climate_card::{self, CLIMATE_WIDE};
use crate::ui::widgets::breakdown_card;
use crate::ui::widgets::cover_card::COVER_WIDE;
use crate::ui::widgets::glance_card::{self, glance_height};
use crate::ui::widgets::header::{read_device_status, HeaderStatus, SIDE_PAD};
use crate::ui::widgets::scene_card::SCENES_TITLE_H;
use crate::ui::widgets::sensor_card::{sensor_grid_columns, sensor_grid_height, sensor_grid_min_height, TILE_GAP};
use crate::ui::widgets::*;
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use crate::display::canvas::ImageIssue;
use std::sync::Arc;

const GAP: f32 = 18.0;
/// How far a card may come out short of a height it wants when a page gets a
/// grid, rather than take a whole extra row
const SHORT_BY: f32 = 12.0;
const BOTTOM_PAD: f32 = 28.0;
/// Space kept at the bottom for the sleep-mode note
const SLEEP_NOTE_H: f32 = 124.0;
/// Width of the left column in landscape (weather + sensor grid)
const LANDSCAPE_LEFT_W: f32 = 440.0;
/// Narrowest half-width card the right column may hold before the landscape
/// layout falls back to a single full-width column
const LANDSCAPE_MIN_HALF_W: f32 = 300.0;

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
        | CardConfig::MediaPlayer { .. }
        | CardConfig::Heading { .. }
        | CardConfig::Statistics { .. }
        | CardConfig::Glance { .. }
        | CardConfig::Image { .. } => Span::Full,
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
        CardConfig::Climate { show_graph: true, .. } => {
            let controls = climate_card::graph_controls_h(w);
            (controls + 200.0, controls + climate_card::GRAPH_MIN_H)
        }
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
        CardConfig::SensorGrid { title, sensors, .. } => {
            let nat = sensor_grid_height(w, sensors.len(), title.is_some());
            let rows = sensors.len().div_ceil(sensor_grid_columns(w, sensors.len())).max(1);
            (nat, nat - rows as f32 * 10.0)
        }
        CardConfig::MediaPlayer { .. } => (180.0, 140.0),
        CardConfig::Scene { .. } => (112.0, 86.0),
        CardConfig::Heading { .. } => (heading_card::HEADING_H, heading_card::HEADING_H - 12.0),
        CardConfig::Statistics { .. } => (380.0, 300.0),
        CardConfig::Progress { .. } => (190.0, 110.0),
        CardConfig::Breakdown { title, entities } => breakdown_card::breakdown_heights(w, entities.len(), title.is_some()),
        CardConfig::Cover { .. } => {
            if w >= COVER_WIDE {
                (150.0, 110.0)
            } else {
                (220.0, 160.0)
            }
        }
        CardConfig::Glance { title, entities } => (
            glance_height(w, entities.len(), title.is_some(), glance_card::CELL_H),
            glance_height(w, entities.len(), title.is_some(), glance_card::CELL_MIN_H),
        ),
        CardConfig::Image { title, .. } => {
            let t = if title.is_some() { image_card::TITLE_H } else { 0.0 };
            (t + 320.0, t + 220.0)
        }
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
            // A heading whose cards were all cut off would stand alone at the
            // bottom: it goes with them.
            while let Some(&(last, _)) = out.last() {
                let card: &CardConfig = cards[last];
                if !matches!(card, CardConfig::Heading { .. }) {
                    break;
                }
                out.pop();
            }
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

/// Where a page's cards go: below the header, inside the side and bottom pads.
fn content_area(cw: f32, ch: f32, top: f32) -> Rect {
    Rect::new(SIDE_PAD, top, cw - 2.0 * SIDE_PAD, ch - top - BOTTOM_PAD)
}

/// Cards flowing into rows by themselves, for pages without a grid. Indices
/// are into `cards`.
fn flow_zones(cards: &[&CardConfig], area: Rect) -> Vec<(usize, Rect)> {
    let wide = area.w + 2.0 * SIDE_PAD > area.y + area.h + BOTTOM_PAD;
    if wide {
        let (left, right): (Vec<usize>, Vec<usize>) =
            (0..cards.len()).partition(|&i| matches!(cards[i], CardConfig::Weather { .. } | CardConfig::SensorGrid { .. }));
        let right_w = area.w - LANDSCAPE_LEFT_W - GAP;
        // On small landscape screens (Nia, Glo, Aura: 1024 px) the right
        // column would squeeze two cards per row into ~240 px; use the full
        // width instead (issue #4).
        if !left.is_empty() && !right.is_empty() && (right_w - GAP) / 2.0 >= LANDSCAPE_MIN_HALF_W {
            let right_x = area.x + LANDSCAPE_LEFT_W + GAP;
            let l: Vec<&CardConfig> = left.iter().map(|&i| cards[i]).collect();
            let r: Vec<&CardConfig> = right.iter().map(|&i| cards[i]).collect();
            let mut v: Vec<(usize, Rect)> =
                layout_zone(&l, area.x, area.y, LANDSCAPE_LEFT_W, area.h).into_iter().map(|(i, rect)| (left[i], rect)).collect();
            v.extend(layout_zone(&r, right_x, area.y, right_w, area.h).into_iter().map(|(i, rect)| (right[i], rect)));
            return v;
        }
    }
    layout_zone(cards, area.x, area.y, area.w, area.h)
}

/// Rectangle of `pos` on `grid` laid over `area`
pub fn cell_rect(area: Rect, grid: GridSize, pos: GridPos) -> Rect {
    let (cols, rows) = (grid.columns as f32, grid.rows as f32);
    let cell_w = (area.w - GAP * (cols - 1.0)) / cols;
    let cell_h = (area.h - GAP * (rows - 1.0)) / rows;
    Rect::new(
        area.x + pos.x as f32 * (cell_w + GAP),
        area.y + pos.y as f32 * (cell_h + GAP),
        pos.w as f32 * cell_w + (pos.w as f32 - 1.0) * GAP,
        pos.h as f32 * cell_h + (pos.h as f32 - 1.0) * GAP,
    )
}

/// `pos` cut to the grid; `None` when it starts outside it.
fn clip(pos: GridPos, grid: GridSize) -> Option<GridPos> {
    if pos.x >= grid.columns || pos.y >= grid.rows {
        return None;
    }
    Some(GridPos {
        x: pos.x,
        y: pos.y,
        w: pos.w.clamp(1, grid.columns - pos.x),
        h: pos.h.clamp(1, grid.rows - pos.y),
    })
}

/// Cells a card takes when it has no place of its own: the width it has in
/// the flowing layout (full or half), and rows for its natural height.
pub fn natural_cells(card: &CardConfig, grid: GridSize, area: Rect) -> (u8, u8) {
    let grid = grid.clamped();
    let w = match span_of(card) {
        Span::Full => grid.columns,
        Span::Half => (grid.columns / 2).max(1),
    };
    let width = cell_rect(area, grid, GridPos { x: 0, y: 0, w, h: 1 }).w;
    let pitch = (area.h + GAP) / grid.rows as f32;
    let rows = ((heights(card, width).0 + GAP) / pitch - 0.01).ceil();
    (w, (rows as u8).clamp(1, grid.rows))
}

/// First free spot of `w`x`h` cells, row by row.
fn free_spot(taken: &[Vec<bool>], w: u8, h: u8) -> Option<(u8, u8)> {
    let rows = taken.len();
    let cols = taken.first().map_or(0, |r| r.len());
    let (w, h) = (w as usize, h as usize);
    for y in 0..=rows.saturating_sub(h) {
        for x in 0..=cols.saturating_sub(w) {
            if (y..y + h).all(|r| (x..x + w).all(|c| !taken[r][c])) {
                return Some((x as u8, y as u8));
            }
        }
    }
    None
}

/// Rows (or columns) of `pitch` that `len` units need, allowing `SHORT_BY`
fn cells_for(len: f32, pitch: f32, max: u8) -> u8 {
    (((len + GAP - SHORT_BY) / pitch).ceil().max(1.0) as u8).min(max.max(1))
}

/// Smallest (width, height) of a card on a grid page, at width `w`. Below the
/// sizes the flowing layout uses, every card has a compact form: a sensor
/// becomes a row with its icon and value, the weather card leaves out the
/// forecast, the media card the skip buttons, and so on.
fn grid_min(card: &CardConfig, w: f32) -> (f32, f32) {
    match card {
        CardConfig::Sensor { .. } => (150.0, 80.0),
        CardConfig::Light { .. } | CardConfig::Switch { .. } => (120.0, 80.0),
        CardConfig::Scene { .. } => (120.0, 72.0),
        CardConfig::Climate { show_graph: true, .. } => (240.0, climate_card::graph_controls_h(w) + climate_card::GRAPH_MIN_H),
        CardConfig::Climate { .. } => (240.0, 136.0),
        CardConfig::Weather { .. } => (220.0, 130.0),
        CardConfig::MediaPlayer { .. } => (200.0, 100.0),
        CardConfig::SensorGrid { title, sensors, .. } => (150.0, sensor_grid_min_height(w, sensors.len(), title.is_some())),
        CardConfig::ScenesRow { title, scenes } => {
            let t = if title.is_some() { SCENES_TITLE_H } else { 0.0 };
            ((scenes.len() as f32 * 116.0 - 16.0).max(120.0), t + 72.0)
        }
        CardConfig::Heading { .. } => (160.0, heading_card::HEADING_H - 12.0),
        CardConfig::Statistics { .. } => (240.0, 170.0),
        CardConfig::Progress { .. } => (180.0, 72.0),
        CardConfig::Breakdown { title, entities } => (200.0, breakdown_card::breakdown_heights(w, entities.len(), title.is_some()).1),
        CardConfig::Cover { .. } => (200.0, 80.0),
        CardConfig::Glance { title, entities } => (150.0, glance_height(w, entities.len(), title.is_some(), glance_card::CELL_MIN_H)),
        CardConfig::Image { title, .. } => {
            let t = if title.is_some() { image_card::TITLE_H } else { 0.0 };
            (160.0, t + 120.0)
        }
    }
}

/// Smallest size of `card` on `grid` over `area`, in cells, at a width of
/// `w` cells (some cards need less height when wider).
pub fn min_cells(card: &CardConfig, grid: GridSize, area: Rect, w: u8) -> (u8, u8) {
    let grid = grid.clamped();
    let pitch_x = (area.w + GAP) / grid.columns as f32;
    let pitch_y = (area.h + GAP) / grid.rows as f32;
    let min_w = cells_for(grid_min(card, 0.0).0, pitch_x, grid.columns);
    let width = cell_rect(area, grid, GridPos { x: 0, y: 0, w: w.max(min_w).min(grid.columns), h: 1 }).w;
    (min_w, cells_for(grid_min(card, width).1, pitch_y, grid.rows))
}

/// Where each card of a grid page goes, in cells. A card keeps its own place,
/// grown to its smallest size and kept on the grid. One that would then sit
/// on an earlier card moves down in its own columns, else to the first free
/// spot; a card without a place goes to the first free spot big enough for
/// it. A card that finds none gets `None` and is not shown.
pub fn grid_places(cards: &[Card], grid: GridSize, area: Rect) -> Vec<Option<GridPos>> {
    let grid = grid.clamped();
    let mut taken = vec![vec![false; grid.columns as usize]; grid.rows as usize];
    let free = |taken: &[Vec<bool>], p: GridPos| {
        (p.y..p.y + p.h).all(|r| (p.x..p.x + p.w).all(|c| !taken[r as usize][c as usize]))
    };
    let take = |taken: &mut [Vec<bool>], p: GridPos| {
        for row in &mut taken[p.y as usize..(p.y + p.h) as usize] {
            row[p.x as usize..(p.x + p.w) as usize].fill(true);
        }
    };
    // Own place, at least the card's smallest size, inside the grid
    let wanted: Vec<Option<GridPos>> = cards
        .iter()
        .map(|card| {
            let p = card.pos.and_then(|p| clip(p, grid))?;
            let (mw, mh) = min_cells(card, grid, area, p.w);
            let (w, h) = (p.w.max(mw).min(grid.columns), p.h.max(mh).min(grid.rows));
            Some(GridPos { x: p.x.min(grid.columns - w), y: p.y.min(grid.rows - h), w, h })
        })
        .collect();
    let mut placed: Vec<Option<GridPos>> = vec![None; cards.len()];
    let mut moved = Vec::new();
    for (i, p) in wanted.iter().enumerate() {
        let Some(p) = *p else { continue };
        if free(&taken, p) {
            take(&mut taken, p);
            placed[i] = Some(p);
        } else {
            moved.push(i);
        }
    }
    for i in moved {
        let p = wanted[i].expect("only cards with a place move");
        // Its own size lower down in its columns, or anywhere; then the same
        // at its smallest size, rather than not at all
        let (mw, mh) = min_cells(&cards[i], grid, area, p.w);
        let lower = GridPos { h: mh.min(p.h), ..p };
        let small = GridPos { w: mw.min(p.w), h: mh.min(p.h), x: p.x.min(grid.columns - mw.min(p.w)), ..p };
        let spot = [p, lower, small].into_iter().find_map(|p| {
            let below = (p.y..=grid.rows.saturating_sub(p.h)).map(|y| GridPos { y, ..p }).find(|q| free(&taken, *q));
            below.or_else(|| free_spot(&taken, p.w, p.h).map(|(x, y)| GridPos { x, y, ..p }))
        });
        if let Some(q) = spot {
            take(&mut taken, q);
            placed[i] = Some(q);
        }
    }
    // Cards without a place all follow one rule for their size, the first
    // that fits them all: natural, lower, narrower, smallest. A page of
    // twenty lights then gets twenty equal cards, not a few big ones and the
    // rest left out. When no rule fits them all, the one that fits most.
    let unplaced: Vec<usize> = (0..cards.len()).filter(|&i| wanted[i].is_none()).collect();
    if unplaced.is_empty() {
        return placed;
    }
    let sizes = |rule: usize, card: &Card| -> (u8, u8) {
        let (w, h) = natural_cells(card, grid, area);
        let (mw, mh) = min_cells(card, grid, area, w);
        let narrow = (w / 2).max(mw);
        let (nmw, nmh) = min_cells(card, grid, area, narrow);
        match rule {
            0 => (w, h),
            1 => (w, mh.min(h)),
            2 => (narrow, h.max(nmh)),
            3 => (narrow.max(nmw), nmh),
            _ => (mw, mh),
        }
    };
    let try_rule = |rule: usize| {
        let mut t = taken.clone();
        let mut out = Vec::new();
        for &i in &unplaced {
            let (w, h) = sizes(rule, &cards[i]);
            if let Some((x, y)) = free_spot(&t, w, h) {
                let p = GridPos { x, y, w, h };
                take(&mut t, p);
                out.push((i, p));
            }
        }
        out
    };
    let mut best: Vec<(usize, GridPos)> = Vec::new();
    for rule in 0..5 {
        let got = try_rule(rule);
        let all = got.len() == unplaced.len();
        if got.len() > best.len() {
            best = got;
        }
        if all {
            break;
        }
    }
    for (i, p) in best {
        placed[i] = Some(p);
    }
    placed
}

/// Cards on a grid page (see [`grid_places`])
fn grid_zones(cards: &[Card], grid: GridSize, area: Rect) -> Vec<(usize, Rect)> {
    let grid = grid.clamped();
    let placed = grid_places(cards, grid, area);
    let shown = placed.iter().flatten().count();
    if shown < cards.len() {
        tracing::debug!("{} card(s) have no room on the grid", cards.len() - shown);
    }
    placed.into_iter().enumerate().filter_map(|(i, p)| p.map(|p| (i, cell_rect(area, grid, p)))).collect()
}

/// What the setup tool's layout editor needs to lay its grid over the preview:
/// all in UI units, as the renderer uses them.
#[derive(Debug, Clone, serde::Serialize)]
pub struct GridLayout {
    /// The whole screen
    pub screen: (f32, f32),
    /// Where the cells are
    pub area: Rect,
    pub gap: f32,
    pub grid: GridSize,
    /// Where each card is drawn, `None` when it has no room
    pub places: Vec<Option<GridPos>>,
    /// Smallest size of each card, (columns, rows)
    pub mins: Vec<(u8, u8)>,
    /// The page tabs in the header, split evenly between the pages
    pub tab_bar: Option<Rect>,
    /// Size each card has when nothing squeezes it, (columns, rows)
    pub naturals: Vec<(u8, u8)>,
    /// Size of a new card of each type, as the editor makes it, and its
    /// smallest size: (columns, rows, smallest columns, smallest rows)
    pub new_sizes: std::collections::BTreeMap<String, (u8, u8, u8, u8)>,
}

/// A card of each type with the settings the layout editor gives a new one.
fn new_cards() -> Vec<CardConfig> {
    let e = "";
    vec![
        CardConfig::Weather { entity_id: e.into(), title: None, forecast_days: 5 },
        CardConfig::Light { entity_id: e.into(), title: None, subtitle: None, icon: None, show_brightness: false },
        CardConfig::Switch { entity_id: e.into(), title: None, subtitle: None, icon: None },
        CardConfig::Climate { entity_id: e.into(), title: None, step: 0.5, show_graph: false, graph_hours: None },
        CardConfig::Sensor { entity_id: e.into(), title: None, unit: None, icon: None, show_graph: false, large_value: false },
        CardConfig::SensorGrid { title: None, sensors: Vec::new(), large_value: false },
        CardConfig::MediaPlayer { entity_id: e.into(), title: None },
        CardConfig::ScenesRow { title: None, scenes: Vec::new() },
        CardConfig::Scene { entity_id: e.into(), title: String::new(), icon: None },
        CardConfig::Image { entity_id: e.into(), title: None, frameless: false, fit: ImageFit::Contain },
        CardConfig::Progress { entity_id: e.into(), title: None, icon: None, secondary_entity: None, max: None },
        CardConfig::Cover { entity_id: e.into(), title: None, icon: None },
        CardConfig::Glance { title: None, entities: Vec::new() },
        CardConfig::Breakdown { title: None, entities: Vec::new() },
        CardConfig::Heading { title: String::new(), icon: None },
        CardConfig::Statistics {
            entity_id: e.into(),
            title: None,
            period: "day".into(),
            days: 14,
            hours: None,
            stat: "mean".into(),
            unit: None,
            chart: Default::default(),
        },
    ]
}

/// The `type` a card is written with in the config.
fn type_name(card: &CardConfig) -> String {
    serde_json::to_value(card)
        .ok()
        .and_then(|v| v.get("type").and_then(|t| t.as_str()).map(str::to_string))
        .unwrap_or_default()
}

/// Where the header draws the page tabs on a screen `w` wide
fn tab_bar(w: f32) -> Rect {
    Rect::new(SIDE_PAD, HEADER_WITH_TABS - 84.0, w - 2.0 * SIDE_PAD, 64.0)
}

/// The layout editor's view of a grid page on a `w`x`h` screen; `None` for a
/// page without a grid.
pub fn grid_layout(page: &PageConfig, w: u32, h: u32, with_tabs: bool) -> Option<GridLayout> {
    let grid = page.grid?.clamped();
    let (w, h) = ui_size(w, h);
    let area = content_area(w, h, header_height(with_tabs));
    let places = grid_places(&page.cards, grid, area);
    let mins = page
        .cards
        .iter()
        .zip(&places)
        .map(|(c, p)| min_cells(c, grid, area, p.map_or(1, |p| p.w)))
        .collect();
    let naturals = page.cards.iter().map(|c| natural_cells(c, grid, area)).collect();
    let new_sizes = new_cards()
        .iter()
        .map(|c| {
            let (w, h) = natural_cells(c, grid, area);
            let (mw, mh) = min_cells(c, grid, area, 1);
            (type_name(c), (w, h, mw.min(w), mh.min(h)))
        })
        .collect();
    let tab_bar = with_tabs.then(|| tab_bar(w));
    Some(GridLayout { screen: (w, h), area, gap: GAP, grid, places, mins, tab_bar, naturals, new_sizes })
}

/// Where each card of `page` goes on a `cw`x`ch` screen whose header ends at
/// `top`. Indices are into `page.cards`; cards that don't fit are left out.
fn page_zones(page: &PageConfig, cw: f32, ch: f32, top: f32) -> Vec<(usize, Rect)> {
    let area = content_area(cw, ch, top);
    match page.grid {
        Some(grid) => grid_zones(&page.cards, grid, area),
        None => {
            let cards: Vec<&CardConfig> = page.cards.iter().map(|c| &c.kind).collect();
            flow_zones(&cards, area)
        }
    }
}

fn header_height(with_tabs: bool) -> f32 {
    if with_tabs {
        HEADER_WITH_TABS
    } else {
        HEADER_WITHOUT_TABS
    }
}

/// How many cards of `page` would not be shown on a `w`x`h` screen.
pub fn hidden_card_count(page: &PageConfig, w: u32, h: u32, with_tabs: bool) -> usize {
    let (w, h) = ui_size(w, h);
    page.cards.len() - page_zones(page, w, h, header_height(with_tabs)).len()
}

/// How many of `cards` would not fit when they flow into rows by themselves,
/// as on a page without a grid.
pub fn flow_hidden_count(cards: &[CardConfig], w: u32, h: u32, with_tabs: bool) -> usize {
    let (w, h) = ui_size(w, h);
    let refs: Vec<&CardConfig> = cards.iter().collect();
    cards.len() - flow_zones(&refs, content_area(w, h, header_height(with_tabs))).len()
}

/// Give pages without a grid one, keeping their cards where they flow on a
/// `w`x`h` screen. Pages that have a grid are left alone. Returns whether any
/// page changed.
pub fn grid_pages(pages: &mut [PageConfig], w: u32, h: u32, with_tabs: bool) -> bool {
    let mut changed = false;
    for page in pages.iter_mut().filter(|p| p.grid.is_none()) {
        let (grid, places) = grid_from_flow(page, w, h, with_tabs);
        page.grid = Some(grid);
        for (card, pos) in page.cards.iter_mut().zip(places) {
            card.pos = pos;
        }
        changed = true;
    }
    changed
}

/// Grid places for a page without a grid, keeping where its cards flow on a
/// `w`x`h` screen. A portrait page gets 6 columns and 12 rows, a landscape
/// one 12x8; a page too full for them gets finer rows. Each card keeps its columns and gets the rows its natural height needs
/// (its minimum when the page is full), cards side by side get the same rows,
/// and the cards below move down to make room. Cards that don't fit get no
/// place, as they were hidden before too.
pub fn grid_from_flow(page: &PageConfig, w: u32, h: u32, with_tabs: bool) -> (GridSize, Vec<Option<GridPos>>) {
    let (uw, uh) = ui_size(w, h);
    let (columns, rows): (u8, &[u8]) = if uw > uh { (12, &[8, 12, 16]) } else { (6, &[12, 18, 24]) };
    let mut last = None;
    for &r in rows {
        let (places, fits) = pack_flow(page, GridSize { columns, rows: r }, uw, uh, with_tabs);
        if fits {
            return (GridSize { columns, rows: r }, places);
        }
        last = Some((GridSize { columns, rows: r }, places));
    }
    last.expect("at least one grid size")
}

/// Places for `page` on `grid` (see [`grid_from_flow`]), and whether every
/// card that flows on screen found one.
fn pack_flow(page: &PageConfig, grid: GridSize, w: f32, h: f32, with_tabs: bool) -> (Vec<Option<GridPos>>, bool) {
    let area = content_area(w, h, header_height(with_tabs));
    let cards: Vec<&CardConfig> = page.cards.iter().map(|c| &c.kind).collect();
    let pitch_x = (area.w + GAP) / grid.columns as f32;
    let pitch_y = (area.h + GAP) / grid.rows as f32;
    let col = |v: f32| ((v / pitch_x).round().max(0.0) as u8).min(grid.columns);
    let rows_for = |height: f32| cells_for(height, pitch_y, grid.rows);

    let mut zones = flow_zones(&cards, area);
    zones.sort_by(|a, b| a.1.y.total_cmp(&b.1.y).then(a.1.x.total_cmp(&b.1.x)));
    // Columns of each card, and the rows it wants: (natural, minimum)
    let wants: Vec<(usize, Rect, u8, u8, u8, u8)> = zones
        .iter()
        .map(|&(i, r)| {
            let x0 = col(r.x - area.x).min(grid.columns - 1);
            let x1 = col(r.x - area.x + r.w + GAP).max(x0 + 1);
            let width = cell_rect(area, grid, GridPos { x: x0, y: 0, w: x1 - x0, h: 1 }).w;
            let (nat, min) = heights(cards[i], width);
            (i, r, x0, x1, rows_for(nat), rows_for(min))
        })
        .collect();

    let pack = |use_min: bool| {
        let mut out: Vec<Option<GridPos>> = vec![None; cards.len()];
        let mut floor = vec![0u8; grid.columns as usize];
        let mut fits = true;
        let mut k = 0;
        while k < wants.len() {
            // Cards of one flowing row share its top and height
            let (_, r0, ..) = wants[k];
            let end = k + wants[k..].iter().take_while(|w| w.1.y == r0.y && w.1.h == r0.h).count();
            let row = &wants[k..end];
            let hh = row.iter().map(|w| if use_min { w.5 } else { w.4 }).max().unwrap_or(1);
            let top = row.iter().map(|w| floor[w.2 as usize..w.3 as usize].iter().copied().max().unwrap_or(0)).max().unwrap_or(0);
            if top as u16 + hh as u16 > grid.rows as u16 {
                fits = false;
                k = end;
                continue;
            }
            for &(i, _, x0, x1, ..) in row {
                out[i] = Some(GridPos { x: x0, y: top, w: x1 - x0, h: hh });
                floor[x0 as usize..x1 as usize].fill(top + hh);
            }
            k = end;
        }
        (out, fits)
    };
    let (natural, fits) = pack(false);
    if fits {
        (natural, true)
    } else {
        pack(true)
    }
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
    /// Draw the "battery empty" screen instead of the dashboard
    pub battery_empty: bool,
    /// Battery, charging and Wi-Fi to show instead of the device's own (the
    /// setup tool's preview runs on a computer)
    pub device_status: Option<(Option<u8>, bool, Option<bool>)>,
    /// Pixels per UI unit on the last rendered canvas
    scale: f32,
    /// Raw images of `image.*` entities, as fetched from Home Assistant
    images: HashMap<String, Arc<Vec<u8>>>,
    /// Decoded and scaled copies, so a redraw does not decode again. Keyed by
    /// entity and box size; the `Arc` identity says whether the bytes changed.
    image_cache: HashMap<String, (Arc<Vec<u8>>, Result<Arc<tiny_skia::Pixmap>, ImageIssue>)>,
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
            battery_empty: false,
            device_status: None,
            scale: 1.0,
            images: HashMap::new(),
            image_cache: HashMap::new(),
        }
    }

    /// Pictures for the `image` cards; decoded lazily, only when they change.
    pub fn set_images(&mut self, images: HashMap<String, Arc<Vec<u8>>>) {
        self.images = images;
    }

    /// Decoded picture for one card, scaled to its box, or why it can't be
    /// shown. `None` while it has not been fetched.
    fn image_pixmap(&mut self, entity_id: &str, area: Rect, fit: ImageFit, scale: f32) -> Option<Result<Arc<tiny_skia::Pixmap>, ImageIssue>> {
        let bytes = self.images.get(entity_id)?.clone();
        let bw = ((area.w * scale).round() as i32).clamp(1, 4096) as u32;
        let bh = ((area.h * scale).round() as i32).clamp(1, 4096) as u32;
        let key = format!("{}@{}x{}:{:?}", entity_id, bw, bh, fit);
        if let Some((cached, pixmap)) = self.image_cache.get(&key) {
            if Arc::ptr_eq(cached, &bytes) {
                return Some(pixmap.clone());
            }
        }
        // A failure is cached too: a picture that is too large or broken is not
        // decoded again (and logged again) on every redraw.
        let pixmap = crate::display::canvas::fit_image_as(&bytes, bw, bh, fit).map(Arc::new);
        self.image_cache.insert(key, (bytes, pixmap.clone()));
        Some(pixmap)
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
        self.scale = canvas.scale;
        canvas.clear(self.theme.bg);

        if self.battery_empty {
            self.render_battery_empty(canvas);
            return;
        }

        let (battery, charging, wifi_up) = self.device_status.unwrap_or_else(read_device_status);
        let wifi_up = crate::ui::widgets::header::shown_wifi(wifi_up, self.sleep_until.is_some(), crate::power::wifi_switched_off());
        let status = HeaderStatus {
            is_live,
            is_demo: config.home_assistant.token.trim().is_empty(),
            last_update: self.last_update,
            battery,
            charging,
            wifi_up,
            asleep: self.sleep_until.is_some(),
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
        let (w, h) = canvas.ui_size();
        let zones = page_zones(page, w, h - reserve, header_bottom);
        self.hidden_cards = page.cards.len() - zones.len();
        let placements: Vec<(&CardConfig, Rect)> = zones.into_iter().map(|(i, r)| (&page.cards[i].kind, r)).collect();

        // Pictures are decoded before drawing: the widget only places them.
        let scale = canvas.scale;
        let prepared: Vec<Option<Result<Arc<tiny_skia::Pixmap>, ImageIssue>>> = placements
            .iter()
            .map(|(card, rect)| match card {
                CardConfig::Image { entity_id, title, frameless, fit } => {
                    self.image_pixmap(entity_id, image_card::picture_box(*rect, title.is_some(), *frameless), *fit, scale)
                }
                _ => None,
            })
            .collect();

        for (i, (card, rect)) in placements.into_iter().enumerate() {
            let image = prepared.get(i).and_then(|p| p.as_ref()).map(|r| r.as_deref().map_err(|e| *e));
            self.render_card(canvas, card, rect, states, image);
        }

        if let Some(next) = self.sleep_until.clone() {
            let bar = if has_tabs {
                tab_bar(w)
            } else {
                Rect::new(SIDE_PAD, h - SLEEP_NOTE_H + 4.0, w - 2.0 * SIDE_PAD, SLEEP_NOTE_H - 28.0)
            };
            self.render_sleep_note(canvas, bar, &next);
            // Nothing is tappable while the device sleeps.
            self.targets.clear();
        }
    }

    /// The whole screen, not a note on the dashboard: it can stay up for days
    /// and old values shouldn't look current. E-ink keeps it without power.
    fn render_battery_empty(&self, canvas: &mut Canvas) {
        use crate::display::TextAlign;
        use crate::i18n;
        let theme = &self.theme;
        let (w, h) = canvas.ui_size();
        let icon = 220.0;
        let top = h / 2.0 - icon / 2.0 - 90.0;
        let max_w = w - 2.0 * SIDE_PAD;
        canvas.draw_icon("battery_empty", w / 2.0 - icon / 2.0, top, icon, theme.text, 3.0);
        canvas.draw_text_fit(&i18n::t("sleep.battery_empty"), w / 2.0, top + icon + 24.0, 60.0, true, theme.text, TextAlign::Center, max_w);
        canvas.draw_text_fit(&i18n::t("sleep.charge_hint"), w / 2.0, top + icon + 108.0, 32.0, false, theme.text, TextAlign::Center, max_w);
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

    fn render_card(
        &mut self,
        canvas: &mut Canvas,
        card: &CardConfig,
        rect: Rect,
        states: &HashMap<String, EntityState>,
        image: Option<Result<&tiny_skia::Pixmap, ImageIssue>>,
    ) {
        let theme = &self.theme;
        let targets = &mut self.targets;
        match card {
            CardConfig::Weather { entity_id, title, forecast_days } => {
                render_weather_card(canvas, theme, rect, entity_id, states.get(entity_id), title.as_deref(), *forecast_days, targets)
            }
            CardConfig::ScenesRow { title, scenes } => render_scenes_row(canvas, theme, rect, title.as_deref(), scenes, targets),
            CardConfig::SensorGrid { title, sensors, large_value } => {
                render_sensor_grid(canvas, theme, rect, title.as_deref(), sensors, *large_value, states, targets)
            }
            CardConfig::MediaPlayer { entity_id, title } => {
                render_media_card(canvas, theme, rect, entity_id, states.get(entity_id), title.as_deref(), targets)
            }
            CardConfig::Climate { entity_id, title, step, show_graph, graph_hours } => render_climate_card(
                canvas,
                theme,
                rect,
                entity_id,
                states.get(entity_id),
                title.as_deref(),
                *step,
                show_graph.then(|| graph_hours.unwrap_or(24)),
                targets,
            ),
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
            CardConfig::Sensor { entity_id, title, unit, icon, show_graph, large_value } => render_sensor_card(
                canvas,
                theme,
                rect,
                entity_id,
                states.get(entity_id),
                title.as_deref(),
                unit.as_deref(),
                icon.as_deref(),
                *show_graph,
                *large_value,
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
            CardConfig::Heading { title, icon } => render_heading_card(canvas, theme, rect, title, icon.as_deref()),
            CardConfig::Statistics { entity_id, title, period, days, hours, stat, unit, chart } => render_statistics_card(
                canvas,
                theme,
                rect,
                entity_id,
                states.get(entity_id),
                title.as_deref(),
                period,
                statistics_card::window_hours(*days, *hours),
                stat,
                unit.as_deref(),
                *chart,
                targets,
            ),
            CardConfig::Progress { entity_id, title, icon, secondary_entity, max } => render_progress_card(
                canvas,
                theme,
                rect,
                entity_id,
                states.get(entity_id),
                secondary_entity.as_ref().and_then(|e| states.get(e)),
                title.as_deref(),
                icon.as_deref(),
                max.unwrap_or(100.0),
                targets,
            ),
            CardConfig::Cover { entity_id, title, icon } => {
                render_cover_card(canvas, theme, rect, entity_id, states.get(entity_id), title.as_deref(), icon.as_deref(), targets)
            }
            CardConfig::Glance { title, entities } => {
                render_glance_card(canvas, theme, rect, title.as_deref(), entities, states, targets)
            }
            CardConfig::Breakdown { title, entities } => {
                render_breakdown_card(canvas, theme, rect, title.as_deref(), entities, states, targets)
            }
            CardConfig::Image { entity_id, title, frameless, .. } => {
                render_image_card(canvas, theme, rect, entity_id, title.as_deref(), *frameless, image, targets)
            }
        }
    }

    /// `x`, `y` in screen pixels.
    pub fn handle_touch(&self, x: f32, y: f32) -> Option<UiAction> {
        let (x, y) = (x / self.scale, y / self.scale);
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
    fn a_heading_does_not_stay_behind_when_its_cards_are_cut_off() {
        let heading = CardConfig::Heading { title: "Pictures".into(), icon: None };
        let sensor = |id: &str| CardConfig::Sensor {
            entity_id: id.into(),
            title: None,
            unit: None,
            icon: None,
            show_graph: true,
            large_value: false,
        };
        let cards = [sensor("sensor.a"), sensor("sensor.b"), heading.clone(), sensor("sensor.c")];
        let refs: Vec<&CardConfig> = cards.iter().collect();
        // Room for the first three rows only: the heading's card is cut off.
        let full = layout_zone(&refs, 0.0, 0.0, 600.0, 10_000.0);
        let h2 = full[2].1.y;
        let cut = layout_zone(&refs, 0.0, 0.0, 600.0, h2 + 1.0 + (full[2].1.h));
        assert!(cut.iter().all(|&(i, _)| !matches!(cards[i], CardConfig::Heading { .. })), "{:?}", cut);
        assert_eq!(cut.len(), 2);
    }

    #[test]
    fn renders_an_image_card_from_fetched_bytes() {
        let mut config = AppConfig::default();
        let page = &mut config.pages[0];
        page.cards = vec![CardConfig::Image { entity_id: "image.chart".into(), title: Some("Prices".into()), frameless: false, fit: ImageFit::Contain }.into()];

        // A red PNG, as Home Assistant would serve it from /api/image_proxy
        let mut img = image::RgbaImage::new(400, 200);
        for px in img.pixels_mut() {
            *px = image::Rgba([200, 30, 30, 255]);
        }
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(img).write_to(&mut bytes, image::ImageFormat::Png).unwrap();

        let mut images = HashMap::new();
        images.insert("image.chart".to_string(), Arc::new(bytes.into_inner()));

        let mut renderer = ScreenRenderer::new(&config);
        renderer.set_images(images);
        let mut canvas = Canvas::new(758, 1024);
        renderer.render(&mut canvas, &config, 0, &HashMap::new(), false);

        let data = canvas.raw_rgba_data();
        let red = (0..758 * 1024)
            .filter(|i| {
                let p = i * 4;
                data[p] > 150 && data[p + 1] < 90 && data[p + 2] < 90
            })
            .count();
        assert!(red > 20_000, "{} red pixels: the picture is not drawn", red);

        // Without the bytes the card still draws, with a placeholder
        let mut renderer = ScreenRenderer::new(&config);
        let mut canvas = Canvas::new(758, 1024);
        renderer.render(&mut canvas, &config, 0, &HashMap::new(), false);
        assert_eq!(renderer.hidden_cards, 0);
    }

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

    #[test]
    fn battery_empty_screen_replaces_the_dashboard() {
        let _guard = crate::i18n::TEST_LANGUAGE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let config = AppConfig::default();
        let mut canvas = Canvas::new(758, 1024);
        let mut renderer = ScreenRenderer::new(&config);
        renderer.battery_empty = true;
        renderer.render(&mut canvas, &config, 0, &HashMap::new(), false);
        assert!(renderer.targets.is_empty(), "nothing is tappable");
        // The narrowest screen fits both lines whole in every language
        for (code, _) in crate::i18n::BUILTIN {
            crate::i18n::set_language(code);
            let max_w = 758.0 - 2.0 * SIDE_PAD;
            for (key, size, bold) in [("sleep.battery_empty", 60.0, true), ("sleep.charge_hint", 32.0, false)] {
                let text = crate::i18n::t(key);
                assert!(canvas.measure_text(&text, size, bold).0 <= max_w, "{}: {}", code, text);
            }
        }
        crate::i18n::set_language("en");
    }

    #[test]
    fn smallest_screens_scale_the_layout_down() {
        // Kobo Touch (issue #7): 6" like the Glo, the same page has to fit
        let page = AppConfig::default().pages[0].clone();
        for ((w, h), (glo_w, glo_h)) in [((600, 800), (758, 1024)), ((800, 600), (1024, 758))] {
            assert_eq!(hidden_card_count(&page, w, h, true), hidden_card_count(&page, glo_w, glo_h, true));
            assert_eq!(ui_size(w, h).0.max(ui_size(w, h).1), 1024.0);
        }
        // Larger screens stay at one pixel per unit
        assert_eq!(ui_size(758, 1024), (758.0, 1024.0));
        assert_eq!(ui_size(1072, 1448), (1072.0, 1448.0));
    }

    #[test]
    fn taps_land_on_scaled_targets() {
        let _guard = crate::i18n::TEST_LANGUAGE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let config = AppConfig::default();
        let mut canvas = Canvas::for_screen(600, 800);
        let mut renderer = ScreenRenderer::new(&config);
        renderer.render(&mut canvas, &config, 0, &HashMap::new(), false);
        let (w, h) = canvas.ui_size();
        // Everything tappable is on screen, and a tap in the middle of a
        // target's pixels hits that target rather than a neighbour.
        let s = canvas.scale;
        for t in &renderer.targets {
            assert!(t.rect.x + t.rect.w <= w + 0.5 && t.rect.y + t.rect.h <= h + 0.5, "{:?}", t.rect);
            let hit = renderer.handle_touch((t.rect.x + t.rect.w / 2.0) * s, (t.rect.y + t.rect.h / 2.0) * s);
            assert_eq!(format!("{:?}", hit), format!("{:?}", Some(t.action.clone())));
        }
    }

    #[test]
    fn small_landscape_uses_one_column() {
        // Nia/Glo/Aura in landscape: two columns left ~240 px cards (issue #4)
        let page = AppConfig::default().pages[0].clone();
        for (_, r) in page_zones(&page, 1024.0, 758.0, HEADER_WITH_TABS) {
            assert!(r.w >= LANDSCAPE_MIN_HALF_W, "{:?}", r);
        }
        // Larger screens keep the weather column
        let placed = page_zones(&page, 1448.0, 1072.0, HEADER_WITH_TABS);
        assert!(placed.iter().any(|(_, r)| r.w == LANDSCAPE_LEFT_W));
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
            let mut states = crate::ha::mock::generate_mock_states();
        let bars: Vec<(f64, f64)> = (0..14).map(|i| (1.7e12 + i as f64 * 86.4e6, 3.0 + (i * 7 % 5) as f64)).collect();
        states.get_mut("sensor.energy_today").unwrap().attributes.insert(
            crate::ui::widgets::statistics_card::STATS_ATTR.into(),
            serde_json::json!({ crate::ui::widgets::statistics_card::stat_key("day", 14 * 24, "change"): bars }),
        );
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
            let all: Vec<&CardConfig> = config.pages[0].cards.iter().map(|c| &c.kind).collect();
            let placed = layout_zone(&all, 32.0, 200.0, w as f32 - 64.0, h as f32 - 230.0);
            assert_eq!(placed.len(), all.len(), "a card was dropped (landscape={})", landscape);
            for (i, (_, a)) in placed.iter().enumerate() {
                for (_, b) in placed.iter().skip(i + 1) {
                    assert!(!overlaps(a, b), "{:?} overlaps {:?}", a, b);
                }
            }
        }
    }

    fn switch(id: &str, pos: Option<GridPos>) -> Card {
        Card {
            kind: CardConfig::Switch { entity_id: id.into(), title: None, subtitle: None, icon: None },
            pos,
        }
    }

    fn grid_page(columns: u8, rows: u8, cards: Vec<Card>) -> PageConfig {
        PageConfig {
            id: "p".into(),
            title: "P".into(),
            icon: "home".into(),
            grid: Some(GridSize { columns, rows }),
            cards,
            imported_from: None,
        }
    }

    #[test]
    fn a_card_sits_where_its_grid_place_says() {
        // 6x9 grid, a 2x2 card in the bottom-right corner
        let page = grid_page(6, 9, vec![switch("switch.a", Some(GridPos { x: 4, y: 7, w: 2, h: 2 }))]);
        let (w, h) = (1072.0, 1448.0);
        let zones = page_zones(&page, w, h, HEADER_WITH_TABS);
        assert_eq!(zones.len(), 1);
        let r = zones[0].1;
        let area = content_area(w, h, HEADER_WITH_TABS);
        assert!((r.x + r.w - (area.x + area.w)).abs() < 0.01, "{:?} not at the right edge of {:?}", r, area);
        assert!((r.y + r.h - (area.y + area.h)).abs() < 0.01, "{:?} not at the bottom of {:?}", r, area);
        let cell_w = (area.w - 5.0 * GAP) / 6.0;
        assert!((r.w - (2.0 * cell_w + GAP)).abs() < 0.01);

        // The same page on a Glo: same share of the screen
        let (gw, gh) = (758.0, 1024.0);
        let g = page_zones(&page, gw, gh, HEADER_WITH_TABS)[0].1;
        let garea = content_area(gw, gh, HEADER_WITH_TABS);
        assert!((g.x + g.w - (garea.x + garea.w)).abs() < 0.01 && (g.y + g.h - (garea.y + garea.h)).abs() < 0.01);
    }

    #[test]
    fn cards_without_a_place_fill_free_cells_and_the_rest_is_hidden() {
        // The top half is taken; the others go below it, in order
        let mut cards = vec![switch("switch.big", Some(GridPos { x: 0, y: 0, w: 6, h: 6 }))];
        cards.extend((0..10).map(|i| switch(&format!("switch.{}", i), None)));
        let page = grid_page(6, 12, cards);
        let area = content_area(1072.0, 1448.0, HEADER_WITH_TABS);
        let zones = page_zones(&page, 1072.0, 1448.0, HEADER_WITH_TABS);
        let big = zones[0].1;
        for (_, r) in &zones[1..] {
            assert!(r.y >= big.y + big.h, "{:?} overlaps the placed card", r);
            assert!(r.y + r.h <= area.y + area.h + 0.01);
        }
        for (i, (_, a)) in zones.iter().enumerate() {
            for (_, b) in zones.iter().skip(i + 1) {
                assert!(!overlaps(a, b), "{:?} overlaps {:?}", a, b);
            }
        }
        // At their natural size only 6 of them would fit; all 10 share a
        // smaller size instead
        assert_eq!(zones.len(), 1 + 10);
        assert_eq!(hidden_card_count(&page, 1072, 1448, true), 0);
        // Too many even at the smallest size: the rest is hidden
        let mut cards = vec![switch("switch.big", Some(GridPos { x: 0, y: 0, w: 6, h: 6 }))];
        cards.extend((0..60).map(|i| switch(&format!("switch.{}", i), None)));
        let page = grid_page(6, 12, cards);
        assert!(hidden_card_count(&page, 1072, 1448, true) > 0);
    }

    #[test]
    fn places_outside_the_grid_are_cut_or_moved() {
        let page = grid_page(
            4,
            4,
            vec![
                switch("switch.over", Some(GridPos { x: 2, y: 2, w: 5, h: 5 })),
                switch("switch.outside", Some(GridPos { x: 4, y: 0, w: 1, h: 1 })),
            ],
        );
        let area = content_area(1072.0, 1448.0, HEADER_WITH_TABS);
        let zones = page_zones(&page, 1072.0, 1448.0, HEADER_WITH_TABS);
        // The one starting outside goes to the first free spot instead
        assert_eq!(zones.len(), 2);
        assert_eq!(zones[1].1.y, area.y);
        let r = zones[0].1;
        assert!((r.x + r.w - (area.x + area.w)).abs() < 0.01 && (r.y + r.h - (area.y + area.h)).abs() < 0.01);
        // A zero-sized grid or place is read as one cell
        let mut page = grid_page(0, 0, vec![switch("switch.a", Some(GridPos { x: 0, y: 0, w: 0, h: 0 }))]);
        assert_eq!(page_zones(&page, 1072.0, 1448.0, HEADER_WITH_TABS).len(), 1);
        page.grid = Some(GridSize { columns: 200, rows: 200 });
        assert_eq!(page_zones(&page, 1072.0, 1448.0, HEADER_WITH_TABS).len(), 1);
    }

    #[test]
    fn older_pages_get_grid_places_matching_their_layout() {
        let config = AppConfig::default();
        let screens = [(1072, 1448), (1448, 1072), (758, 1024), (1024, 758), (1264, 1680), (1680, 1264)];
        for page in &config.pages {
            for (w, h) in screens {
                let (grid, places) = grid_from_flow(page, w, h, true);
                let hidden = hidden_card_count(page, w, h, true);
                assert_eq!(places.iter().filter(|p| p.is_none()).count(), hidden, "{} {}x{}", page.id, w, h);
                let mut gridded = page.clone();
                gridded.grid = Some(grid);
                for (card, pos) in gridded.cards.iter_mut().zip(&places) {
                    card.pos = *pos;
                }
                let (uw, uh) = ui_size(w, h);
                // The places stand as they are: nothing is grown or moved
                let resolved = grid_places(&gridded.cards, grid, content_area(uw, uh, HEADER_WITH_TABS));
                for (i, p) in places.iter().enumerate() {
                    if p.is_some() {
                        assert_eq!(resolved[i], *p, "{} {}x{}: card {}", page.id, w, h, i);
                    }
                }
                let mut flow = page_zones(page, uw, uh, HEADER_WITH_TABS);
                flow.sort_by_key(|z| z.0);
                let all = page_zones(&gridded, uw, uh, HEADER_WITH_TABS);
                // Cards cut off before may now find room in cells left free
                assert!(all.len() >= flow.len(), "{} {}x{}", page.id, w, h);
                let snapped: Vec<(usize, Rect)> = all.iter().copied().filter(|(i, _)| places[*i].is_some()).collect();
                assert_eq!(flow.len(), snapped.len(), "{} {}x{}", page.id, w, h);
                for (i, (_, a)) in all.iter().enumerate() {
                    for (_, b) in all.iter().skip(i + 1) {
                        assert!(!overlaps(a, b), "{} {}x{}: {:?} overlaps {:?}", page.id, w, h, a, b);
                    }
                }
                // Each card keeps its columns, give or take half a cell, and
                // what was above it stays above it
                let cw = (uw - 2.0 * SIDE_PAD) / grid.columns as f32;
                for ((i, f), (j, g)) in flow.iter().zip(&snapped) {
                    assert_eq!(i, j);
                    assert!((f.x - g.x).abs() <= cw / 2.0 + GAP, "{} {}x{}: {:?} -> {:?}", page.id, w, h, f, g);
                    assert!((f.w - g.w).abs() <= cw + GAP, "{} {}x{}: {:?} -> {:?}", page.id, w, h, f, g);
                    for ((_, f2), (_, g2)) in flow.iter().zip(&snapped) {
                        let same_columns = f.x < f2.x + f2.w && f2.x < f.x + f.w;
                        if same_columns && f.y + f.h <= f2.y {
                            assert!(g.y + g.h <= g2.y + 0.5, "{} {}x{}: {:?} no longer above {:?}", page.id, w, h, g, g2);
                        }
                    }
                }
                // Side by side before, side by side now: same top, same height
                for ((_, f), (_, g)) in flow.iter().zip(&snapped) {
                    for ((_, f2), (_, g2)) in flow.iter().zip(&snapped) {
                        if f.y == f2.y && f.h == f2.h {
                            assert!(g.y == g2.y && g.h == g2.h, "{} {}x{}: {:?} / {:?}", page.id, w, h, g, g2);
                        }
                    }
                }
                // No card is squeezed far below its smallest height
                for (i, g) in &snapped {
                    let min = heights(&page.cards[*i], g.w).1;
                    assert!(g.h >= min - SHORT_BY - 0.5, "{} {}x{}: {:?} below {}", page.id, w, h, g, min);
                }
            }
        }
    }

    #[test]
    fn a_card_on_top_of_another_moves_to_a_free_spot() {
        let page = grid_page(
            6,
            12,
            vec![
                switch("switch.a", Some(GridPos { x: 0, y: 0, w: 3, h: 2 })),
                switch("switch.b", Some(GridPos { x: 2, y: 1, w: 3, h: 2 })),
            ],
        );
        let area = content_area(1072.0, 1448.0, HEADER_WITH_TABS);
        let places = grid_places(&page.cards, page.grid.unwrap(), area);
        assert_eq!(places[0], Some(GridPos { x: 0, y: 0, w: 3, h: 2 }));
        // Same size and columns, just below the earlier card
        assert_eq!(places[1], Some(GridPos { x: 2, y: 2, w: 3, h: 2 }));

        // Smaller than it can be drawn: grown, and kept on the grid
        let climate = Card {
            kind: CardConfig::Climate { entity_id: "climate.a".into(), title: None, step: 0.5, show_graph: false, graph_hours: None },
            pos: Some(GridPos { x: 5, y: 11, w: 1, h: 1 }),
        };
        let page = grid_page(6, 12, vec![climate]);
        let (mw, mh) = min_cells(&page.cards[0], page.grid.unwrap(), area, 1);
        assert!(mw > 1 && mh > 1, "{}x{}", mw, mh);
        let p = grid_places(&page.cards, page.grid.unwrap(), area)[0].unwrap();
        assert_eq!((p.w, p.h), (mw, mh));
        assert_eq!((p.x + p.w, p.y + p.h), (6, 12));
    }

    #[test]
    fn grid_pages_made_from_older_ones_respect_the_smallest_sizes() {
        // Otherwise the layout editor would grow those cards when it opens
        let config = AppConfig::default();
        for (w, h) in [(1072, 1448), (1448, 1072), (758, 1024), (1024, 758), (1264, 1680)] {
            let mut pages = config.pages.clone();
            grid_pages(&mut pages, w, h, true);
            for page in &pages {
                let layout = grid_layout(page, w, h, true).unwrap();
                for (i, (place, (mw, mh))) in layout.places.iter().zip(&layout.mins).enumerate() {
                    // Hidden before too: the page is full on this screen
                    let Some(place) = place else { continue };
                    assert!(place.w >= *mw && place.h >= *mh, "{} {}x{}: card {} at {:?}, smallest {}x{}", page.id, w, h, i, place, mw, mh);
                }
            }
        }
    }

    #[test]
    fn every_card_type_has_a_size_for_new_cards() {
        let mut config = AppConfig::default();
        grid_pages(&mut config.pages, 1072, 1448, true);
        let layout = grid_layout(&config.pages[0], 1072, 1448, true).unwrap();
        assert_eq!(layout.new_sizes.len(), 16, "{:?}", layout.new_sizes);
        for (kind, (w, h, mw, mh)) in &layout.new_sizes {
            assert!(*w >= 1 && *h >= 1 && *w <= layout.grid.columns && *h <= layout.grid.rows, "{kind}: {w}x{h}");
            assert!(mw <= w && mh <= h && *mw >= 1 && *mh >= 1, "{kind}: smallest {mw}x{mh} of {w}x{h}");
        }
        assert_eq!(layout.naturals.len(), config.pages[0].cards.len());
    }

    #[test]
    fn a_moved_card_shrinks_to_its_smallest_size_rather_than_vanish() {
        let grid = GridSize { columns: 6, rows: 12 };
        let area = content_area(1072.0, 1448.0, HEADER_WITH_TABS);
        // Two rows left at the top; the second card wants 6x4 on top of the first
        let page = grid_page(
            6,
            12,
            vec![
                switch("switch.big", Some(GridPos { x: 0, y: 2, w: 6, h: 10 })),
                switch("switch.late", Some(GridPos { x: 0, y: 4, w: 6, h: 4 })),
            ],
        );
        let places = grid_places(&page.cards, grid, area);
        let (_, mh) = min_cells(&page.cards[1], grid, area, 6);
        let p = places[1].expect("it fits, lower");
        // Its own width is kept; only the height comes down
        assert_eq!((p.w, p.h), (6, mh.min(4)));
        assert!(p.y + p.h <= 2, "{:?}", p);
    }

    #[test]
    fn cards_without_a_place_share_one_size() {
        // More lights than fit at their natural size: all get the same,
        // smaller size rather than a few big ones and the rest left out
        let grid = GridSize { columns: 12, rows: 18 };
        let area = content_area(1072.0, 1448.0, HEADER_WITH_TABS);
        let cards: Vec<Card> = (0..24).map(|i| switch(&format!("switch.s{i}"), None)).collect();
        let places = grid_places(&cards, grid, area);
        let shown: Vec<GridPos> = places.iter().flatten().copied().collect();
        let (nw, nh) = natural_cells(&cards[0], grid, area);
        assert!(shown.len() > (12 / nw as usize) * (18 / nh as usize), "only {} shown", shown.len());
        assert!(shown.iter().all(|p| (p.w, p.h) == (shown[0].w, shown[0].h)), "{:?}", shown);
    }

    #[test]
    fn small_cards_fit_one_cell() {
        // A sensor, a light or a switch fits a single cell of a 6x12 grid in
        // its compact form; a thermostat needs more.
        let grid = GridSize { columns: 6, rows: 12 };
        let area = content_area(1072.0, 1448.0, HEADER_WITH_TABS);
        let sensor = CardConfig::Sensor {
            entity_id: "sensor.t".into(),
            title: None,
            unit: None,
            icon: None,
            show_graph: true,
            large_value: false,
        };
        assert_eq!(min_cells(&sensor, grid, area, 1), (1, 1));
        assert_eq!(min_cells(&switch("switch.a", None).kind, grid, area, 1), (1, 1));
        let climate = CardConfig::Climate { entity_id: "climate.a".into(), title: None, step: 0.5, show_graph: false, graph_hours: None };
        let (w, h) = min_cells(&climate, grid, area, 1);
        assert!(w >= 2 && h >= 2, "{}x{}", w, h);
    }

    #[test]
    fn a_new_card_takes_its_smallest_size_when_only_that_fits() {
        // Everything but the last two rows is taken
        let grid = GridSize { columns: 6, rows: 18 };
        let area = content_area(1072.0, 1448.0, HEADER_WITH_TABS);
        let new = switch("switch.new", None);
        let (_, natural) = natural_cells(&new, grid, area);
        assert!(natural > 2, "the test needs a card taller than the room left");
        let page = grid_page(6, 18, vec![switch("switch.big", Some(GridPos { x: 0, y: 0, w: 6, h: 16 })), new]);
        let p = grid_places(&page.cards, grid, area)[1].expect("the card fits at its smallest");
        assert_eq!((p.y, p.h), (16, 2));
    }


    /// Every card at sizes from its natural one down to its smallest, written
    /// to `$CARD_GALLERY` as a PNG: `CARD_GALLERY=/tmp/g.png cargo test card_gallery -- --ignored`
    #[test]
    #[ignore]
    fn card_gallery() {
        let Ok(out) = std::env::var("CARD_GALLERY") else { return };
        let cards: Vec<CardConfig> = serde_yaml::from_str(
            r#"
- {type: sensor, entity_id: sensor.temperature, title: Living room, icon: thermometer}
- {type: sensor, entity_id: sensor.energy_today, title: Energy, icon: zap, show_graph: true}
- {type: light, entity_id: light.ceiling, title: Ceiling, show_brightness: true}
- {type: switch, entity_id: switch.air_purifier, title: Air purifier}
- {type: scene, entity_id: scene.movie, title: Movie, icon: film}
- {type: climate, entity_id: climate.living_room, title: Living room}
- {type: weather, entity_id: weather.home, forecast_days: 5}
- {type: media_player, entity_id: media_player.speaker}
- type: sensor_grid
  title: Climate
  sensors:
    - {entity_id: sensor.temperature, title: Temp, icon: thermometer}
    - {entity_id: sensor.humidity, title: Humidity, icon: droplet}
    - {entity_id: sensor.co2, title: CO2, icon: leaf}
    - {entity_id: sensor.power, title: Power, icon: zap}
- {type: statistics, entity_id: sensor.energy_today, title: Energy, period: day, days: 14, stat: change}
- {type: statistics, entity_id: sensor.energy_today, title: Energy, period: day, days: 14, stat: change, chart: hbar}
- {type: statistics, entity_id: sensor.energy_today, title: Energy, period: day, days: 14, stat: change, chart: line}
- {type: progress, entity_id: sensor.print_progress, title: Print, icon: printer, secondary_entity: sensor.print_end}
- {type: cover, entity_id: cover.living_room, title: Living room blinds}
- {type: climate, entity_id: climate.living_room, title: Living room, show_graph: true}
- type: breakdown
  title: Power now
  entities:
    - {entity_id: sensor.power, title: Kitchen}
    - {entity_id: sensor.co2, title: Office}
    - {entity_id: sensor.humidity, title: Bedroom}
- type: glance
  title: House
  entities:
    - {entity_id: light.ceiling, title: Ceiling}
    - {entity_id: switch.air_purifier, title: Purifier}
    - {entity_id: sensor.temperature, title: Temp}
    - {entity_id: cover.living_room, title: Blinds}
"#,
        )
        .unwrap();
        let sizes: Vec<(f32, f32)> = std::env::var("CARD_SIZES")
            .unwrap_or("600x320,420x260,320x180,240x136,180x110,150x76".into())
            .split(',')
            .map(|s| {
                let (w, h) = s.split_once('x').unwrap();
                (w.parse().unwrap(), h.parse().unwrap())
            })
            .collect();
        // Never below what the grid allows (a cell may be SHORT_BY short)
        let size_of = |card: &CardConfig, (w, h): (f32, f32)| {
            let w = w.max(grid_min(card, 0.0).0);
            (w, h.max(grid_min(card, w).1 - SHORT_BY))
        };
        let width = sizes.iter().map(|s| s.0 + GAP).sum::<f32>() + GAP + 200.0;
        let row_h: Vec<f32> =
            cards.iter().map(|c| sizes.iter().map(|s| size_of(c, *s).1).fold(0.0, f32::max)).collect();
        let height = row_h.iter().map(|h| h + GAP).sum::<f32>() + GAP;
        let config = AppConfig::default();
        let mut renderer = ScreenRenderer::new(&config);
        let mut states = crate::ha::mock::generate_mock_states();
        let now = chrono::Utc::now().timestamp_millis() as f64;
        let rows: Vec<[f64; 4]> = (0..96)
            .map(|i| {
                let t = now - (96 - i) as f64 * 900_000.0;
                let target = if i < 30 { 25.0 } else { 26.0 };
                let room = target - 0.8 + ((i as f64) * 0.7).sin() * 1.2;
                [t, (room * 10.0f64).round() / 10.0, target, if room < target { 1.0 } else { 0.0 }]
            })
            .collect();
        if let Some(c) = states.get_mut("climate.living_room") {
            c.attributes.insert(crate::ui::widgets::climate_card::CLIMATE_HISTORY_ATTR.into(), serde_json::json!(rows));
        }
        let mut add = |id: &str, state: &str, attrs: serde_json::Value| {
            let attributes = attrs.as_object().unwrap().iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            states.insert(id.into(), EntityState { entity_id: id.into(), state: state.into(), attributes, last_changed: None, last_updated: None });
        };
        add("sensor.print_progress", "42", serde_json::json!({ "unit_of_measurement": "%" }));
        add("sensor.print_end", "2026-11-18T19:05:00+00:00", serde_json::json!({ "device_class": "timestamp" }));
        add("cover.living_room", "open", serde_json::json!({ "current_position": 60, "device_class": "shutter", "friendly_name": "Blinds" }));
        let bars: Vec<(f64, f64)> = (0..14).map(|i| (1.7e12 + i as f64 * 86.4e6, 3.0 + (i * 7 % 5) as f64)).collect();
        states.get_mut("sensor.energy_today").unwrap().attributes.insert(
            crate::ui::widgets::statistics_card::STATS_ATTR.into(),
            serde_json::json!({ crate::ui::widgets::statistics_card::stat_key("day", 14 * 24, "change"): bars }),
        );
        let mut canvas = Canvas::new(width as u32, height as u32);
        canvas.clear(renderer.theme.bg);
        let mut y = GAP;
        for (card, rh) in cards.iter().zip(&row_h) {
            let mut x = GAP;
            for &size in &sizes {
                let (w, h) = size_of(card, size);
                renderer.render_card(&mut canvas, card, Rect::new(x, y, w, h), &states, None);
                x += w + GAP;
            }
            y += rh + GAP;
        }
        std::fs::write(out, canvas.to_png_bytes().unwrap()).unwrap();
    }
}
