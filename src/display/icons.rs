//! Line icons drawn on a 24x24 grid (Lucide-style), plus duotone weather art.

use crate::display::color::Color;
use tiny_skia::{FillRule, LineCap, LineJoin, Paint, Path, PathBuilder, PixmapMut, Stroke, Transform};

enum Mode {
    Stroke,
    Fill,
}

fn paint(color: Color) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color(color.to_skia());
    paint.anti_alias = true;
    paint
}

fn stroke(width: f32) -> Stroke {
    Stroke {
        width,
        line_cap: LineCap::Round,
        line_join: LineJoin::Round,
        ..Default::default()
    }
}

fn build(f: impl FnOnce(&mut PathBuilder)) -> Option<Path> {
    let mut pb = PathBuilder::new();
    f(&mut pb);
    pb.finish()
}

fn rect(pb: &mut PathBuilder, x: f32, y: f32, w: f32, h: f32) {
    if let Some(r) = tiny_skia::Rect::from_xywh(x, y, w, h) {
        pb.push_rect(r);
    }
}

fn round_rect(pb: &mut PathBuilder, x: f32, y: f32, w: f32, h: f32, r: f32) {
    let k = 0.5523 * r;
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.cubic_to(x + w - r + k, y, x + w, y + r - k, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.cubic_to(x + w, y + h - r + k, x + w - r + k, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.cubic_to(x + r - k, y + h, x, y + h - r + k, x, y + h - r);
    pb.line_to(x, y + r);
    pb.cubic_to(x, y + r - k, x + r - k, y, x + r, y);
    pb.close();
}

fn cloud_path(pb: &mut PathBuilder) {
    pb.move_to(7.0, 19.0);
    pb.cubic_to(4.2, 19.0, 2.0, 16.9, 2.0, 14.2);
    pb.cubic_to(2.0, 11.7, 3.9, 9.7, 6.3, 9.5);
    pb.cubic_to(7.1, 6.6, 9.7, 4.5, 12.8, 4.5);
    pb.cubic_to(16.3, 4.5, 19.2, 7.2, 19.5, 10.6);
    pb.cubic_to(21.5, 11.0, 23.0, 12.8, 23.0, 14.9);
    pb.cubic_to(23.0, 17.2, 21.2, 19.0, 18.9, 19.0);
    pb.close();
}

fn icon_shapes(name: &str) -> Vec<(Mode, Option<Path>)> {
    use Mode::*;
    match name {
        "lightbulb" | "light" => vec![(Stroke, build(|pb| {
            pb.move_to(9.0, 18.0);
            pb.line_to(15.0, 18.0);
            pb.move_to(10.0, 21.5);
            pb.line_to(14.0, 21.5);
            pb.move_to(9.0, 14.5);
            pb.cubic_to(9.0, 12.5, 5.5, 11.0, 5.5, 8.5);
            pb.cubic_to(5.5, 4.9, 8.4, 2.0, 12.0, 2.0);
            pb.cubic_to(15.6, 2.0, 18.5, 4.9, 18.5, 8.5);
            pb.cubic_to(18.5, 11.0, 15.0, 12.5, 15.0, 14.5);
            pb.close();
        }))],
        "lightbulb_on" => vec![
            (Fill, build(|pb| {
                pb.move_to(9.0, 14.5);
                pb.cubic_to(9.0, 12.5, 5.5, 11.0, 5.5, 8.5);
                pb.cubic_to(5.5, 4.9, 8.4, 2.0, 12.0, 2.0);
                pb.cubic_to(15.6, 2.0, 18.5, 4.9, 18.5, 8.5);
                pb.cubic_to(18.5, 11.0, 15.0, 12.5, 15.0, 14.5);
                pb.close();
            })),
            (Stroke, build(|pb| {
                pb.move_to(9.0, 18.0);
                pb.line_to(15.0, 18.0);
                pb.move_to(10.0, 21.5);
                pb.line_to(14.0, 21.5);
            })),
        ],
        "thermometer" | "temperature" => vec![(Stroke, build(|pb| {
            pb.move_to(14.0, 14.8);
            pb.line_to(14.0, 4.5);
            pb.cubic_to(14.0, 3.1, 13.1, 2.0, 12.0, 2.0);
            pb.cubic_to(10.9, 2.0, 10.0, 3.1, 10.0, 4.5);
            pb.line_to(10.0, 14.8);
            pb.cubic_to(8.8, 15.5, 8.0, 16.8, 8.0, 18.2);
            pb.cubic_to(8.0, 20.3, 9.8, 22.0, 12.0, 22.0);
            pb.cubic_to(14.2, 22.0, 16.0, 20.3, 16.0, 18.2);
            pb.cubic_to(16.0, 16.8, 15.2, 15.5, 14.0, 14.8);
            pb.close();
            pb.move_to(12.0, 9.0);
            pb.line_to(12.0, 17.0);
        }))],
        "sun" | "sunny" => vec![(Stroke, build(|pb| {
            pb.push_circle(12.0, 12.0, 4.5);
            for (x1, y1, x2, y2) in [
                (12.0, 1.5, 12.0, 3.5),
                (12.0, 20.5, 12.0, 22.5),
                (1.5, 12.0, 3.5, 12.0),
                (20.5, 12.0, 22.5, 12.0),
                (4.6, 4.6, 6.0, 6.0),
                (18.0, 18.0, 19.4, 19.4),
                (4.6, 19.4, 6.0, 18.0),
                (18.0, 6.0, 19.4, 4.6),
            ] {
                pb.move_to(x1, y1);
                pb.line_to(x2, y2);
            }
        }))],
        "moon" => vec![(Stroke, build(|pb| {
            pb.move_to(20.5, 14.5);
            pb.cubic_to(19.3, 18.3, 15.8, 21.0, 11.7, 21.0);
            pb.cubic_to(6.6, 21.0, 2.5, 16.9, 2.5, 11.8);
            pb.cubic_to(2.5, 7.7, 5.2, 4.2, 9.0, 3.0);
            pb.cubic_to(8.4, 4.2, 8.1, 5.5, 8.1, 6.9);
            pb.cubic_to(8.1, 11.9, 12.1, 15.9, 17.1, 15.9);
            pb.cubic_to(18.3, 15.9, 19.5, 15.4, 20.5, 14.5);
            pb.close();
        }))],
        "cloud" | "cloudy" => vec![(Stroke, build(cloud_path))],
        "partlycloudy" | "cloud_sun" => vec![(Stroke, build(|pb| {
            pb.move_to(8.0, 2.0);
            pb.line_to(8.0, 3.5);
            pb.move_to(2.0, 8.0);
            pb.line_to(3.5, 8.0);
            pb.move_to(3.8, 3.8);
            pb.line_to(4.8, 4.8);
            pb.move_to(12.2, 3.8);
            pb.line_to(11.2, 4.8);
            pb.move_to(5.2, 11.6);
            pb.cubic_to(4.4, 10.8, 4.0, 9.7, 4.0, 8.5);
            pb.cubic_to(4.0, 6.0, 6.0, 4.0, 8.5, 4.0);
            pb.cubic_to(10.2, 4.0, 11.7, 4.9, 12.5, 6.4);
            pb.move_to(9.0, 21.0);
            pb.cubic_to(6.8, 21.0, 5.0, 19.3, 5.0, 17.1);
            pb.cubic_to(5.0, 15.1, 6.5, 13.5, 8.4, 13.3);
            pb.cubic_to(9.1, 10.8, 11.3, 9.0, 13.9, 9.0);
            pb.cubic_to(16.9, 9.0, 19.3, 11.3, 19.5, 14.2);
            pb.cubic_to(21.0, 14.5, 22.5, 15.9, 22.5, 17.7);
            pb.cubic_to(22.5, 19.5, 21.0, 21.0, 19.2, 21.0);
            pb.close();
        }))],
        "rain" | "rainy" | "cloud_rain" => vec![(Stroke, build(|pb| {
            pb.move_to(6.5, 16.0);
            pb.cubic_to(4.0, 15.6, 2.0, 13.6, 2.0, 11.2);
            pb.cubic_to(2.0, 8.7, 3.9, 6.7, 6.3, 6.5);
            pb.cubic_to(7.1, 3.6, 9.7, 1.5, 12.8, 1.5);
            pb.cubic_to(16.3, 1.5, 19.2, 4.2, 19.5, 7.6);
            pb.cubic_to(21.5, 8.0, 23.0, 9.8, 23.0, 11.9);
            pb.cubic_to(23.0, 13.9, 21.6, 15.5, 19.7, 15.9);
            for x in [8.0, 12.5, 17.0] {
                pb.move_to(x, 14.0);
                pb.line_to(x - 1.5, 21.5);
            }
        }))],
        "droplet" | "humidity" => vec![(Stroke, build(|pb| {
            pb.move_to(12.0, 2.5);
            pb.cubic_to(12.0, 2.5, 5.0, 10.5, 5.0, 15.0);
            pb.cubic_to(5.0, 18.9, 8.1, 22.0, 12.0, 22.0);
            pb.cubic_to(15.9, 22.0, 19.0, 18.9, 19.0, 15.0);
            pb.cubic_to(19.0, 10.5, 12.0, 2.5, 12.0, 2.5);
            pb.close();
        }))],
        "wind" => vec![(Stroke, build(|pb| {
            pb.move_to(2.5, 8.0);
            pb.line_to(14.5, 8.0);
            pb.cubic_to(16.4, 8.0, 18.0, 6.4, 18.0, 4.8);
            pb.cubic_to(18.0, 3.1, 16.4, 2.0, 14.8, 2.5);
            pb.move_to(2.5, 12.0);
            pb.line_to(19.0, 12.0);
            pb.cubic_to(20.9, 12.0, 22.0, 13.4, 22.0, 15.0);
            pb.cubic_to(22.0, 16.8, 20.4, 18.2, 18.5, 17.6);
            pb.move_to(2.5, 16.0);
            pb.line_to(11.0, 16.0);
            pb.cubic_to(12.7, 16.0, 14.0, 17.3, 14.0, 19.0);
            pb.cubic_to(14.0, 20.7, 12.4, 21.8, 10.8, 21.3);
        }))],
        "zap" | "energy" | "bolt" => vec![(Fill, build(|pb| {
            pb.move_to(13.5, 2.0);
            pb.line_to(4.5, 13.5);
            pb.line_to(11.5, 13.5);
            pb.line_to(10.5, 22.0);
            pb.line_to(19.5, 10.5);
            pb.line_to(12.5, 10.5);
            pb.close();
        }))],
        "power" => vec![(Stroke, build(|pb| {
            pb.move_to(12.0, 2.5);
            pb.line_to(12.0, 11.5);
            pb.move_to(7.0, 5.5);
            pb.cubic_to(4.5, 7.2, 3.0, 10.0, 3.0, 13.0);
            pb.cubic_to(3.0, 18.0, 7.0, 22.0, 12.0, 22.0);
            pb.cubic_to(17.0, 22.0, 21.0, 18.0, 21.0, 13.0);
            pb.cubic_to(21.0, 10.0, 19.5, 7.2, 17.0, 5.5);
        }))],
        "plug" => vec![(Stroke, build(|pb| {
            pb.move_to(9.0, 2.5);
            pb.line_to(9.0, 7.5);
            pb.move_to(15.0, 2.5);
            pb.line_to(15.0, 7.5);
            pb.move_to(5.5, 7.5);
            pb.line_to(18.5, 7.5);
            pb.line_to(18.5, 11.5);
            pb.cubic_to(18.5, 15.1, 15.6, 18.0, 12.0, 18.0);
            pb.cubic_to(8.4, 18.0, 5.5, 15.1, 5.5, 11.5);
            pb.close();
            pb.move_to(12.0, 18.0);
            pb.line_to(12.0, 22.0);
        }))],
        "fan" => vec![(Stroke, build(|pb| {
            pb.push_circle(12.0, 12.0, 2.0);
            pb.move_to(12.0, 10.0);
            pb.cubic_to(9.0, 6.0, 10.0, 2.0, 13.0, 2.0);
            pb.cubic_to(16.0, 2.0, 16.5, 6.0, 12.0, 10.0);
            pb.move_to(14.0, 12.0);
            pb.cubic_to(18.0, 9.0, 22.0, 10.0, 22.0, 13.0);
            pb.cubic_to(22.0, 16.0, 18.0, 16.5, 14.0, 12.0);
            pb.move_to(12.0, 14.0);
            pb.cubic_to(15.0, 18.0, 14.0, 22.0, 11.0, 22.0);
            pb.cubic_to(8.0, 22.0, 7.5, 18.0, 12.0, 14.0);
            pb.move_to(10.0, 12.0);
            pb.cubic_to(6.0, 15.0, 2.0, 14.0, 2.0, 11.0);
            pb.cubic_to(2.0, 8.0, 6.0, 7.5, 10.0, 12.0);
        }))],
        "snowflake" | "snow" | "cool" => vec![(Stroke, build(|pb| {
            for (dx, dy) in [(0.0f32, 10.0f32), (8.66, 5.0), (8.66, -5.0)] {
                pb.move_to(12.0 - dx, 12.0 - dy);
                pb.line_to(12.0 + dx, 12.0 + dy);
            }
            for (x, y, s) in [(12.0, 4.0, 1.0), (12.0, 20.0, -1.0)] {
                pb.move_to(x - 2.5, y - 1.5 * s);
                pb.line_to(x, y + 1.0 * s);
                pb.line_to(x + 2.5, y - 1.5 * s);
            }
        }))],
        "flame" | "fire" | "heat" => vec![(Stroke, build(|pb| {
            pb.move_to(12.0, 22.0);
            pb.cubic_to(8.1, 22.0, 5.0, 19.0, 5.0, 15.2);
            pb.cubic_to(5.0, 11.0, 8.5, 9.0, 9.5, 5.5);
            pb.cubic_to(11.5, 7.0, 12.0, 9.0, 11.5, 11.0);
            pb.cubic_to(13.5, 9.5, 14.5, 6.0, 13.5, 2.5);
            pb.cubic_to(17.0, 4.5, 19.0, 9.5, 19.0, 14.8);
            pb.cubic_to(19.0, 19.0, 15.9, 22.0, 12.0, 22.0);
            pb.close();
        }))],
        "wifi" => vec![
            (Stroke, build(|pb| {
                pb.move_to(2.0, 8.8);
                pb.cubic_to(7.6, 3.7, 16.4, 3.7, 22.0, 8.8);
                pb.move_to(5.2, 12.6);
                pb.cubic_to(9.0, 9.2, 15.0, 9.2, 18.8, 12.6);
                pb.move_to(8.6, 16.3);
                pb.cubic_to(10.5, 14.6, 13.5, 14.6, 15.4, 16.3);
            })),
            (Fill, build(|pb| pb.push_circle(12.0, 19.8, 1.6))),
        ],
        "wifi_off" => vec![
            (Stroke, build(|pb| {
                pb.move_to(8.6, 16.3);
                pb.cubic_to(10.5, 14.6, 13.5, 14.6, 15.4, 16.3);
                pb.move_to(5.2, 12.6);
                pb.cubic_to(6.4, 11.5, 7.8, 10.8, 9.3, 10.4);
                pb.move_to(2.0, 8.8);
                pb.cubic_to(3.4, 7.5, 5.0, 6.6, 6.7, 5.9);
                pb.move_to(13.0, 4.6);
                pb.cubic_to(16.3, 4.8, 19.5, 6.2, 22.0, 8.8);
                pb.move_to(3.0, 2.5);
                pb.line_to(21.0, 21.5);
            })),
            (Fill, build(|pb| pb.push_circle(12.0, 19.8, 1.6))),
        ],
        "plus" => vec![(Stroke, build(|pb| {
            pb.move_to(12.0, 5.0);
            pb.line_to(12.0, 19.0);
            pb.move_to(5.0, 12.0);
            pb.line_to(19.0, 12.0);
        }))],
        "minus" => vec![(Stroke, build(|pb| {
            pb.move_to(5.0, 12.0);
            pb.line_to(19.0, 12.0);
        }))],
        "check" => vec![(Stroke, build(|pb| {
            pb.move_to(4.5, 12.5);
            pb.line_to(9.5, 17.5);
            pb.line_to(19.5, 6.5);
        }))],
        "x" | "close" => vec![(Stroke, build(|pb| {
            pb.move_to(6.0, 6.0);
            pb.line_to(18.0, 18.0);
            pb.move_to(18.0, 6.0);
            pb.line_to(6.0, 18.0);
        }))],
        "play" => vec![(Fill, build(|pb| {
            pb.move_to(7.5, 4.5);
            pb.line_to(19.5, 12.0);
            pb.line_to(7.5, 19.5);
            pb.close();
        }))],
        "pause" => vec![(Fill, build(|pb| {
            round_rect(pb, 6.0, 4.5, 4.0, 15.0, 1.0);
            round_rect(pb, 14.0, 4.5, 4.0, 15.0, 1.0);
        }))],
        "skip_next" => vec![(Fill, build(|pb| {
            pb.move_to(5.0, 5.0);
            pb.line_to(15.0, 12.0);
            pb.line_to(5.0, 19.0);
            pb.close();
            round_rect(pb, 16.5, 5.0, 3.0, 14.0, 1.0);
        }))],
        "skip_previous" => vec![(Fill, build(|pb| {
            pb.move_to(19.0, 5.0);
            pb.line_to(9.0, 12.0);
            pb.line_to(19.0, 19.0);
            pb.close();
            round_rect(pb, 4.5, 5.0, 3.0, 14.0, 1.0);
        }))],
        "home" => vec![(Stroke, build(|pb| {
            pb.move_to(3.0, 10.0);
            pb.line_to(12.0, 3.0);
            pb.line_to(21.0, 10.0);
            pb.line_to(21.0, 20.0);
            pb.cubic_to(21.0, 20.6, 20.6, 21.0, 20.0, 21.0);
            pb.line_to(15.0, 21.0);
            pb.line_to(15.0, 14.0);
            pb.line_to(9.0, 14.0);
            pb.line_to(9.0, 21.0);
            pb.line_to(4.0, 21.0);
            pb.cubic_to(3.4, 21.0, 3.0, 20.6, 3.0, 20.0);
            pb.close();
        }))],
        "refresh" => vec![(Stroke, build(|pb| {
            pb.move_to(20.0, 12.0);
            pb.cubic_to(20.0, 16.4, 16.4, 20.0, 12.0, 20.0);
            pb.cubic_to(9.6, 20.0, 7.5, 19.0, 6.0, 17.3);
            pb.move_to(4.0, 12.0);
            pb.cubic_to(4.0, 7.6, 7.6, 4.0, 12.0, 4.0);
            pb.cubic_to(14.4, 4.0, 16.5, 5.0, 18.0, 6.7);
            pb.move_to(18.5, 2.5);
            pb.line_to(18.5, 7.0);
            pb.line_to(14.0, 7.0);
            pb.move_to(5.5, 21.5);
            pb.line_to(5.5, 17.0);
            pb.line_to(10.0, 17.0);
        }))],
        "logout" | "exit" => vec![(Stroke, build(|pb| {
            pb.move_to(9.5, 21.0);
            pb.line_to(5.0, 21.0);
            pb.cubic_to(3.9, 21.0, 3.0, 20.1, 3.0, 19.0);
            pb.line_to(3.0, 5.0);
            pb.cubic_to(3.0, 3.9, 3.9, 3.0, 5.0, 3.0);
            pb.line_to(9.5, 3.0);
            pb.move_to(10.0, 12.0);
            pb.line_to(21.0, 12.0);
            pb.move_to(16.5, 7.5);
            pb.line_to(21.0, 12.0);
            pb.line_to(16.5, 16.5);
        }))],
        "music" | "speaker" | "media" => vec![
            (Stroke, build(|pb| {
                pb.move_to(9.0, 18.0);
                pb.line_to(9.0, 5.0);
                pb.line_to(21.0, 3.0);
                pb.line_to(21.0, 16.0);
            })),
            (Fill, build(|pb| {
                pb.push_circle(6.0, 18.0, 3.0);
                pb.push_circle(18.0, 16.0, 3.0);
            })),
        ],
        "tv" => vec![(Stroke, build(|pb| {
            round_rect(pb, 2.0, 6.0, 20.0, 14.0, 2.0);
            pb.move_to(7.0, 2.0);
            pb.line_to(12.0, 6.0);
            pb.line_to(17.0, 2.0);
        }))],
        "coffee" => vec![(Stroke, build(|pb| {
            pb.move_to(17.0, 8.0);
            pb.line_to(17.0, 15.0);
            pb.cubic_to(17.0, 17.8, 14.8, 20.0, 12.0, 20.0);
            pb.line_to(8.0, 20.0);
            pb.cubic_to(5.2, 20.0, 3.0, 17.8, 3.0, 15.0);
            pb.line_to(3.0, 8.0);
            pb.close();
            pb.move_to(17.0, 9.0);
            pb.line_to(18.5, 9.0);
            pb.cubic_to(20.4, 9.0, 22.0, 10.6, 22.0, 12.5);
            pb.cubic_to(22.0, 14.4, 20.4, 16.0, 18.5, 16.0);
            pb.line_to(17.0, 16.0);
            pb.move_to(7.0, 2.0);
            pb.line_to(7.0, 4.5);
            pb.move_to(11.0, 2.0);
            pb.line_to(11.0, 4.5);
        }))],
        "lock" => vec![(Stroke, build(|pb| {
            round_rect(pb, 4.0, 11.0, 16.0, 11.0, 2.0);
            pb.move_to(7.5, 11.0);
            pb.line_to(7.5, 7.0);
            pb.cubic_to(7.5, 4.5, 9.5, 2.5, 12.0, 2.5);
            pb.cubic_to(14.5, 2.5, 16.5, 4.5, 16.5, 7.0);
            pb.line_to(16.5, 11.0);
        }))],
        "door" => vec![(Stroke, build(|pb| {
            pb.move_to(3.0, 21.0);
            pb.line_to(21.0, 21.0);
            pb.move_to(6.0, 21.0);
            pb.line_to(6.0, 4.0);
            pb.cubic_to(6.0, 2.9, 6.9, 2.0, 8.0, 2.0);
            pb.line_to(16.0, 2.0);
            pb.cubic_to(17.1, 2.0, 18.0, 2.9, 18.0, 4.0);
            pb.line_to(18.0, 21.0);
            pb.move_to(14.5, 12.0);
            pb.line_to(14.5, 12.5);
        }))],
        "sofa" | "couch" => vec![(Stroke, build(|pb| {
            pb.move_to(5.0, 10.0);
            pb.line_to(5.0, 7.0);
            pb.cubic_to(5.0, 5.3, 6.3, 4.0, 8.0, 4.0);
            pb.line_to(16.0, 4.0);
            pb.cubic_to(17.7, 4.0, 19.0, 5.3, 19.0, 7.0);
            pb.line_to(19.0, 10.0);
            pb.move_to(3.0, 18.0);
            pb.line_to(3.0, 12.0);
            pb.cubic_to(3.0, 10.9, 3.9, 10.0, 5.0, 10.0);
            pb.cubic_to(6.1, 10.0, 7.0, 10.9, 7.0, 12.0);
            pb.line_to(7.0, 14.0);
            pb.line_to(17.0, 14.0);
            pb.line_to(17.0, 12.0);
            pb.cubic_to(17.0, 10.9, 17.9, 10.0, 19.0, 10.0);
            pb.cubic_to(20.1, 10.0, 21.0, 10.9, 21.0, 12.0);
            pb.line_to(21.0, 18.0);
            pb.close();
            pb.move_to(5.0, 18.0);
            pb.line_to(5.0, 20.5);
            pb.move_to(19.0, 18.0);
            pb.line_to(19.0, 20.5);
        }))],
        "bed" => vec![(Stroke, build(|pb| {
            pb.move_to(2.0, 4.0);
            pb.line_to(2.0, 20.0);
            pb.move_to(2.0, 16.0);
            pb.line_to(22.0, 16.0);
            pb.line_to(22.0, 20.0);
            pb.move_to(2.0, 11.0);
            pb.line_to(19.0, 11.0);
            pb.cubic_to(20.7, 11.0, 22.0, 12.3, 22.0, 14.0);
            pb.line_to(22.0, 16.0);
            pb.move_to(6.0, 11.0);
            pb.line_to(6.0, 8.0);
        }))],
        "printer" | "3d_printer" => vec![(Stroke, build(|pb| {
            round_rect(pb, 3.0, 2.5, 18.0, 19.0, 2.0);
            pb.move_to(3.0, 6.5);
            pb.line_to(21.0, 6.5);
            pb.move_to(9.0, 6.5);
            pb.line_to(9.0, 9.5);
            pb.line_to(12.0, 12.0);
            pb.line_to(15.0, 9.5);
            pb.line_to(15.0, 6.5);
            pb.move_to(7.0, 18.0);
            pb.line_to(17.0, 18.0);
            pb.move_to(9.0, 15.5);
            pb.line_to(15.0, 15.5);
        }))],
        "battery" => vec![(Stroke, build(|pb| {
            round_rect(pb, 2.0, 7.0, 17.0, 10.0, 2.0);
            pb.move_to(22.0, 11.0);
            pb.line_to(22.0, 13.0);
            pb.move_to(6.0, 10.5);
            pb.line_to(6.0, 13.5);
            pb.move_to(10.0, 10.5);
            pb.line_to(10.0, 13.5);
        }))],
        "clock" | "timer" => vec![(Stroke, build(|pb| {
            pb.push_circle(12.0, 12.0, 9.5);
            pb.move_to(12.0, 6.5);
            pb.line_to(12.0, 12.0);
            pb.line_to(15.5, 14.0);
        }))],
        "layers" => vec![(Stroke, build(|pb| {
            pb.move_to(12.0, 2.5);
            pb.line_to(22.0, 7.5);
            pb.line_to(12.0, 12.5);
            pb.line_to(2.0, 7.5);
            pb.close();
            pb.move_to(2.0, 12.0);
            pb.line_to(12.0, 17.0);
            pb.line_to(22.0, 12.0);
            pb.move_to(2.0, 16.5);
            pb.line_to(12.0, 21.5);
            pb.line_to(22.0, 16.5);
        }))],
        "progress" | "percent" => vec![(Stroke, build(|pb| {
            pb.move_to(19.0, 5.0);
            pb.line_to(5.0, 19.0);
            pb.push_circle(6.5, 6.5, 2.5);
            pb.push_circle(17.5, 17.5, 2.5);
        }))],
        "washer" | "washing_machine" => vec![(Stroke, build(|pb| {
            round_rect(pb, 3.5, 2.0, 17.0, 20.0, 2.0);
            pb.push_circle(12.0, 13.5, 5.0);
            pb.move_to(7.0, 5.5);
            pb.line_to(8.0, 5.5);
            pb.move_to(11.0, 5.5);
            pb.line_to(12.0, 5.5);
        }))],
        _ => vec![(Stroke, build(|pb| pb.push_circle(12.0, 12.0, 8.0)))],
    }
}

/// Draw a named line icon whose 24x24 box has its top-left corner at (x, y).
/// `stroke_width` is in 24-grid units (2.0 matches Lucide).
pub fn draw_icon(pixmap: &mut PixmapMut, name: &str, x: f32, y: f32, size: f32, color: Color, stroke_width: f32) {
    let scale = size / 24.0;
    let transform = Transform::from_scale(scale, scale).post_translate(x, y);
    let paint = paint(color);
    let stroke = stroke(stroke_width);
    for (mode, path) in icon_shapes(name) {
        let Some(path) = path else { continue };
        match mode {
            Mode::Stroke => pixmap.stroke_path(&path, &paint, &stroke, transform, None),
            Mode::Fill => pixmap.fill_path(&path, &paint, FillRule::Winding, transform, None),
        }
    }
}

/// Colours used for the duotone weather illustrations.
#[derive(Debug, Clone, Copy)]
pub struct WeatherInk {
    pub line: Color,
    pub sun: Color,
    pub cloud: Color,
    pub rain: Color,
}

/// Larger, filled weather illustration keyed on the HA condition string.
pub fn draw_weather_art(pixmap: &mut PixmapMut, condition: &str, x: f32, y: f32, size: f32, ink: WeatherInk) {
    let scale = size / 24.0;
    let t = Transform::from_scale(scale, scale).post_translate(x, y);
    let line_w = (2.4f32).min(1.6 + 12.0 / size);
    let s = stroke(line_w);

    let fill = |pm: &mut PixmapMut, path: Option<Path>, color: Color, tr: Transform| {
        if let Some(p) = path {
            pm.fill_path(&p, &paint(color), FillRule::Winding, tr, None);
        }
    };
    let outline = |pm: &mut PixmapMut, path: Option<Path>, color: Color, tr: Transform| {
        if let Some(p) = path {
            pm.stroke_path(&p, &paint(color), &s, tr, None);
        }
    };

    let sun_at = |pm: &mut PixmapMut, cx: f32, cy: f32, r: f32| {
        fill(pm, build(|pb| pb.push_circle(cx, cy, r)), ink.sun, t);
        outline(
            pm,
            build(|pb| {
                for i in 0..8 {
                    let a = i as f32 * std::f32::consts::FRAC_PI_4;
                    let (sx, sy) = (a.cos(), a.sin());
                    pb.move_to(cx + sx * (r + 2.0), cy + sy * (r + 2.0));
                    pb.line_to(cx + sx * (r + 3.8), cy + sy * (r + 3.8));
                }
            }),
            ink.sun,
            t,
        );
    };
    let cloud_at = |pm: &mut PixmapMut, dx: f32, dy: f32, k: f32| {
        let tr = Transform::from_scale(k, k).post_translate(dx, dy).post_concat(t);
        fill(pm, build(cloud_path), ink.cloud, tr);
        if let Some(p) = build(cloud_path) {
            let ks = stroke(line_w / k);
            pm.stroke_path(&p, &paint(ink.line), &ks, tr, None);
        }
    };

    match condition {
        "sunny" => sun_at(pixmap, 12.0, 12.0, 6.0),
        "clear-night" => {
            if let Some(p) = icon_shapes("moon").into_iter().next().and_then(|(_, p)| p) {
                pixmap.fill_path(&p, &paint(ink.sun), FillRule::Winding, t, None);
                pixmap.stroke_path(&p, &paint(ink.line), &s, t, None);
            }
        }
        "partlycloudy" => {
            sun_at(pixmap, 8.5, 8.0, 4.2);
            cloud_at(pixmap, 4.0, 5.5, 0.82);
        }
        "rainy" | "pouring" | "snowy" | "snowy-rainy" | "hail" => {
            cloud_at(pixmap, 0.5, -2.0, 0.95);
            let snow = condition.starts_with("snowy");
            outline(
                pixmap,
                build(|pb| {
                    for (i, xx) in [7.5f32, 12.0, 16.5].iter().enumerate() {
                        if snow && i != 1 {
                            pb.push_circle(*xx - 0.8, 20.5, 0.9);
                        } else {
                            pb.move_to(*xx, 18.5);
                            pb.line_to(*xx - 1.5, 23.0);
                        }
                    }
                }),
                ink.rain,
                t,
            );
        }
        "lightning" | "lightning-rainy" => {
            cloud_at(pixmap, 0.5, -2.5, 0.95);
            fill(
                pixmap,
                build(|pb| {
                    pb.move_to(13.0, 14.0);
                    pb.line_to(9.0, 19.5);
                    pb.line_to(12.0, 19.5);
                    pb.line_to(10.5, 24.0);
                    pb.line_to(15.5, 17.5);
                    pb.line_to(12.5, 17.5);
                    pb.line_to(14.0, 14.0);
                    pb.close();
                }),
                ink.sun,
                t,
            );
        }
        "fog" => {
            cloud_at(pixmap, 0.5, -3.0, 0.9);
            outline(
                pixmap,
                build(|pb| {
                    pb.move_to(4.0, 18.5);
                    pb.line_to(20.0, 18.5);
                    pb.move_to(7.0, 22.0);
                    pb.line_to(17.0, 22.0);
                }),
                ink.line,
                t,
            );
        }
        "windy" | "windy-variant" => draw_icon(pixmap, "wind", x, y, size, ink.line, line_w),
        _ => cloud_at(pixmap, 0.0, 0.0, 1.0),
    }
}
