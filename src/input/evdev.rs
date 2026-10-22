use crate::display::framebuffer::Rotation;
use tokio::sync::mpsc::{self, Receiver, Sender};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InputEvent {
    /// A single tap, in canvas coordinates
    Tap { x: f32, y: f32 },
    /// The hardware power button was pressed
    PowerButton,
    /// A USB cable to a computer was connected (not just a charger)
    Plugged,
    /// A charger or cable was connected or removed (the battery icon changes)
    PowerSourceChanged,
}

/// Throw away power-key presses queued for Nickel (see `linux::discard_nickel_power_presses`).
pub fn discard_nickel_power_presses() {
    #[cfg(target_os = "linux")]
    linux::discard_nickel_power_presses();
}

/// How raw touch-controller coordinates map to the app canvas.
#[derive(Debug, Clone)]
pub struct TouchMapping {
    /// Size of the panel in its physical portrait layout
    pub portrait_w: u32,
    pub portrait_h: u32,
    /// Raw axes are swapped relative to portrait (true on almost all Kobos)
    pub swap_xy: bool,
    /// Mirror X after swapping (true on almost all Kobos)
    pub invert_x: bool,
    pub invert_y: bool,
    /// Rotation from the app canvas to the portrait layout (landscape mode)
    pub canvas_rotation: Rotation,
}

impl TouchMapping {
    /// `nx`, `ny` are raw coordinates normalised to 0..=1 by the axis range.
    pub fn to_canvas(&self, nx: f32, ny: f32) -> (f32, f32) {
        let (mut x, mut y) = if self.swap_xy { (ny, nx) } else { (nx, ny) };
        if self.invert_x {
            x = 1.0 - x;
        }
        if self.invert_y {
            y = 1.0 - y;
        }
        let pw = self.portrait_w as f32;
        let ph = self.portrait_h as f32;
        let px = (x * pw).clamp(0.0, pw - 1.0);
        let py = (y * ph).clamp(0.0, ph - 1.0);
        self.canvas_rotation.inverse().map(px, py, pw, ph)
    }
}

pub struct TouchDriver {
    rx: Receiver<InputEvent>,
}

impl TouchDriver {
    #[allow(unused_variables)]
    pub fn new(device_path: &str, mapping: TouchMapping) -> (Self, Sender<InputEvent>) {
        let (tx, rx) = mpsc::channel(32);

        #[cfg(target_os = "linux")]
        linux::spawn_listeners(device_path, mapping, tx.clone());

        (Self { rx }, tx)
    }

    pub async fn recv(&mut self) -> Option<InputEvent> {
        self.rx.recv().await
    }

    pub fn try_recv(&mut self) -> Option<InputEvent> {
        self.rx.try_recv().ok()
    }

    /// Throw away queued events (e.g. the power-button press that woke the device).
    pub fn drain(&mut self) {
        while self.rx.try_recv().is_ok() {}
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use std::fs::File;
    use std::io::Read;
    use std::os::unix::io::AsRawFd;
    use std::time::Duration;

    const EV_SYN: u16 = 0x00;
    const EV_KEY: u16 = 0x01;
    const EV_ABS: u16 = 0x03;
    const SYN_REPORT: u16 = 0x00;
    const ABS_X: u16 = 0x00;
    const ABS_Y: u16 = 0x01;
    const ABS_PRESSURE: u16 = 0x18;
    const ABS_MT_TOUCH_MAJOR: u16 = 0x30;
    const ABS_MT_POSITION_X: u16 = 0x35;
    const ABS_MT_POSITION_Y: u16 = 0x36;
    const ABS_MT_TRACKING_ID: u16 = 0x39;
    const ABS_MT_PRESSURE: u16 = 0x3a;
    const BTN_TOUCH: u16 = 0x14a;
    const KEY_POWER: u16 = 116;

    /// A gap this long between reports means the finger was lifted, even if
    /// the controller never told us (some Kobo "snow" controllers don't).
    const NEW_TOUCH_GAP_US: i64 = 250_000;

    // struct input_event on the running kernel: timeval is two `long`s.
    #[cfg(target_pointer_width = "32")]
    const EVENT_SIZE: usize = 16;
    #[cfg(target_pointer_width = "64")]
    const EVENT_SIZE: usize = 24;

    struct RawEvent {
        time_us: i64,
        type_: u16,
        code: u16,
        value: i32,
    }

    fn parse_event(buf: &[u8; EVENT_SIZE]) -> RawEvent {
        let word = EVENT_SIZE / 2 - 4; // size of one `long`
        let read_long = |off: usize| -> i64 {
            if word == 4 {
                i32::from_ne_bytes(buf[off..off + 4].try_into().unwrap()) as i64
            } else {
                i64::from_ne_bytes(buf[off..off + 8].try_into().unwrap())
            }
        };
        let sec = read_long(0);
        let usec = read_long(word);
        let o = word * 2;
        RawEvent {
            time_us: sec * 1_000_000 + usec,
            type_: u16::from_ne_bytes([buf[o], buf[o + 1]]),
            code: u16::from_ne_bytes([buf[o + 2], buf[o + 3]]),
            value: i32::from_ne_bytes(buf[o + 4..o + 8].try_into().unwrap()),
        }
    }

    fn ioc(dir: u32, nr: u32, size: u32) -> libc::c_ulong {
        ((dir << 30) | (size << 16) | ((b'E' as u32) << 8) | nr) as libc::c_ulong
    }

    fn has_bit(fd: i32, ev_type: u32, bit: u16) -> bool {
        let mut bits = [0u8; 96];
        let req = ioc(2, 0x20 + ev_type, bits.len() as u32);
        let ret = unsafe { libc::ioctl(fd, req as _, bits.as_mut_ptr()) };
        ret >= 0 && (bits[(bit / 8) as usize] >> (bit % 8)) & 1 == 1
    }

    fn abs_range(fd: i32, axis: u16) -> Option<(i32, i32)> {
        let mut info = [0i32; 6]; // value, minimum, maximum, fuzz, flat, resolution
        let req = ioc(2, 0x40 + axis as u32, 24);
        let ret = unsafe { libc::ioctl(fd, req as _, info.as_mut_ptr()) };
        (ret >= 0 && info[2] > info[1]).then_some((info[1], info[2]))
    }

    fn device_name(fd: i32) -> String {
        let mut name = [0u8; 128];
        let ret = unsafe { libc::ioctl(fd, ioc(2, 0x06, name.len() as u32) as _, name.as_mut_ptr()) };
        if ret <= 0 {
            return String::new();
        }
        String::from_utf8_lossy(&name[..name.iter().position(|b| *b == 0).unwrap_or(name.len())]).into_owned()
    }

    fn grab(file: &File, path: &str) {
        let req = ioc(1, 0x90, std::mem::size_of::<libc::c_int>() as u32);
        let ret = unsafe { libc::ioctl(file.as_raw_fd(), req as _, 1 as libc::c_int) };
        if ret < 0 {
            tracing::warn!("EVIOCGRAB on {} failed ({}); Nickel may receive input events too", path, std::io::Error::last_os_error());
        }
    }

    fn event_devices() -> Vec<String> {
        let mut paths: Vec<String> = std::fs::read_dir("/dev/input")
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .map(|e| e.path().to_string_lossy().into_owned())
                    .filter(|p| p.contains("/event"))
                    .collect()
            })
            .unwrap_or_default();
        paths.sort();
        paths
    }

    pub fn spawn_listeners(device_path: &str, mapping: TouchMapping, tx: Sender<InputEvent>) {
        let devices = event_devices();
        let mut touch_path = None;
        let mut power_paths = Vec::new();

        for path in &devices {
            let Ok(file) = File::open(path) else { continue };
            let fd = file.as_raw_fd();
            let is_touch = has_bit(fd, EV_ABS as u32, ABS_MT_POSITION_X) || has_bit(fd, EV_ABS as u32, ABS_X);
            let has_power = has_bit(fd, EV_KEY as u32, KEY_POWER);
            tracing::info!("Input device {} ({:?}): touch={} power_key={}", path, device_name(fd), is_touch, has_power);
            if is_touch && touch_path.is_none() {
                touch_path = Some(path.clone());
            }
            if has_power && !is_touch {
                power_paths.push(path.clone());
            }
        }

        let touch_path = if device_path.is_empty() || device_path == "auto" {
            touch_path.unwrap_or_else(|| "/dev/input/event1".to_string())
        } else {
            device_path.to_string()
        };

        {
            let tx = tx.clone();
            let path = touch_path.clone();
            std::thread::spawn(move || run_touch_listener(&path, mapping, tx));
        }

        for path in power_paths.into_iter().filter(|p| *p != touch_path) {
            let tx = tx.clone();
            std::thread::spawn(move || run_power_listener(&path, tx));
        }

        std::thread::spawn(move || run_power_watch(tx));
    }

    /// Interrupt count of the power key, summed over CPUs and matching lines.
    /// Nickel keeps the power-key device grabbed (EVIOCGRAB fails with EBUSY),
    /// so its events never reach us; the interrupt counter still moves.
    fn power_key_interrupts() -> Option<u64> {
        let text = std::fs::read_to_string("/proc/interrupts").ok()?;
        let mut total = None;
        for line in text.lines().filter(|l| is_power_key_line(l)) {
            let count: u64 = line.split_whitespace().skip(1).map_while(|t| t.parse::<u64>().ok()).sum();
            total = Some(total.unwrap_or(0) + count);
        }
        total
    }

    fn is_power_key_line(line: &str) -> bool {
        let l = line.to_ascii_lowercase();
        ["pwrkey", "pwr-key", "pwr_key", "powerkey", "power-key", "power_key"].iter().any(|k| l.contains(k))
    }

    /// Once a second: power-key presses (by interrupt count) and the USB cable
    /// or charger being plugged in or out. A charger reads `online=1` and a
    /// computer `online=2` on the Clara Colour, a few seconds after the cable
    /// goes in.
    fn run_power_watch(tx: Sender<InputEvent>) {
        if let Ok(text) = std::fs::read_to_string("/proc/interrupts") {
            for line in text.lines().filter(|l| {
                let l = l.to_ascii_lowercase();
                l.contains("key") || l.contains("bd7") || l.contains("pwr") || l.contains("power")
            }) {
                tracing::info!("Interrupt: {}", line.trim());
            }
        }
        let mut last_irq = power_key_interrupts();
        if last_irq.is_none() {
            tracing::warn!("No power-key interrupt found in /proc/interrupts; the power button is only seen through its input device");
        }
        let mut was_plugged = crate::power::is_plugged();
        loop {
            std::thread::sleep(Duration::from_secs(1));
            if let Some(now) = power_key_interrupts() {
                if last_irq.is_some_and(|prev| now > prev) {
                    tracing::info!("Power key pressed (interrupts {:?} -> {})", last_irq, now);
                    if tx.blocking_send(InputEvent::PowerButton).is_err() {
                        return;
                    }
                }
                last_irq = Some(now);
            }
            let plugged = crate::power::is_plugged();
            if plugged != was_plugged && tx.blocking_send(InputEvent::PowerSourceChanged).is_err() {
                return;
            }
            if plugged && !was_plugged {
                tracing::info!("USB cable or charger connected");
                // A computer enumerates the USB controller within a moment.
                let computer = (0..5).any(|i| {
                    if i > 0 {
                        std::thread::sleep(Duration::from_secs(1));
                    }
                    crate::power::connected_to_computer()
                });
                if !computer {
                    tracing::info!("Only a charger: staying in the dashboard");
                } else if tx.blocking_send(InputEvent::Plugged).is_err() {
                    return;
                }
            }
            was_plugged = plugged;
        }
    }

    /// Nickel holds the power-key device grabbed, so every press while the
    /// dashboard runs (including the one that wakes it from sleep) waits in
    /// Nickel's queue, and Nickel goes to sleep as soon as it is resumed.
    /// Writing to the device injects events that only the grabbing client
    /// receives: enough harmless SYN_CONFIG events overflow its queue, and the
    /// kernel then drops everything that was waiting.
    pub fn discard_nickel_power_presses() {
        use std::io::Write;
        const SYN_CONFIG: u16 = 1;
        for path in event_devices() {
            let Ok(file) = File::open(&path) else { continue };
            let fd = file.as_raw_fd();
            let is_touch = has_bit(fd, EV_ABS as u32, ABS_MT_POSITION_X) || has_bit(fd, EV_ABS as u32, ABS_X);
            if is_touch || !has_bit(fd, EV_KEY as u32, KEY_POWER) {
                continue;
            }
            let Ok(mut dev) = std::fs::OpenOptions::new().write(true).open(&path) else { continue };
            let event = |code: u16| {
                let mut e = [0u8; EVENT_SIZE];
                let o = EVENT_SIZE - 8;
                e[o..o + 2].copy_from_slice(&EV_SYN.to_ne_bytes());
                e[o + 2..o + 4].copy_from_slice(&code.to_ne_bytes());
                e
            };
            let mut buf = Vec::with_capacity(EVENT_SIZE * 1025);
            for _ in 0..1024 {
                buf.extend_from_slice(&event(SYN_CONFIG));
            }
            buf.extend_from_slice(&event(SYN_REPORT));
            match dev.write_all(&buf) {
                Ok(()) => tracing::info!("Cleared Nickel's queued power-key presses on {}", path),
                Err(e) => tracing::warn!("Could not clear power-key presses on {}: {}", path, e),
            }
        }
    }

    fn open_with_retry(path: &str) -> Option<File> {
        for attempt in 0..5 {
            match File::open(path) {
                Ok(f) => return Some(f),
                Err(e) => {
                    tracing::warn!("Could not open input device {} (attempt {}): {}", path, attempt + 1, e);
                    std::thread::sleep(Duration::from_millis(500));
                }
            }
        }
        None
    }

    fn run_power_listener(path: &str, tx: Sender<InputEvent>) {
        let Some(mut file) = open_with_retry(path) else { return };
        // Grab it so Nickel doesn't process the press (and go to sleep) once resumed.
        grab(&file, path);
        let mut buf = [0u8; EVENT_SIZE];
        while file.read_exact(&mut buf).is_ok() {
            let ev = parse_event(&buf);
            if ev.type_ == EV_KEY && ev.code == KEY_POWER && ev.value == 1 {
                tracing::info!("Power key pressed ({})", path);
                if tx.blocking_send(InputEvent::PowerButton).is_err() {
                    return;
                }
            }
        }
    }

    fn run_touch_listener(path: &str, mapping: TouchMapping, tx: Sender<InputEvent>) {
        tracing::info!("Starting touch listener on {} ({:?})", path, mapping);
        let Some(mut file) = open_with_retry(path) else { return };
        grab(&file, path);

        let fd = file.as_raw_fd();
        let fallback_x = (0, if mapping.swap_xy { mapping.portrait_h } else { mapping.portrait_w } as i32 - 1);
        let fallback_y = (0, if mapping.swap_xy { mapping.portrait_w } else { mapping.portrait_h } as i32 - 1);
        let range_x = abs_range(fd, ABS_MT_POSITION_X).or_else(|| abs_range(fd, ABS_X)).unwrap_or(fallback_x);
        let range_y = abs_range(fd, ABS_MT_POSITION_Y).or_else(|| abs_range(fd, ABS_Y)).unwrap_or(fallback_y);
        tracing::info!("Touch axis ranges: x={:?} y={:?}", range_x, range_y);

        let norm = |v: i32, (lo, hi): (i32, i32)| ((v - lo) as f32 / (hi - lo) as f32).clamp(0.0, 1.0);

        let mut cur_x: Option<i32> = None;
        let mut cur_y: Option<i32> = None;
        let mut released = false;
        let mut moved = false;
        let mut emitted = false;
        let mut last_report_us: i64 = 0;
        let mut buf = [0u8; EVENT_SIZE];

        while file.read_exact(&mut buf).is_ok() {
            let ev = parse_event(&buf);
            match (ev.type_, ev.code) {
                (EV_ABS, ABS_MT_POSITION_X) | (EV_ABS, ABS_X) => {
                    cur_x = Some(ev.value);
                    moved = true;
                }
                (EV_ABS, ABS_MT_POSITION_Y) | (EV_ABS, ABS_Y) => {
                    cur_y = Some(ev.value);
                    moved = true;
                }
                (EV_ABS, ABS_MT_TRACKING_ID) if ev.value < 0 => released = true,
                (EV_ABS, ABS_MT_PRESSURE) | (EV_ABS, ABS_PRESSURE) | (EV_ABS, ABS_MT_TOUCH_MAJOR) if ev.value == 0 => {
                    released = true
                }
                (EV_KEY, BTN_TOUCH) if ev.value == 0 => released = true,
                // Some models report the power key on the touch device too.
                (EV_KEY, KEY_POWER) if ev.value == 1 => {
                    tracing::info!("Power key pressed ({})", path);
                    if tx.blocking_send(InputEvent::PowerButton).is_err() {
                        return;
                    }
                }
                (EV_SYN, SYN_REPORT) => {
                    if ev.time_us - last_report_us > NEW_TOUCH_GAP_US {
                        emitted = false;
                    }
                    last_report_us = ev.time_us;

                    // Fire once per touch, as soon as we know where the finger went down.
                    if !emitted && moved {
                        if let (Some(rx), Some(ry)) = (cur_x, cur_y) {
                            let (x, y) = mapping.to_canvas(norm(rx, range_x), norm(ry, range_y));
                            tracing::debug!("Tap raw=({}, {}) canvas=({:.0}, {:.0})", rx, ry, x, y);
                            emitted = true;
                            if tx.blocking_send(InputEvent::Tap { x, y }).is_err() {
                                return;
                            }
                        }
                    }
                    if released {
                        emitted = false;
                        released = false;
                    }
                    moved = false;
                }
                _ => {}
            }
        }
        tracing::warn!("Touch device {} closed", path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kobo() -> TouchMapping {
        TouchMapping {
            portrait_w: 1072,
            portrait_h: 1448,
            swap_xy: true,
            invert_x: true,
            invert_y: false,
            canvas_rotation: Rotation::R0,
        }
    }

    #[test]
    fn kobo_default_matches_koreader_convention() {
        // KOReader: screen_x = (W - 1) - raw_y, screen_y = raw_x
        let m = kobo();
        let (x, y) = m.to_canvas(0.0, 0.0);
        assert_eq!((x.round(), y.round()), (1071.0, 0.0));
        let (x, y) = m.to_canvas(1.0, 1.0);
        assert_eq!((x.round(), y.round()), (0.0, 1447.0));
        let (x, y) = m.to_canvas(0.5, 0.25);
        assert_eq!((x.round(), y.round()), (804.0, 724.0));
    }

    #[test]
    fn landscape_maps_into_landscape_canvas() {
        let mut m = kobo();
        m.canvas_rotation = Rotation::R90;
        for (nx, ny) in [(0.0, 0.0), (1.0, 1.0), (0.3, 0.7)] {
            let (x, y) = m.to_canvas(nx, ny);
            assert!((0.0..1448.0).contains(&x) && (0.0..1072.0).contains(&y), "({}, {})", x, y);
        }
    }
}
