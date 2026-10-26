use crate::config::{PageConfig, ThemeConfig};
use crate::display::{Canvas, TextAlign};
use crate::i18n;
use crate::ui::layout::{InteractiveTarget, Rect, UiAction};
use crate::ui::theme::Theme;
use chrono::{DateTime, Datelike, FixedOffset, Local, Timelike, Utc};

pub const SIDE_PAD: f32 = 32.0;

#[derive(Debug, Clone, Default)]
pub struct HeaderStatus {
    pub is_live: bool,
    pub is_demo: bool,
    pub last_update: Option<DateTime<Utc>>,
    pub battery: Option<u8>,
    pub charging: bool,
    pub wifi_up: Option<bool>,
    /// Frontlight state, when the device has one we can control
    pub frontlight: Option<bool>,
}

/// Current time in the configured timezone.
pub fn now_local(theme_cfg: &ThemeConfig) -> DateTime<FixedOffset> {
    to_local(Utc::now(), theme_cfg)
}

pub fn to_local(t: DateTime<Utc>, theme_cfg: &ThemeConfig) -> DateTime<FixedOffset> {
    let offset = theme_cfg
        .utc_offset
        .as_deref()
        .and_then(parse_offset)
        .unwrap_or_else(|| *Local::now().offset());
    t.with_timezone(&offset)
}

fn parse_offset(s: &str) -> Option<FixedOffset> {
    let s = s.trim();
    let (sign, rest) = match s.chars().next()? {
        '+' => (1, &s[1..]),
        '-' => (-1, &s[1..]),
        _ => (1, s),
    };
    let mut parts = rest.split(':');
    let h: i32 = parts.next()?.parse().ok()?;
    let m: i32 = parts.next().unwrap_or("0").parse().ok()?;
    FixedOffset::east_opt(sign * (h * 3600 + m * 60))
}

pub fn format_time(t: &DateTime<FixedOffset>, theme_cfg: &ThemeConfig) -> String {
    if theme_cfg.time_format == "12h" {
        let (pm, h) = t.hour12();
        format!("{}:{:02} {}", h, t.minute(), if pm { "PM" } else { "AM" })
    } else {
        format!("{:02}:{:02}", t.hour(), t.minute())
    }
}

/// Battery and Wi-Fi state from sysfs (Kobo); None elsewhere.
pub fn read_device_status() -> (Option<u8>, bool, Option<bool>) {
    let mut battery = None;
    let mut charging = false;
    if let Ok(entries) = std::fs::read_dir("/sys/class/power_supply") {
        for e in entries.flatten() {
            let p = e.path();
            let is_battery = std::fs::read_to_string(p.join("type")).map(|t| t.trim() == "Battery").unwrap_or(false);
            if !is_battery {
                continue;
            }
            if let Some(cap) = std::fs::read_to_string(p.join("capacity")).ok().and_then(|c| c.trim().parse::<u8>().ok()) {
                battery = Some(cap.min(100));
                // On the cable: a full battery reads "Full" or "Not charging", not "Charging".
                charging = crate::power::is_plugged();
                break;
            }
        }
    }
    let wifi = std::fs::read_to_string(crate::power::wifi_operstate_path()).ok().map(|s| s.trim() == "up");
    (battery, charging, wifi)
}

fn draw_battery(canvas: &mut Canvas, theme: &Theme, x: f32, y: f32, pct: u8, charging: bool) {
    let (w, h) = (40.0, 22.0);
    canvas.stroke_rounded_rect(x, y, w, h, 5.0, 2.5, theme.text);
    canvas.fill_rounded_rect(x + w + 1.5, y + 7.0, 4.0, 8.0, 1.5, theme.text);
    let level = pct as f32 / 100.0;
    let color = if pct <= 15 { theme.heat.solid } else { theme.text };
    if level > 0.02 {
        canvas.fill_rounded_rect(x + 4.0, y + 4.0, (w - 8.0) * level, h - 8.0, 2.5, color);
    }
    if charging {
        // A bolt with a light outline reads on both the filled and the empty part.
        let (cx, cy) = (x + w / 2.0, y + h / 2.0);
        canvas.draw_icon("zap", cx - 14.0, cy - 14.0, 28.0, theme.surface, 1.0);
        canvas.stroke_rounded_rect(x, y, w, h, 5.0, 2.5, theme.text);
        canvas.draw_icon("zap", cx - 10.0, cy - 10.0, 20.0, theme.text, 1.0);
    }
}

fn draw_header_button(canvas: &mut Canvas, theme: &Theme, rect: Rect, icon: &str) {
    canvas.fill_rounded_rect(rect.x, rect.y, rect.w, rect.h, 20.0, theme.surface);
    canvas.stroke_rounded_rect(rect.x + 1.0, rect.y + 1.0, rect.w - 2.0, rect.h - 2.0, 19.0, 2.0, theme.border_strong);
    let s = 34.0;
    canvas.draw_icon(icon, rect.x + (rect.w - s) / 2.0, rect.y + (rect.h - s) / 2.0, s, theme.text, 2.2);
}

pub fn render_header(
    canvas: &mut Canvas,
    theme: &Theme,
    theme_cfg: &ThemeConfig,
    pages: &[PageConfig],
    active_page: usize,
    status: &HeaderStatus,
    targets: &mut Vec<InteractiveTarget>,
) -> f32 {
    let now = now_local(theme_cfg);
    let time_str = format_time(&now, theme_cfg);
    let weekday = i18n::nth("date.weekdays", now.weekday().num_days_from_monday() as usize);
    let date = i18n::tf(
        "date.long",
        &[("day", &now.day().to_string()), ("month", &i18n::nth("date.months", now.month0() as usize))],
    );

    let w = canvas.width as f32;
    let top = 26.0;
    let time_size = 76.0;

    // Clock + date
    canvas.draw_text(&time_str, SIDE_PAD, top - 4.0, time_size, true, theme.text, TextAlign::Left);
    let time_w = canvas.measure_text(&time_str, time_size, true).0;
    let dx = SIDE_PAD + time_w + 24.0;
    canvas.draw_text(&weekday, dx, top + 6.0, 28.0, true, theme.text, TextAlign::Left);
    canvas.draw_text(&date, dx, top + 42.0, 24.0, false, theme.text_muted, TextAlign::Left);

    // Exit + refresh buttons (big touch targets)
    let btn = 76.0;
    let exit_rect = Rect::new(w - SIDE_PAD - btn, top, btn, btn);
    let refresh_rect = Rect::new(exit_rect.x - 14.0 - btn, top, btn, btn);
    draw_header_button(canvas, theme, exit_rect, "logout");
    draw_header_button(canvas, theme, refresh_rect, "refresh");
    targets.push(InteractiveTarget { rect: exit_rect.inset(-8.0), action: UiAction::ExitToReader });
    targets.push(InteractiveTarget { rect: refresh_rect.inset(-8.0), action: UiAction::Refresh });

    let mut leftmost = refresh_rect.x;
    if let Some(on) = status.frontlight {
        let light_rect = Rect::new(refresh_rect.x - 14.0 - btn, top, btn, btn);
        draw_header_button(canvas, theme, light_rect, if on { "lightbulb_on" } else { "lightbulb" });
        targets.push(InteractiveTarget { rect: light_rect.inset(-8.0), action: UiAction::ToggleFrontlight });
        leftmost = light_rect.x;
    }

    // Status block, right aligned before the buttons
    let right = leftmost - 24.0;
    let mut sx = right;
    if let Some(up) = status.wifi_up {
        sx -= 30.0;
        canvas.draw_icon(if up { "wifi" } else { "wifi_off" }, sx, top + 4.0, 30.0, if up { theme.text } else { theme.heat.solid }, 2.2);
        sx -= 18.0;
    }
    if let Some(pct) = status.battery {
        let label = i18n::tf("meta.percent", &[("value", &pct.to_string())]);
        let lw = canvas.measure_text(&label, 24.0, true).0;
        canvas.draw_text(&label, sx, top + 7.0, 24.0, true, theme.text, TextAlign::Right);
        sx -= lw + 10.0 + 46.0;
        draw_battery(canvas, theme, sx, top + 8.0, pct, status.charging);
    }

    let (dot, text) = if status.is_demo {
        (theme.warm.solid, i18n::t("header.demo"))
    } else if status.is_live {
        let t = status
            .last_update
            .map(|t| format_time(&to_local(t, theme_cfg), theme_cfg))
            .unwrap_or_default();
        (theme.eco.solid, i18n::tf("header.live", &[("time", &t)]))
    } else {
        (theme.heat.solid, i18n::t("header.offline"))
    };
    let tw = canvas.measure_text(&text, 22.0, false).0;
    canvas.draw_text(&text, right, top + 46.0, 22.0, false, theme.text_muted, TextAlign::Right);
    canvas.fill_circle(right - tw - 14.0, top + 46.0 + 11.0, 7.0, dot);

    let mut y = top + btn + 20.0;

    // Page tabs as a segmented control
    if pages.len() > 1 {
        let tab_h = 64.0;
        let total_w = w - 2.0 * SIDE_PAD;
        canvas.fill_rounded_rect(SIDE_PAD, y, total_w, tab_h, tab_h / 2.0, theme.surface);
        canvas.stroke_rounded_rect(SIDE_PAD + 1.0, y + 1.0, total_w - 2.0, tab_h - 2.0, tab_h / 2.0 - 1.0, 2.0, theme.border);

        let seg_w = total_w / pages.len() as f32;
        for (idx, page) in pages.iter().enumerate() {
            let seg = Rect::new(SIDE_PAD + idx as f32 * seg_w, y, seg_w, tab_h);
            let active = idx == active_page;
            let inner = seg.inset(6.0);
            if active {
                canvas.fill_rounded_rect(inner.x, inner.y, inner.w, inner.h, inner.h / 2.0, theme.text);
            }
            let color = if active { theme.surface } else { theme.text_muted };
            let icon_size = 28.0;
            let label = canvas.fit_text(&page.title, 24.0, true, seg_w - 40.0 - icon_size - 10.0);
            let lw = canvas.measure_text(&label, 24.0, true).0;
            let start = seg.x + (seg.w - (icon_size + 10.0 + lw)) / 2.0;
            canvas.draw_icon(&page.icon, start, y + (tab_h - icon_size) / 2.0, icon_size, color, 2.2);
            canvas.draw_text(&label, start + icon_size + 10.0, y + tab_h / 2.0 - 13.0, 24.0, true, color, TextAlign::Left);
            targets.push(InteractiveTarget { rect: seg, action: UiAction::SwitchPage(idx) });
        }
        y += tab_h + 20.0;
    }

    y
}
