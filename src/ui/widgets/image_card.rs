//! A picture from a Home Assistant `image` entity.
//!
//! Home Assistant renders the picture (a chart, a snapshot) and serves it at
//! `/api/image_proxy/<entity_id>`; the client fetches the bytes and decodes them
//! once, so this widget only has to place them.

use crate::display::{Canvas, TextAlign};
use crate::ui::layout::{InteractiveTarget, Rect, UiAction};
use crate::ui::theme::Theme;
use crate::ui::widgets::{draw_card, type_scale, PAD};
use tiny_skia::Pixmap;

/// Height of the title row, when the card has one.
pub const TITLE_H: f32 = 44.0;

/// The part of the card the picture gets, in UI units.
pub fn picture_box(rect: Rect, titled: bool) -> Rect {
    let top = rect.y + PAD + if titled { TITLE_H } else { 0.0 };
    Rect::new(rect.x + PAD, top, rect.w - 2.0 * PAD, (rect.y + rect.h - PAD - top).max(1.0))
}

/// Draw the card and the picture, centred and as large as fits.
pub fn render_image_card(
    canvas: &mut Canvas,
    theme: &Theme,
    rect: Rect,
    entity_id: &str,
    title: Option<&str>,
    image: Option<&Pixmap>,
    targets: &mut Vec<InteractiveTarget>,
) {
    draw_card(canvas, theme, rect, None);
    let area = picture_box(rect, title.is_some());
    if let Some(title) = title {
        canvas.draw_text_fit(&title, rect.x + PAD, rect.y + PAD, type_scale::BODY, true, theme.text_muted, TextAlign::Left, area.w);
    }
    match image {
        Some(pixmap) => {
            // The pixmap is in device pixels; the layout is in UI units.
            let w = pixmap.width() as f32 / canvas.scale;
            let h = pixmap.height() as f32 / canvas.scale;
            let x = area.x + (area.w - w) / 2.0;
            let y = area.y + (area.h - h) / 2.0;
            canvas.draw_image(pixmap, x, y);
        }
        None => {
            canvas.draw_text_fit(
                "--",
                area.x + area.w / 2.0,
                area.y + area.h / 2.0 - type_scale::VALUE / 2.0,
                type_scale::VALUE,
                true,
                theme.text_muted,
                TextAlign::Center,
                area.w,
            );
            // Say which entity is missing, so a wrong id is visible on the device.
            canvas.draw_text_fit(
                entity_id,
                area.x + area.w / 2.0,
                area.y + area.h - 30.0,
                type_scale::LABEL,
                false,
                theme.text_faint,
                TextAlign::Center,
                area.w,
            );
        }
    }
    targets.push(InteractiveTarget { rect, action: UiAction::Refresh });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ThemeConfig;

    fn png(width: u32, height: u32, color: [u8; 3]) -> Vec<u8> {
        let mut img = image::RgbaImage::new(width, height);
        for px in img.pixels_mut() {
            *px = image::Rgba([color[0], color[1], color[2], 255]);
        }
        let mut out = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(img).write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    #[test]
    fn picture_keeps_its_aspect_ratio_inside_the_box() {
        let pixmap = crate::display::canvas::fit_image(&png(200, 100, [200, 60, 40]), 400, 400).expect("decodes");
        // 2:1 picture in a square box: twice as wide as high, never larger than the box
        assert_eq!(pixmap.width(), 400);
        assert_eq!(pixmap.height(), 200);

        let tall = crate::display::canvas::fit_image(&png(100, 400, [40, 60, 200]), 400, 300).expect("decodes");
        assert_eq!(tall.height(), 300);
        assert_eq!(tall.width(), 75);
    }

    #[test]
    fn rubbish_is_not_an_image() {
        assert!(crate::display::canvas::fit_image(b"not an image", 100, 100).is_none());
        assert!(crate::display::canvas::fit_image(&png(10, 10, [0, 0, 0]), 0, 100).is_none());
    }

    #[test]
    fn draws_the_picture_and_a_placeholder() {
        let theme = Theme::from_config(&ThemeConfig::default());
        let mut canvas = Canvas::new(800, 400);
        let rect = Rect::new(20.0, 20.0, 760.0, 340.0);
        let mut targets = Vec::new();
        render_image_card(&mut canvas, &theme, rect, "image.price", None, None, &mut targets);
        assert_eq!(targets.len(), 1);

        let pixmap = crate::display::canvas::fit_image(&png(400, 200, [10, 120, 30]), 712, 292).unwrap();
        let mut canvas = Canvas::new(800, 400);
        render_image_card(&mut canvas, &theme, rect, "image.price", Some("Stroomprijs"), Some(&pixmap), &mut targets);
        // The picture is drawn inside the card, and the title does not overlap it
        let data = canvas.raw_rgba_data();
        let green = (0..400 * 800)
            .filter(|i| {
                let p = i * 4;
                data[p] < 60 && data[p + 1] > 100 && data[p + 2] < 60
            })
            .count();
        assert!(green > 1000, "{} green pixels", green);
    }
}