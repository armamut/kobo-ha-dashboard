use crate::display::color::Color;
use crate::display::icons::{draw_icon, draw_weather_art, WeatherInk};
use ab_glyph::{Font as _, ScaleFont as _};
use fontdue::{Font, FontSettings};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock, RwLock};
use tiny_skia::{FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Rect, Stroke, Transform};

static FONT_REGULAR_BYTES: &[u8] = include_bytes!("../../assets/fonts/Roboto-Regular.ttf");
static FONT_BOLD_BYTES: &[u8] = include_bytes!("../../assets/fonts/Roboto-Bold.ttf");

static FONTS: OnceLock<(Font, Font)> = OnceLock::new();

fn get_fonts() -> &'static (Font, Font) {
    FONTS.get_or_init(|| {
        let regular = Font::from_bytes(FONT_REGULAR_BYTES, FontSettings::default())
            .expect("Failed to parse embedded Roboto-Regular font");
        let bold = Font::from_bytes(FONT_BOLD_BYTES, FontSettings::default())
            .expect("Failed to parse embedded Roboto-Bold font");
        (regular, bold)
    })
}

/// Optional font for scripts Roboto lacks (CJK). Parsed lazily by ab_glyph,
/// so even multi-megabyte Noto fonts load instantly.
struct Fallback {
    regular: ab_glyph::FontVec,
    bold: Option<ab_glyph::FontVec>,
}

static FALLBACK: RwLock<Option<Arc<Fallback>>> = RwLock::new(None);

fn read_font(dirs: &[PathBuf], names: &[String]) -> Option<ab_glyph::FontVec> {
    for dir in dirs {
        for name in names {
            for ext in ["otf", "ttf"] {
                let path = dir.join(format!("{}.{}", name, ext));
                if let Ok(bytes) = std::fs::read(&path) {
                    match ab_glyph::FontVec::try_from_vec(bytes) {
                        Ok(font) => {
                            tracing::info!("Loaded fallback font {:?}", path);
                            return Some(font);
                        }
                        Err(e) => tracing::warn!("Invalid font {:?}: {}", path, e),
                    }
                }
            }
        }
    }
    None
}

/// Load `<family>-Regular` / `<family>-Bold` (or `fallback-Regular` /
/// `fallback-Bold`) from the first directory that has them.
pub fn load_fallback_font(dirs: &[PathBuf], family: Option<&str>) {
    let mut families: Vec<String> = family.map(|f| vec![f.to_string()]).unwrap_or_default();
    families.push("fallback".to_string());
    let with = |suffix: &str| families.iter().map(|f| format!("{}-{}", f, suffix)).collect::<Vec<_>>();
    let fallback = read_font(dirs, &with("Regular")).map(|regular| Fallback { regular, bold: read_font(dirs, &with("Bold")) });
    if let Ok(mut slot) = FALLBACK.write() {
        *slot = fallback.map(Arc::new);
    }
}

pub fn has_fallback_font() -> bool {
    FALLBACK.read().map(|f| f.is_some()).unwrap_or(false)
}

fn fallback() -> Option<Arc<Fallback>> {
    FALLBACK.read().ok().and_then(|f| f.clone())
}

/// A run of characters drawn with one font.
enum Glyph {
    Primary(char),
    Fallback(ab_glyph::GlyphId),
}

fn resolve(c: char, primary: &Font, fb: Option<&ab_glyph::FontVec>) -> Glyph {
    if primary.lookup_glyph_index(c) != 0 || c.is_whitespace() {
        return Glyph::Primary(c);
    }
    match fb {
        Some(f) if f.glyph_id(c).0 != 0 => Glyph::Fallback(f.glyph_id(c)),
        _ => Glyph::Primary(c),
    }
}

/// ab_glyph scales by ascent-descent height; fontdue by em size.
fn fallback_scale(font: &ab_glyph::FontVec, size: f32) -> ab_glyph::PxScale {
    let upem = font.units_per_em().unwrap_or(1000.0);
    ab_glyph::PxScale::from(size * font.height_unscaled() / upem)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAlign {
    Left,
    Center,
    Right,
}

pub struct Canvas {
    pub width: u32,
    pub height: u32,
    pixmap: Pixmap,
}

impl Canvas {
    pub fn new(width: u32, height: u32) -> Self {
        let pixmap = Pixmap::new(width, height).expect("Failed to allocate pixmap");
        Self {
            width,
            height,
            pixmap,
        }
    }

    pub fn clear(&mut self, color: Color) {
        self.pixmap.fill(color.to_skia());
    }

    pub fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: Color) {
        if let Some(rect) = Rect::from_xywh(x, y, w, h) {
            let mut paint = Paint::default();
            paint.set_color(color.to_skia());
            paint.anti_alias = true;
            self.pixmap.fill_rect(rect, &paint, Transform::identity(), None);
        }
    }

    pub fn fill_rounded_rect(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32, color: Color) {
        let r = radius.min(w / 2.0).min(h / 2.0);
        let mut pb = PathBuilder::new();
        // Rounded rectangle path
        pb.move_to(x + r, y);
        pb.line_to(x + w - r, y);
        pb.quad_to(x + w, y, x + w, y + r);
        pb.line_to(x + w, y + h - r);
        pb.quad_to(x + w, y + h, x + w - r, y + h);
        pb.line_to(x + r, y + h);
        pb.quad_to(x, y + h, x, y + h - r);
        pb.line_to(x, y + r);
        pb.quad_to(x, y, x + r, y);
        pb.close();

        if let Some(path) = pb.finish() {
            let mut paint = Paint::default();
            paint.set_color(color.to_skia());
            paint.anti_alias = true;
            self.pixmap
                .fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
        }
    }

    pub fn stroke_rounded_rect(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        radius: f32,
        stroke_width: f32,
        color: Color,
    ) {
        let r = radius.min(w / 2.0).min(h / 2.0);
        let mut pb = PathBuilder::new();
        pb.move_to(x + r, y);
        pb.line_to(x + w - r, y);
        pb.quad_to(x + w, y, x + w, y + r);
        pb.line_to(x + w, y + h - r);
        pb.quad_to(x + w, y + h, x + w - r, y + h);
        pb.line_to(x + r, y + h);
        pb.quad_to(x, y + h, x, y + h - r);
        pb.line_to(x, y + r);
        pb.quad_to(x, y, x + r, y);
        pb.close();

        if let Some(path) = pb.finish() {
            let mut paint = Paint::default();
            paint.set_color(color.to_skia());
            paint.anti_alias = true;
            let stroke = Stroke {
                width: stroke_width,
                ..Default::default()
            };
            self.pixmap
                .stroke_path(&path, &paint, &stroke, Transform::identity(), None);
        }
    }

    pub fn draw_line(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, stroke_width: f32, color: Color) {
        let mut pb = PathBuilder::new();
        pb.move_to(x1, y1);
        pb.line_to(x2, y2);
        if let Some(path) = pb.finish() {
            let mut paint = Paint::default();
            paint.set_color(color.to_skia());
            paint.anti_alias = true;
            let stroke = Stroke {
                width: stroke_width,
                line_cap: LineCap::Round,
                ..Default::default()
            };
            self.pixmap
                .stroke_path(&path, &paint, &stroke, Transform::identity(), None);
        }
    }

    pub fn draw_icon(
        &mut self,
        name: &str,
        x: f32,
        y: f32,
        size: f32,
        color: Color,
        stroke_width: f32,
    ) {
        let mut pm_mut = self.pixmap.as_mut();
        draw_icon(&mut pm_mut, name, x, y, size, color, stroke_width);
    }

    pub fn measure_text(&self, text: &str, font_size: f32, bold: bool) -> (f32, f32) {
        let (regular, bold_font) = get_fonts();
        let font = if bold { bold_font } else { regular };
        let fb = fallback();
        let fb_font = fb.as_ref().map(|f| if bold { f.bold.as_ref().unwrap_or(&f.regular) } else { &f.regular });

        let mut total_width = 0.0;
        let mut prev: Option<char> = None;
        for c in text.chars() {
            match resolve(c, font, fb_font) {
                Glyph::Primary(c) => {
                    if let Some(p) = prev {
                        total_width += font.horizontal_kern(p, c, font_size).unwrap_or(0.0);
                    }
                    total_width += font.metrics(c, font_size).advance_width;
                    prev = Some(c);
                }
                Glyph::Fallback(id) => {
                    let f = fb_font.expect("fallback glyph implies fallback font");
                    total_width += f.as_scaled(fallback_scale(f, font_size)).h_advance(id);
                    prev = None;
                }
            }
        }
        (total_width, font_size)
    }

    /// Shorten `text` with an ellipsis so it fits in `max_width`.
    pub fn fit_text(&self, text: &str, font_size: f32, bold: bool, max_width: f32) -> String {
        if self.measure_text(text, font_size, bold).0 <= max_width {
            return text.to_string();
        }
        let chars: Vec<char> = text.chars().collect();
        let mut n = chars.len();
        while n > 0 {
            n -= 1;
            let candidate: String = chars[..n].iter().collect::<String>().trim_end().to_string() + "…";
            if self.measure_text(&candidate, font_size, bold).0 <= max_width {
                return candidate;
            }
        }
        String::new()
    }

    /// Like `draw_text`, but truncates with an ellipsis beyond `max_width`.
    pub fn draw_text_fit(
        &mut self,
        text: &str,
        x: f32,
        y: f32,
        font_size: f32,
        bold: bool,
        color: Color,
        align: TextAlign,
        max_width: f32,
    ) -> f32 {
        let fitted = self.fit_text(text, font_size, bold, max_width);
        self.draw_text(&fitted, x, y, font_size, bold, color, align);
        self.measure_text(&fitted, font_size, bold).0
    }

    pub fn fill_circle(&mut self, cx: f32, cy: f32, r: f32, color: Color) {
        let mut pb = PathBuilder::new();
        pb.push_circle(cx, cy, r);
        if let Some(path) = pb.finish() {
            let mut paint = Paint::default();
            paint.set_color(color.to_skia());
            paint.anti_alias = true;
            self.pixmap.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
        }
    }

    pub fn stroke_circle(&mut self, cx: f32, cy: f32, r: f32, stroke_width: f32, color: Color) {
        let mut pb = PathBuilder::new();
        pb.push_circle(cx, cy, r);
        if let Some(path) = pb.finish() {
            let mut paint = Paint::default();
            paint.set_color(color.to_skia());
            paint.anti_alias = true;
            let stroke = Stroke { width: stroke_width, ..Default::default() };
            self.pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
        }
    }

    pub fn draw_weather_art(&mut self, condition: &str, x: f32, y: f32, size: f32, ink: WeatherInk) {
        let mut pm_mut = self.pixmap.as_mut();
        draw_weather_art(&mut pm_mut, condition, x, y, size, ink);
    }

    pub fn draw_text(
        &mut self,
        text: &str,
        x: f32,
        y: f32,
        font_size: f32,
        bold: bool,
        color: Color,
        align: TextAlign,
    ) {
        let (regular, bold_font) = get_fonts();
        let font = if bold { bold_font } else { regular };

        let (text_width, _) = self.measure_text(text, font_size, bold);

        let start_x = match align {
            TextAlign::Left => x,
            TextAlign::Center => x - (text_width / 2.0),
            TextAlign::Right => x - text_width,
        };

        let mut current_x = start_x;
        let baseline_y = y + (font_size * 0.85); // Approximate typographic baseline
        let fb = fallback();
        let fb_font = fb.as_ref().map(|f| if bold { f.bold.as_ref().unwrap_or(&f.regular) } else { &f.regular });

        let mut prev: Option<char> = None;
        for c in text.chars() {
            match resolve(c, font, fb_font) {
                Glyph::Primary(c) => {
                    if let Some(p) = prev {
                        current_x += font.horizontal_kern(p, c, font_size).unwrap_or(0.0);
                    }
                    prev = Some(c);
                    let (metrics, bitmap) = font.rasterize(c, font_size);
                    let glyph_x = current_x + metrics.xmin as f32;
                    let glyph_y = baseline_y - metrics.height as f32 - metrics.ymin as f32;
                    self.blend_glyph(glyph_x.round() as i32, glyph_y.round() as i32, metrics.width, metrics.height, &bitmap, color);
                    current_x += metrics.advance_width;
                }
                Glyph::Fallback(id) => {
                    prev = None;
                    let f = fb_font.expect("fallback glyph implies fallback font");
                    let scale = fallback_scale(f, font_size);
                    let glyph = id.with_scale_and_position(scale, ab_glyph::point(current_x, baseline_y));
                    if let Some(outlined) = f.outline_glyph(glyph) {
                        let bb = outlined.px_bounds();
                        let (w, h) = ((bb.max.x - bb.min.x).ceil() as usize + 1, (bb.max.y - bb.min.y).ceil() as usize + 1);
                        let mut bitmap = vec![0u8; w * h];
                        outlined.draw(|gx, gy, cov| {
                            if let Some(px) = bitmap.get_mut(gy as usize * w + gx as usize) {
                                *px = (cov.clamp(0.0, 1.0) * 255.0) as u8;
                            }
                        });
                        self.blend_glyph(bb.min.x.floor() as i32, bb.min.y.floor() as i32, w, h, &bitmap, color);
                    }
                    current_x += f.as_scaled(scale).h_advance(id);
                }
            }
        }
    }

    fn blend_glyph(
        &mut self,
        start_x: i32,
        start_y: i32,
        width: usize,
        height: usize,
        bitmap: &[u8],
        color: Color,
    ) {
        let fb_w = self.width as i32;
        let fb_h = self.height as i32;
        let pixels = self.pixmap.data_mut();

        for gy in 0..height {
            let py = start_y + gy as i32;
            if py < 0 || py >= fb_h {
                continue;
            }

            for gx in 0..width {
                let px = start_x + gx as i32;
                if px < 0 || px >= fb_w {
                    continue;
                }

                let alpha = bitmap[gy * width + gx];
                if alpha == 0 {
                    continue;
                }

                let idx = ((py as usize) * (self.width as usize) + (px as usize)) * 4;
                let src_a = (alpha as u32 * color.a as u32) / 255;
                if src_a == 0 {
                    continue;
                }

                let dst_r = pixels[idx] as u32;
                let dst_g = pixels[idx + 1] as u32;
                let dst_b = pixels[idx + 2] as u32;
                let dst_a = pixels[idx + 3] as u32;

                let inv_src_a = 255 - src_a;

                // Alpha compositing
                let out_r = ((color.r as u32 * src_a) + (dst_r * inv_src_a)) / 255;
                let out_g = ((color.g as u32 * src_a) + (dst_g * inv_src_a)) / 255;
                let out_b = ((color.b as u32 * src_a) + (dst_b * inv_src_a)) / 255;
                let out_a = src_a + ((dst_a * inv_src_a) / 255);

                pixels[idx] = out_r as u8;
                pixels[idx + 1] = out_g as u8;
                pixels[idx + 2] = out_b as u8;
                pixels[idx + 3] = out_a as u8;
            }
        }
    }

    pub fn draw_progress_bar(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        radius: f32,
        progress: f32,
        bg_color: Color,
        fill_color: Color,
    ) {
        self.fill_rounded_rect(x, y, w, h, radius, bg_color);
        let progress = progress.clamp(0.0, 1.0);
        if progress > 0.01 {
            let fill_w = w * progress;
            self.fill_rounded_rect(x, y, fill_w, h, radius, fill_color);
        }
    }

    /// Line chart with an optional soft area fill underneath.
    pub fn draw_sparkline(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        values: &[f32],
        color: Color,
        stroke_width: f32,
        area: Option<Color>,
    ) {
        if values.len() < 2 {
            return;
        }

        let min_val = values.iter().copied().fold(f32::INFINITY, f32::min);
        let max_val = values.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let range = (max_val - min_val).max(f32::EPSILON);
        let pad = stroke_width;
        let step_x = w / ((values.len() - 1) as f32);
        let point = |i: usize, v: f32| (x + i as f32 * step_x, y + pad + (1.0 - (v - min_val) / range) * (h - 2.0 * pad));

        let mut line = PathBuilder::new();
        for (i, &v) in values.iter().enumerate() {
            let (px, py) = point(i, v);
            if i == 0 {
                line.move_to(px, py);
            } else {
                line.line_to(px, py);
            }
        }

        let mut paint = Paint::default();
        paint.anti_alias = true;

        if let Some(area_color) = area {
            let mut pb = PathBuilder::new();
            pb.move_to(x, y + h);
            for (i, &v) in values.iter().enumerate() {
                let (px, py) = point(i, v);
                pb.line_to(px, py);
            }
            pb.line_to(x + w, y + h);
            pb.close();
            if let Some(path) = pb.finish() {
                paint.set_color(area_color.to_skia());
                self.pixmap.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
            }
        }

        if let Some(path) = line.finish() {
            paint.set_color(color.to_skia());
            let stroke = Stroke {
                width: stroke_width,
                line_cap: LineCap::Round,
                line_join: LineJoin::Round,
                ..Default::default()
            };
            self.pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
        }
    }

    pub fn to_png_bytes(&self) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        let png = self.pixmap.encode_png()?;
        Ok(png)
    }

    pub fn to_grayscale_png_bytes(&self) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        let mut gray_pixmap = self.pixmap.clone();
        for pixel in gray_pixmap.pixels_mut() {
            let r = pixel.red();
            let g = pixel.green();
            let b = pixel.blue();
            let gray = ((0.299 * r as f32) + (0.587 * g as f32) + (0.114 * b as f32)).round() as u8;
            *pixel = tiny_skia::PremultipliedColorU8::from_rgba(gray, gray, gray, pixel.alpha()).unwrap();
        }
        let png = gray_pixmap.encode_png()?;
        Ok(png)
    }

    pub fn to_rgb565_bytes(&self) -> Vec<u8> {
        let pixel_count = (self.width * self.height) as usize;
        let mut bytes = Vec::with_capacity(pixel_count * 2);
        let data = self.pixmap.data();

        for i in 0..pixel_count {
            let idx = i * 4;
            let r = data[idx];
            let g = data[idx + 1];
            let b = data[idx + 2];

            let r5 = (r >> 3) as u16;
            let g6 = (g >> 2) as u16;
            let b5 = (b >> 3) as u16;
            let rgb565 = (r5 << 11) | (g6 << 5) | b5;

            bytes.extend_from_slice(&rgb565.to_le_bytes());
        }
        bytes
    }

    pub fn to_bgra8888_bytes(&self) -> Vec<u8> {
        let pixel_count = (self.width * self.height) as usize;
        let mut bytes = vec![0u8; pixel_count * 4];
        let data = self.pixmap.data();

        for i in 0..pixel_count {
            let src = i * 4;
            let dst = i * 4;
            bytes[dst] = data[src + 2];     // B
            bytes[dst + 1] = data[src + 1]; // G
            bytes[dst + 2] = data[src];     // R
            bytes[dst + 3] = data[src + 3]; // A
        }
        bytes
    }

    pub fn raw_rgba_data(&self) -> &[u8] {
        self.pixmap.data()
    }
}
