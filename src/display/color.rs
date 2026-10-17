use tiny_skia::Color as SkColor;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    pub fn from_hex(hex: &str) -> Self {
        let hex = hex.trim().trim_start_matches('#');
        if hex.len() == 6 {
            if let Ok(val) = u32::from_str_radix(hex, 16) {
                let r = ((val >> 16) & 0xFF) as u8;
                let g = ((val >> 8) & 0xFF) as u8;
                let b = (val & 0xFF) as u8;
                return Self::rgb(r, g, b);
            }
        } else if hex.len() == 8 {
            if let Ok(val) = u32::from_str_radix(hex, 16) {
                let r = ((val >> 24) & 0xFF) as u8;
                let g = ((val >> 16) & 0xFF) as u8;
                let b = ((val >> 8) & 0xFF) as u8;
                let a = (val & 0xFF) as u8;
                return Self::rgba(r, g, b, a);
            }
        }
        Self::rgb(0, 0, 0)
    }

    pub fn to_skia(&self) -> SkColor {
        SkColor::from_rgba8(self.r, self.g, self.b, self.a)
    }

    /// Convert to RGB565 for 16-bit Kobo framebuffers
    pub fn to_rgb565(&self) -> u16 {
        let r5 = (self.r >> 3) as u16;
        let g6 = (self.g >> 2) as u16;
        let b5 = (self.b >> 3) as u16;
        (r5 << 11) | (g6 << 5) | b5
    }

    /// Convert to 32-bit ARGB (standard Linux FB)
    pub fn to_argb32(&self) -> u32 {
        ((self.a as u32) << 24) | ((self.r as u32) << 16) | ((self.g as u32) << 8) | (self.b as u32)
    }

    /// Convert to 32-bit BGRA (MediaTek Kobo FB)
    pub fn to_bgra32(&self) -> u32 {
        ((self.b as u32) << 24) | ((self.g as u32) << 16) | ((self.r as u32) << 8) | (self.a as u32)
    }

    /// Convert to grayscale intensity (0.0 to 1.0)
    pub fn to_grayscale_u8(&self) -> u8 {
        // Luminance formula (Rec. 601)
        ((0.299 * self.r as f32) + (0.587 * self.g as f32) + (0.114 * self.b as f32)).round() as u8
    }

    /// Quantize to Kaleido 3 12-bit RGB444 color space (4096 colors)
    pub fn to_kaleido_quantized(&self) -> Self {
        // 16 levels per channel: scale 0..15 back to 0..255
        let r4 = (self.r >> 4) & 0x0F;
        let g4 = (self.g >> 4) & 0x0F;
        let b4 = (self.b >> 4) & 0x0F;
        Self {
            r: (r4 * 17),
            g: (g4 * 17),
            b: (b4 * 17),
            a: self.a,
        }
    }

    /// Linear blend towards `other` (t = 0 keeps self, t = 1 gives other)
    pub fn mix(&self, other: Color, t: f32) -> Self {
        let l = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round().clamp(0.0, 255.0) as u8;
        Self::rgba(l(self.r, other.r), l(self.g, other.g), l(self.b, other.b), l(self.a, other.a))
    }

    pub fn with_alpha(&self, alpha: u8) -> Self {
        Self {
            r: self.r,
            g: self.g,
            b: self.b,
            a: alpha,
        }
    }
}

// Curated Kaleido 3 Color Palette - designed for maximum readability, high contrast, and warm elegance
pub mod palette {
    use super::Color;

    // Backgrounds & Surfaces
    pub const WHITE: Color = Color::rgb(255, 255, 255);
    pub const PAPER_BG: Color = Color::rgb(248, 249, 250); // #F8F9FA
    pub const CARD_SURFACE: Color = Color::rgb(255, 255, 255);
    pub const CARD_BORDER: Color = Color::rgb(226, 232, 240); // #E2E8F0
    pub const CARD_BORDER_ACTIVE: Color = Color::rgb(203, 213, 225); // #CBD5E1

    // Text & Ink (300 ppi High-Contrast)
    pub const INK_BLACK: Color = Color::rgb(15, 23, 42); // #0F172A - Deep crisp ink
    pub const INK_MUTED: Color = Color::rgb(100, 116, 139); // #64748B - Slate gray
    pub const INK_FAINT: Color = Color::rgb(148, 163, 184); // #94A3B8

    // Active & Accent Colors (Kaleido 3 Tuned)
    pub const AMBER_GLOW: Color = Color::rgb(245, 158, 11); // #F59E0B - Light ON
    pub const AMBER_BG: Color = Color::rgb(254, 243, 199); // #FEF3C7 - Light card background
    pub const AMBER_BORDER: Color = Color::rgb(251, 191, 36);

    pub const BLUE_COOL: Color = Color::rgb(2, 132, 199); // #0284C7 - Climate cooling / Water
    pub const BLUE_BG: Color = Color::rgb(224, 242, 254); // #E0F2FE

    pub const RED_HEAT: Color = Color::rgb(225, 29, 72); // #E11D48 - Climate heating / Alert
    pub const RED_BG: Color = Color::rgb(255, 228, 230); // #FFE4E6

    pub const GREEN_ECO: Color = Color::rgb(22, 163, 74); // #16A34A - Eco / Safe / Battery Good
    pub const GREEN_BG: Color = Color::rgb(220, 252, 231); // #DCFCE7

    pub const PURPLE_MEDIA: Color = Color::rgb(139, 92, 246); // #8B5CF6 - Media Player
    pub const PURPLE_BG: Color = Color::rgb(243, 232, 255); // #F3E8FF

    pub const YELLOW_SUN: Color = Color::rgb(234, 179, 8); // #EAB308
    pub const GRAY_INACTIVE: Color = Color::rgb(241, 245, 249); // #F1F5F9
}
