use crate::config::SceneItemConfig;
use crate::display::{Canvas, TextAlign};
use crate::ui::layout::{InteractiveTarget, Rect, UiAction};
use crate::ui::theme::Theme;
use crate::ui::widgets::type_scale;

pub const SCENES_TITLE_H: f32 = 38.0;
const GAP: f32 = 16.0;

pub fn render_scenes_row(
    canvas: &mut Canvas,
    theme: &Theme,
    rect: Rect,
    custom_title: Option<&str>,
    scenes: &[SceneItemConfig],
    targets: &mut Vec<InteractiveTarget>,
) {
    if scenes.is_empty() {
        return;
    }

    let mut y = rect.y;
    if let Some(title) = custom_title {
        canvas.draw_text(title, rect.x + 4.0, y + 2.0, type_scale::BODY, true, theme.text_muted, TextAlign::Left);
        y += SCENES_TITLE_H;
    }

    let n = scenes.len() as f32;
    let item_w = (rect.w - GAP * (n - 1.0)) / n;
    let item_h = (rect.y + rect.h - y).max(72.0);
    let horizontal = item_h < 104.0 || scenes.len() == 1;

    for (idx, scene) in scenes.iter().enumerate() {
        let item = Rect::new(rect.x + idx as f32 * (item_w + GAP), y, item_w, item_h);
        let r = theme.card_radius;
        canvas.fill_rounded_rect(item.x, item.y, item.w, item.h, r, theme.surface);
        canvas.stroke_rounded_rect(item.x + 1.0, item.y + 1.0, item.w - 2.0, item.h - 2.0, r - 1.0, 2.0, theme.border);

        let icon_size = 40.0;
        if horizontal {
            let label_w = canvas.measure_text(&scene.title, type_scale::BODY, true).0;
            let total = (icon_size + 14.0 + label_w).min(item.w - 24.0);
            let sx = item.x + (item.w - total) / 2.0;
            let cy = item.y + item.h / 2.0;
            canvas.draw_icon(&scene.icon, sx, cy - icon_size / 2.0, icon_size, theme.accent.solid, 2.2);
            canvas.draw_text_fit(
                &scene.title,
                sx + icon_size + 14.0,
                cy - 13.0,
                type_scale::BODY,
                true,
                theme.text,
                TextAlign::Left,
                item.w - 24.0 - icon_size - 14.0,
            );
        } else {
            let cx = item.x + item.w / 2.0;
            let block = icon_size + 12.0 + 24.0;
            let top = item.y + (item.h - block) / 2.0;
            canvas.draw_icon(&scene.icon, cx - icon_size / 2.0, top, icon_size, theme.accent.solid, 2.2);
            canvas.draw_text_fit(
                &scene.title,
                cx,
                top + icon_size + 12.0,
                type_scale::LABEL,
                true,
                theme.text,
                TextAlign::Center,
                item.w - 20.0,
            );
        }

        targets.push(InteractiveTarget {
            rect: item,
            action: UiAction::ActivateScene(scene.entity_id.clone()),
        });
    }
}
