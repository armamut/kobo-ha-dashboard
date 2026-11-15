use crate::display::{Canvas, TextAlign};
use crate::ui::layout::Rect;
use crate::ui::theme::Theme;
use crate::ui::widgets::{draw_icon_badge, type_scale};

/// Height of a heading row, as used by the layout.
pub const HEADING_H: f32 = 64.0;

/// A section title, from a Lovelace `heading` card. No card surface: it
/// groups the cards under it.
pub fn render_heading_card(canvas: &mut Canvas, theme: &Theme, rect: Rect, title: &str, icon: Option<&str>) {
    let mut x = rect.x;
    let badge = 40.0;
    if let Some(icon) = icon {
        draw_icon_badge(canvas, theme, x, rect.y + (rect.h - badge) / 2.0, badge, icon, theme.accent, true);
        x += badge + 14.0;
    }
    canvas.draw_text_fit(
        title,
        x,
        rect.y + rect.h / 2.0 - type_scale::TITLE / 2.0 - 2.0,
        type_scale::TITLE,
        true,
        theme.text,
        TextAlign::Left,
        (rect.x + rect.w - x).max(40.0),
    );
}