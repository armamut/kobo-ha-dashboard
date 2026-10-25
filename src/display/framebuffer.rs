use crate::display::canvas::Canvas;
use std::error::Error;

/// How to update the e-ink panel after drawing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refresh {
    /// Update only the changed area, if anything changed.
    Partial,
    /// Flash the whole panel to clear ghosting, but only if something changed.
    FullIfChanged,
    /// Flash the whole panel regardless (page switch, sleep screen).
    Full,
}

pub trait FramebufferDriver: Send {
    /// Push the canvas to the screen. Returns whether any pixel changed.
    fn flush(&mut self, canvas: &Canvas, refresh: Refresh) -> Result<bool, Box<dyn Error + Send + Sync>>;
    fn dimensions(&self) -> (u32, u32);
    /// Put back whatever was on screen before we took over (Nickel's last frame).
    fn restore(&mut self) {}
}

/// Clockwise rotation applied when mapping one coordinate space onto another.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rotation {
    R0,
    R90,
    R180,
    R270,
}

impl Rotation {
    pub fn from_degrees(deg: i32) -> Self {
        match deg.rem_euclid(360) {
            90 => Rotation::R90,
            180 => Rotation::R180,
            270 => Rotation::R270,
            _ => Rotation::R0,
        }
    }

    pub fn degrees(self) -> i32 {
        match self {
            Rotation::R0 => 0,
            Rotation::R90 => 90,
            Rotation::R180 => 180,
            Rotation::R270 => 270,
        }
    }

    pub fn then(self, other: Rotation) -> Rotation {
        Rotation::from_degrees(self.degrees() + other.degrees())
    }

    pub fn inverse(self) -> Rotation {
        Rotation::from_degrees(360 - self.degrees())
    }

    pub fn swaps_axes(self) -> bool {
        matches!(self, Rotation::R90 | Rotation::R270)
    }

    /// Map (x, y) inside a `w`x`h` space to the rotated space.
    pub fn map(self, x: f32, y: f32, w: f32, h: f32) -> (f32, f32) {
        match self {
            Rotation::R0 => (x, y),
            Rotation::R90 => (h - 1.0 - y, x),
            Rotation::R180 => (w - 1.0 - x, h - 1.0 - y),
            Rotation::R270 => (y, w - 1.0 - x),
        }
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use std::os::unix::io::AsRawFd;

    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct FbBitfield {
        offset: u32,
        length: u32,
        msb_right: u32,
    }

    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct FbVarScreeninfo {
        xres: u32,
        yres: u32,
        xres_virtual: u32,
        yres_virtual: u32,
        xoffset: u32,
        yoffset: u32,
        bits_per_pixel: u32,
        grayscale: u32,
        red: FbBitfield,
        green: FbBitfield,
        blue: FbBitfield,
        transp: FbBitfield,
        nonstd: u32,
        activate: u32,
        height: u32,
        width: u32,
        accel_flags: u32,
        pixclock: u32,
        left_margin: u32,
        right_margin: u32,
        upper_margin: u32,
        lower_margin: u32,
        hsync_len: u32,
        vsync_len: u32,
        sync: u32,
        vmode: u32,
        rotate: u32,
        colorspace: u32,
        reserved: [u32; 4],
    }

    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct FbFixScreeninfo {
        id: [u8; 16],
        smem_start: libc::c_ulong,
        smem_len: u32,
        type_: u32,
        type_aux: u32,
        visual: u32,
        xpanstep: u16,
        ypanstep: u16,
        ywrapstep: u16,
        line_length: u32,
        mmio_start: libc::c_ulong,
        mmio_len: u32,
        accel: u32,
        capabilities: u16,
        reserved: [u16; 2],
    }

    const FBIOGET_VSCREENINFO: libc::c_ulong = 0x4600;
    const FBIOGET_FSCREENINFO: libc::c_ulong = 0x4602;

    // E-ink update ioctls. All share 'F' 0x2E; the struct size encoded in the
    // request number tells the drivers apart, so a wrong guess simply fails
    // with ENOTTY/EINVAL instead of doing anything harmful.
    const fn iow(nr: u32, size: u32) -> libc::c_ulong {
        (0x4000_0000 | (size << 16) | ((b'F' as u32) << 8) | nr) as libc::c_ulong
    }

    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct EinkRect {
        top: u32,
        left: u32,
        width: u32,
        height: u32,
    }

    /// MediaTek hwtcon (Clara BW / Clara Colour / Libra Colour / Elipsa 2E)
    #[repr(C)]
    #[derive(Default)]
    struct HwtconUpdateData {
        update_region: EinkRect,
        waveform_mode: u32,
        update_mode: u32,
        update_marker: u32,
        flags: u32,
        dither_mode: i32,
    }

    #[repr(C)]
    #[derive(Default)]
    struct MxcfbAltBufferData {
        phys_addr: u32,
        width: u32,
        height: u32,
        alt_update_region: EinkRect,
    }

    /// Netronix kernels on i.MX50/i.MX6SL (Touch, Glo, Mini, Aura...) carry
    /// an extra virtual address pointer in the alt buffer data.
    #[repr(C)]
    #[derive(Default)]
    struct MxcfbAltBufferDataNtx {
        virt_addr: usize,
        phys_addr: u32,
        width: u32,
        height: u32,
        alt_update_region: EinkRect,
    }

    /// NXP i.MX "Mark 7" (Clara HD, Libra, Forma, Sage...)
    #[repr(C)]
    #[derive(Default)]
    struct MxcfbUpdateDataV2 {
        update_region: EinkRect,
        waveform_mode: u32,
        update_mode: u32,
        update_marker: u32,
        temp: i32,
        flags: u32,
        dither_mode: i32,
        quant_bit: i32,
        alt_buffer_data: MxcfbAltBufferData,
    }

    /// Older NXP i.MX Kobos
    #[repr(C)]
    #[derive(Default)]
    struct MxcfbUpdateDataV1 {
        update_region: EinkRect,
        waveform_mode: u32,
        update_mode: u32,
        update_marker: u32,
        temp: i32,
        flags: u32,
        alt_buffer_data: MxcfbAltBufferData,
    }

    /// Oldest Kobos (Touch, Glo, Mini, Aura HD...)
    #[repr(C)]
    #[derive(Default)]
    struct MxcfbUpdateDataV1Ntx {
        update_region: EinkRect,
        waveform_mode: u32,
        update_mode: u32,
        update_marker: u32,
        temp: i32,
        flags: u32,
        alt_buffer_data: MxcfbAltBufferDataNtx,
    }

    const SEND_UPDATE_HWTCON: libc::c_ulong = iow(0x2E, std::mem::size_of::<HwtconUpdateData>() as u32);
    const SEND_UPDATE_MXCFB_V2: libc::c_ulong = iow(0x2E, std::mem::size_of::<MxcfbUpdateDataV2>() as u32);
    const SEND_UPDATE_MXCFB_V1: libc::c_ulong = iow(0x2E, std::mem::size_of::<MxcfbUpdateDataV1>() as u32);
    const SEND_UPDATE_MXCFB_V1_NTX: libc::c_ulong = iow(0x2E, std::mem::size_of::<MxcfbUpdateDataV1Ntx>() as u32);

    // Request numbers as the 32-bit Kobo kernels define them.
    #[cfg(target_pointer_width = "32")]
    const _: () = {
        assert!(SEND_UPDATE_MXCFB_V1 == 0x4040_462E);
        assert!(SEND_UPDATE_MXCFB_V1_NTX == 0x4044_462E);
    };

    const UPDATE_MODE_PARTIAL: u32 = 0;
    const UPDATE_MODE_FULL: u32 = 1;
    const WAVEFORM_MODE_GC16: u32 = 2;
    const WAVEFORM_MODE_GCC16: u32 = 10;
    const WAVEFORM_MODE_GLRC16: u32 = 11;
    const WAVEFORM_MODE_AUTO: u32 = 257;
    const TEMP_USE_AMBIENT: i32 = 0x1000;

    #[derive(Debug, Clone, Copy, PartialEq)]
    enum EinkBackend {
        Unknown,
        Hwtcon,
        MxcfbV2,
        MxcfbV1,
        MxcfbV1Ntx,
        Fbink,
        None,
    }

    #[derive(Debug, Clone, Copy)]
    enum PixelFormat {
        Gray8,
        Rgb565,
        Rgba32,
        Bgra32,
    }

    pub fn probe_resolution(path: &str) -> Option<(u32, u32)> {
        let file = std::fs::OpenOptions::new().read(true).open(path).ok()?;
        let mut vinfo = FbVarScreeninfo::default();
        let ok = unsafe { libc::ioctl(file.as_raw_fd(), FBIOGET_VSCREENINFO as _, &mut vinfo) } >= 0;
        (ok && vinfo.xres > 0 && vinfo.yres > 0).then_some((vinfo.xres, vinfo.yres))
    }

    pub struct LinuxFramebuffer {
        file: std::fs::File,
        xres: u32,
        yres: u32,
        xoffset: u32,
        yoffset: u32,
        line_length: usize,
        bytes_per_pixel: usize,
        format: PixelFormat,
        mmap_ptr: *mut u8,
        mmap_len: usize,
        rotation: Rotation,
        color_panel: bool,
        backend: EinkBackend,
        marker: u32,
        saved: Option<Vec<u8>>,
        scratch: Vec<u8>,
    }

    // The mmap pointer is only ever touched from the thread owning the driver.
    unsafe impl Send for LinuxFramebuffer {}

    impl LinuxFramebuffer {
        /// `canvas_rotation` is the rotation from the app canvas to the panel's
        /// physical portrait layout; `fb_rotation` (None = auto) from that
        /// portrait layout to the framebuffer memory layout.
        pub fn open(
            path: &str,
            canvas_w: u32,
            canvas_h: u32,
            canvas_rotation: Rotation,
            fb_rotation: Option<Rotation>,
            color_panel: bool,
        ) -> Result<Self, Box<dyn Error + Send + Sync>> {
            let file = std::fs::OpenOptions::new().read(true).write(true).open(path)?;
            let fd = file.as_raw_fd();

            let mut vinfo = FbVarScreeninfo::default();
            let mut finfo = FbFixScreeninfo::default();
            unsafe {
                if libc::ioctl(fd, FBIOGET_VSCREENINFO as _, &mut vinfo) < 0 {
                    return Err("FBIOGET_VSCREENINFO failed".into());
                }
                if libc::ioctl(fd, FBIOGET_FSCREENINFO as _, &mut finfo) < 0 {
                    return Err("FBIOGET_FSCREENINFO failed".into());
                }
            }

            let format = match vinfo.bits_per_pixel {
                8 => PixelFormat::Gray8,
                16 => PixelFormat::Rgb565,
                32 if vinfo.red.offset == 0 => PixelFormat::Rgba32,
                32 => PixelFormat::Bgra32,
                bpp => return Err(format!("Unsupported framebuffer depth: {} bpp", bpp).into()),
            };
            let bytes_per_pixel = (vinfo.bits_per_pixel / 8) as usize;
            let line_length = if finfo.line_length > 0 {
                finfo.line_length as usize
            } else {
                vinfo.xres_virtual as usize * bytes_per_pixel
            };

            // Work out how the portrait canvas maps onto framebuffer memory.
            let (pw, ph) = if canvas_rotation.swaps_axes() { (canvas_h, canvas_w) } else { (canvas_w, canvas_h) };
            let fb_rotation = fb_rotation.unwrap_or_else(|| {
                if (vinfo.xres, vinfo.yres) == (pw, ph) || vinfo.xres < vinfo.yres {
                    Rotation::R0
                } else {
                    // Framebuffer is in the panel's native landscape layout.
                    // On Kobo the touch axes follow that layout (x = long edge,
                    // y mirrored), which corresponds to a 270° turn.
                    Rotation::R270
                }
            });
            let rotation = canvas_rotation.then(fb_rotation);

            let needed = (vinfo.yoffset as usize + vinfo.yres as usize) * line_length;
            let mmap_len = (finfo.smem_len as usize).max(needed);
            let mmap_ptr = unsafe {
                libc::mmap(
                    std::ptr::null_mut(),
                    mmap_len,
                    libc::PROT_READ | libc::PROT_WRITE,
                    libc::MAP_SHARED,
                    fd,
                    0,
                )
            };
            if mmap_ptr == libc::MAP_FAILED {
                return Err("mmap of framebuffer failed".into());
            }

            let id = String::from_utf8_lossy(&finfo.id).trim_end_matches('\0').to_string();
            tracing::info!(
                "Framebuffer '{}': {}x{} (virtual {}x{}, offset {},{}), {} bpp {:?}, line_length={}, smem_len={}, rotate={}, canvas={}x{}, mapping rotation={}°",
                id,
                vinfo.xres,
                vinfo.yres,
                vinfo.xres_virtual,
                vinfo.yres_virtual,
                vinfo.xoffset,
                vinfo.yoffset,
                vinfo.bits_per_pixel,
                format,
                line_length,
                finfo.smem_len,
                vinfo.rotate,
                canvas_w,
                canvas_h,
                rotation.degrees()
            );

            let mut fb = Self {
                file,
                xres: vinfo.xres,
                yres: vinfo.yres,
                xoffset: vinfo.xoffset,
                yoffset: vinfo.yoffset,
                line_length,
                bytes_per_pixel,
                format,
                mmap_ptr: mmap_ptr as *mut u8,
                mmap_len,
                rotation,
                color_panel,
                backend: EinkBackend::Unknown,
                marker: 1,
                saved: None,
                scratch: Vec::new(),
            };
            fb.saved = Some(fb.visible_slice().to_vec());
            Ok(fb)
        }

        fn visible_range(&self) -> (usize, usize) {
            let start = self.yoffset as usize * self.line_length;
            let end = (start + self.yres as usize * self.line_length).min(self.mmap_len);
            (start, end)
        }

        fn visible_slice(&self) -> &[u8] {
            let (start, end) = self.visible_range();
            unsafe { &std::slice::from_raw_parts(self.mmap_ptr, self.mmap_len)[start..end] }
        }

        fn visible_slice_mut(&mut self) -> &mut [u8] {
            let (start, end) = self.visible_range();
            unsafe { &mut std::slice::from_raw_parts_mut(self.mmap_ptr, self.mmap_len)[start..end] }
        }

        /// Convert the canvas into framebuffer layout in `self.scratch`.
        fn compose(&mut self, canvas: &Canvas) {
            let src = canvas.raw_rgba_data();
            let cw = canvas.width as usize;
            let ch = canvas.height as usize;
            let fw = self.xres as usize;
            let fh = self.yres as usize;
            let bpp = self.bytes_per_pixel;
            let row_off = self.xoffset as usize * bpp;
            let inv = self.rotation.inverse();
            let (rw, rh) = if self.rotation.swaps_axes() { (ch, cw) } else { (cw, ch) };

            let mut out = std::mem::take(&mut self.scratch);
            out.clear();
            out.extend_from_slice(self.visible_slice());

            for fy in 0..fh.min(rh) {
                let row = fy * self.line_length + row_off;
                for fx in 0..fw.min(rw) {
                    let (sx, sy) = inv.map(fx as f32, fy as f32, rw as f32, rh as f32);
                    let (sx, sy) = (sx as usize, sy as usize);
                    if sx >= cw || sy >= ch {
                        continue;
                    }
                    let s = (sy * cw + sx) * 4;
                    let (r, g, b) = (src[s], src[s + 1], src[s + 2]);
                    let d = row + fx * bpp;
                    if d + bpp > out.len() {
                        continue;
                    }
                    match self.format {
                        PixelFormat::Gray8 => {
                            out[d] = ((r as u32 * 77 + g as u32 * 150 + b as u32 * 29) >> 8) as u8;
                        }
                        PixelFormat::Rgb565 => {
                            let v = ((r as u16 >> 3) << 11) | ((g as u16 >> 2) << 5) | (b as u16 >> 3);
                            out[d..d + 2].copy_from_slice(&v.to_le_bytes());
                        }
                        PixelFormat::Rgba32 => {
                            out[d] = r;
                            out[d + 1] = g;
                            out[d + 2] = b;
                            out[d + 3] = 0xFF;
                        }
                        PixelFormat::Bgra32 => {
                            out[d] = b;
                            out[d + 1] = g;
                            out[d + 2] = r;
                            out[d + 3] = 0xFF;
                        }
                    }
                }
            }
            self.scratch = out;
        }

        /// Copy `self.scratch` to the screen, returning the dirty rectangle.
        fn commit(&mut self) -> Option<EinkRect> {
            let line = self.line_length;
            let bpp = self.bytes_per_pixel;
            let row_off = self.xoffset as usize * bpp;
            let (xres, yres) = (self.xres as usize, self.yres as usize);
            let new = std::mem::take(&mut self.scratch);
            let screen = self.visible_slice_mut();

            let (mut min_x, mut min_y, mut max_x, mut max_y) = (usize::MAX, usize::MAX, 0usize, 0usize);
            for y in 0..yres {
                let a = y * line + row_off;
                let b = (a + xres * bpp).min(new.len()).min(screen.len());
                if a >= b || new[a..b] == screen[a..b] {
                    continue;
                }
                let first = (a..b).find(|&i| new[i] != screen[i]).unwrap_or(a);
                let last = (a..b).rev().find(|&i| new[i] != screen[i]).unwrap_or(b - 1);
                screen[a..b].copy_from_slice(&new[a..b]);
                min_y = min_y.min(y);
                max_y = y;
                min_x = min_x.min((first - a) / bpp);
                max_x = max_x.max((last - a) / bpp);
            }
            self.scratch = new;

            if min_y == usize::MAX {
                return None;
            }
            Some(EinkRect {
                top: min_y as u32,
                left: min_x as u32,
                width: (max_x - min_x + 1) as u32,
                height: (max_y - min_y + 1) as u32,
            })
        }

        fn full_rect(&self) -> EinkRect {
            EinkRect { top: 0, left: 0, width: self.xres, height: self.yres }
        }

        fn send_update(&mut self, rect: EinkRect, full: bool) {
            self.marker = self.marker.wrapping_add(1).max(1);
            let fd = self.file.as_raw_fd();
            let order: &[EinkBackend] = match self.backend {
                EinkBackend::Unknown => &[EinkBackend::Hwtcon, EinkBackend::MxcfbV2, EinkBackend::MxcfbV1, EinkBackend::MxcfbV1Ntx, EinkBackend::Fbink],
                EinkBackend::Hwtcon => &[EinkBackend::Hwtcon],
                EinkBackend::MxcfbV2 => &[EinkBackend::MxcfbV2],
                EinkBackend::MxcfbV1 => &[EinkBackend::MxcfbV1],
                EinkBackend::MxcfbV1Ntx => &[EinkBackend::MxcfbV1Ntx],
                EinkBackend::Fbink => &[EinkBackend::Fbink],
                EinkBackend::None => return,
            };

            for &backend in order {
                let ok = match backend {
                    EinkBackend::Hwtcon => {
                        let (waveform, mode) = match (self.color_panel, full) {
                            (true, true) => (WAVEFORM_MODE_GCC16, UPDATE_MODE_FULL),
                            (true, false) => (WAVEFORM_MODE_GLRC16, UPDATE_MODE_PARTIAL),
                            (false, true) => (WAVEFORM_MODE_GC16, UPDATE_MODE_FULL),
                            (false, false) => (WAVEFORM_MODE_AUTO, UPDATE_MODE_PARTIAL),
                        };
                        let data = HwtconUpdateData {
                            update_region: rect,
                            waveform_mode: waveform,
                            update_mode: mode,
                            update_marker: self.marker,
                            ..Default::default()
                        };
                        let mut ret = unsafe { libc::ioctl(fd, SEND_UPDATE_HWTCON as _, &data) };
                        if ret < 0 && self.color_panel {
                            // Panel/driver without colour waveforms: retry plain.
                            let data = HwtconUpdateData {
                                update_region: rect,
                                waveform_mode: if full { WAVEFORM_MODE_GC16 } else { WAVEFORM_MODE_AUTO },
                                update_mode: mode,
                                update_marker: self.marker,
                                ..Default::default()
                            };
                            ret = unsafe { libc::ioctl(fd, SEND_UPDATE_HWTCON as _, &data) };
                            if ret >= 0 {
                                self.color_panel = false;
                            }
                        }
                        ret >= 0
                    }
                    EinkBackend::MxcfbV2 => {
                        let data = MxcfbUpdateDataV2 {
                            update_region: rect,
                            waveform_mode: if full { WAVEFORM_MODE_GC16 } else { WAVEFORM_MODE_AUTO },
                            update_mode: if full { UPDATE_MODE_FULL } else { UPDATE_MODE_PARTIAL },
                            update_marker: self.marker,
                            temp: TEMP_USE_AMBIENT,
                            ..Default::default()
                        };
                        unsafe { libc::ioctl(fd, SEND_UPDATE_MXCFB_V2 as _, &data) >= 0 }
                    }
                    EinkBackend::MxcfbV1 => {
                        let data = MxcfbUpdateDataV1 {
                            update_region: rect,
                            waveform_mode: if full { WAVEFORM_MODE_GC16 } else { WAVEFORM_MODE_AUTO },
                            update_mode: if full { UPDATE_MODE_FULL } else { UPDATE_MODE_PARTIAL },
                            update_marker: self.marker,
                            temp: TEMP_USE_AMBIENT,
                            ..Default::default()
                        };
                        unsafe { libc::ioctl(fd, SEND_UPDATE_MXCFB_V1 as _, &data) >= 0 }
                    }
                    EinkBackend::MxcfbV1Ntx => {
                        let data = MxcfbUpdateDataV1Ntx {
                            update_region: rect,
                            waveform_mode: if full { WAVEFORM_MODE_GC16 } else { WAVEFORM_MODE_AUTO },
                            update_mode: if full { UPDATE_MODE_FULL } else { UPDATE_MODE_PARTIAL },
                            update_marker: self.marker,
                            temp: TEMP_USE_AMBIENT,
                            ..Default::default()
                        };
                        unsafe { libc::ioctl(fd, SEND_UPDATE_MXCFB_V1_NTX as _, &data) >= 0 }
                    }
                    EinkBackend::Fbink => fbink_refresh(full),
                    _ => false,
                };

                if ok {
                    if self.backend != backend {
                        tracing::info!("E-ink refresh backend: {:?}", backend);
                        self.backend = backend;
                    }
                    return;
                }
            }

            if self.backend == EinkBackend::Unknown {
                tracing::error!("No working e-ink refresh method found (ioctl and fbink both failed); the screen will not update");
                self.backend = EinkBackend::None;
            }
        }
    }

    fn fbink_refresh(full: bool) -> bool {
        let args: &[&str] = if full { &["-q", "-f", "-s"] } else { &["-q", "-s"] };
        for bin in ["/mnt/onboard/.kobo-ha/fbink", "/mnt/onboard/.adds/kfmon/bin/fbink", "/usr/local/kfmon/bin/fbink", "fbink"] {
            if let Ok(status) = std::process::Command::new(bin)
                .args(args)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
            {
                return status.success();
            }
        }
        false
    }

    impl FramebufferDriver for LinuxFramebuffer {
        fn flush(&mut self, canvas: &Canvas, refresh: Refresh) -> Result<bool, Box<dyn Error + Send + Sync>> {
            self.compose(canvas);
            let dirty = self.commit();
            match (refresh, dirty) {
                (Refresh::Full, _) | (Refresh::FullIfChanged, Some(_)) => {
                    let rect = self.full_rect();
                    self.send_update(rect, true);
                }
                (Refresh::Partial, Some(rect)) => self.send_update(rect, false),
                _ => {}
            }
            Ok(dirty.is_some())
        }

        fn dimensions(&self) -> (u32, u32) {
            (self.xres, self.yres)
        }

        fn restore(&mut self) {
            if let Some(saved) = self.saved.take() {
                let screen = self.visible_slice_mut();
                let n = saved.len().min(screen.len());
                screen[..n].copy_from_slice(&saved[..n]);
                let rect = self.full_rect();
                self.send_update(rect, true);
            }
        }
    }

    impl Drop for LinuxFramebuffer {
        fn drop(&mut self) {
            self.restore();
            unsafe {
                libc::munmap(self.mmap_ptr as *mut libc::c_void, self.mmap_len);
            }
        }
    }
}

#[cfg(target_os = "linux")]
pub use linux::{probe_resolution, LinuxFramebuffer};

/// Visible framebuffer resolution, if there is a framebuffer.
#[cfg(not(target_os = "linux"))]
pub fn probe_resolution(_path: &str) -> Option<(u32, u32)> {
    None
}

/// Fallback / Mock Framebuffer for macOS / Windows desktop development
pub struct MockFramebuffer {
    width: u32,
    height: u32,
}

impl MockFramebuffer {
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }
}

impl FramebufferDriver for MockFramebuffer {
    fn flush(&mut self, _canvas: &Canvas, _refresh: Refresh) -> Result<bool, Box<dyn Error + Send + Sync>> {
        // On desktop, canvas is served via web simulator
        Ok(true)
    }

    fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}

#[allow(unused_variables)]
pub fn create_framebuffer(
    path: &str,
    canvas_w: u32,
    canvas_h: u32,
    canvas_rotation: Rotation,
    fb_rotation: Option<Rotation>,
    color_panel: bool,
) -> Box<dyn FramebufferDriver> {
    #[cfg(target_os = "linux")]
    {
        if std::path::Path::new(path).exists() {
            match LinuxFramebuffer::open(path, canvas_w, canvas_h, canvas_rotation, fb_rotation, color_panel) {
                Ok(fb) => return Box::new(fb),
                Err(e) => {
                    tracing::error!("Failed to open Linux framebuffer at {}: {}", path, e);
                }
            }
        }
    }

    Box::new(MockFramebuffer::new(canvas_w, canvas_h))
}

#[cfg(test)]
mod tests {
    use super::Rotation;

    #[test]
    fn rotation_inverse_round_trips() {
        let (w, h) = (1072.0, 1448.0);
        for rot in [Rotation::R0, Rotation::R90, Rotation::R180, Rotation::R270] {
            let (rw, rh) = if rot.swaps_axes() { (h, w) } else { (w, h) };
            for (x, y) in [(0.0, 0.0), (10.0, 20.0), (w - 1.0, h - 1.0), (500.0, 1000.0)] {
                let (rx, ry) = rot.map(x, y, w, h);
                assert!(rx >= 0.0 && rx < rw && ry >= 0.0 && ry < rh, "{:?} out of bounds", rot);
                let (bx, by) = rot.inverse().map(rx, ry, rw, rh);
                assert_eq!((bx, by), (x, y), "{:?}", rot);
            }
        }
    }

    #[test]
    fn rotation_composes() {
        assert_eq!(Rotation::R90.then(Rotation::R270), Rotation::R0);
        assert_eq!(Rotation::R90.then(Rotation::R90), Rotation::R180);
        assert_eq!(Rotation::R270.inverse(), Rotation::R90);
    }
}
