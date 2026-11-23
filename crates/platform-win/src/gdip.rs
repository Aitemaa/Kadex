//! Anti-aliased vector chrome for the sticky header, drawn with GDI+ (the flat API in `gdiplus.dll`,
//! present on every Windows version). The icon geometry comes from `kadex_core::icons`, so it is the
//! same lucide-style data the Wayland frontend strokes with Cairo.

#![cfg(target_os = "windows")]

use std::sync::atomic::{AtomicUsize, Ordering};

use kadex_core::icons::{Icon, Seg, GRID, STROKE};
use windows_sys::Win32::Graphics::Gdi::HDC;
use windows_sys::Win32::Graphics::GdiPlus::{
    DashCapRound, FillModeAlternate, GdipAddPathArc, GdipAddPathBezier, GdipAddPathLine,
    GdipClosePathFigure, GdipCreateFromHDC, GdipCreatePath, GdipCreatePen1, GdipCreateSolidFill,
    GdipDeleteBrush, GdipDeleteGraphics, GdipDeletePath, GdipDeletePen, GdipDrawEllipse,
    GdipDrawPath, GdipFillEllipse, GdipFillPath, GdipSetPenLineCap197819, GdipSetPenLineJoin,
    GdipSetSmoothingMode, GdipStartPathFigure, GdiplusShutdown, GdiplusStartup,
    GdiplusStartupInput, GdiplusStartupOutput, GpBrush, GpGraphics, GpPath, GpPen, GpSolidFill,
    LineCapRound, LineJoinRound, SmoothingModeAntiAlias, UnitPixel,
};

static TOKEN: AtomicUsize = AtomicUsize::new(0);

/// Starts GDI+ on first use (kept off the cold-start path; only the first header paint pays).
fn ensure_started() -> bool {
    if TOKEN.load(Ordering::Relaxed) != 0 {
        return true;
    }
    let input = GdiplusStartupInput {
        GdiplusVersion: 1,
        DebugEventCallback: 0,
        SuppressBackgroundThread: 0,
        SuppressExternalCodecs: 1,
    };
    let mut token = 0usize;
    let status = unsafe {
        GdiplusStartup(
            &mut token,
            &input,
            std::ptr::null_mut::<GdiplusStartupOutput>(),
        )
    };
    if status != 0 || token == 0 {
        return false;
    }
    TOKEN.store(token, Ordering::Relaxed);
    true
}

pub fn shutdown() {
    let token = TOKEN.swap(0, Ordering::Relaxed);
    if token != 0 {
        unsafe { GdiplusShutdown(token) };
    }
}

/// `0xRRGGBB` + alpha (0-255) -> GDI+ ARGB.
pub fn argb(rgb: u32, alpha: u8) -> u32 {
    ((alpha as u32) << 24) | (rgb & 0x00FF_FFFF)
}

/// An anti-aliased drawing surface over a device context.
pub struct Canvas {
    g: *mut GpGraphics,
}

impl Canvas {
    pub fn new(hdc: HDC) -> Option<Canvas> {
        if !ensure_started() {
            return None;
        }
        let mut g: *mut GpGraphics = std::ptr::null_mut();
        unsafe {
            if GdipCreateFromHDC(hdc, &mut g) != 0 || g.is_null() {
                return None;
            }
            GdipSetSmoothingMode(g, SmoothingModeAntiAlias);
        }
        Some(Canvas { g })
    }

    fn fill_path(&self, path: *mut GpPath, color: u32) {
        unsafe {
            let mut brush: *mut GpSolidFill = std::ptr::null_mut();
            if GdipCreateSolidFill(color, &mut brush) == 0 {
                GdipFillPath(self.g, brush as *mut GpBrush, path);
                GdipDeleteBrush(brush as *mut GpBrush);
            }
        }
    }

    fn pen(&self, width: f32, color: u32, round: bool) -> *mut GpPen {
        let mut pen: *mut GpPen = std::ptr::null_mut();
        unsafe {
            if GdipCreatePen1(color, width, UnitPixel, &mut pen) != 0 {
                return std::ptr::null_mut();
            }
            if round {
                GdipSetPenLineCap197819(pen, LineCapRound, LineCapRound, DashCapRound);
                GdipSetPenLineJoin(pen, LineJoinRound);
            }
        }
        pen
    }

    pub fn fill_round_rect(&self, x: f32, y: f32, w: f32, h: f32, r: f32, color: u32) {
        let d = r * 2.0;
        unsafe {
            let mut path: *mut GpPath = std::ptr::null_mut();
            if GdipCreatePath(FillModeAlternate, &mut path) != 0 {
                return;
            }
            GdipAddPathArc(path, x, y, d, d, 180.0, 90.0);
            GdipAddPathArc(path, x + w - d, y, d, d, 270.0, 90.0);
            GdipAddPathArc(path, x + w - d, y + h - d, d, d, 0.0, 90.0);
            GdipAddPathArc(path, x, y + h - d, d, d, 90.0, 90.0);
            GdipClosePathFigure(path);
            self.fill_path(path, color);
            GdipDeletePath(path);
        }
    }

    pub fn fill_circle(&self, cx: f32, cy: f32, r: f32, color: u32) {
        unsafe {
            let mut brush: *mut GpSolidFill = std::ptr::null_mut();
            if GdipCreateSolidFill(color, &mut brush) == 0 {
                GdipFillEllipse(
                    self.g,
                    brush as *mut GpBrush,
                    cx - r,
                    cy - r,
                    r * 2.0,
                    r * 2.0,
                );
                GdipDeleteBrush(brush as *mut GpBrush);
            }
        }
    }

    pub fn stroke_circle(&self, cx: f32, cy: f32, r: f32, width: f32, color: u32) {
        let pen = self.pen(width, color, false);
        if pen.is_null() {
            return;
        }
        unsafe {
            GdipDrawEllipse(self.g, pen, cx - r, cy - r, r * 2.0, r * 2.0);
            GdipDeletePen(pen);
        }
    }

    /// Strokes `icon` centred on `(cx, cy)` in a `px`-sized square (stroke 2 grid units, round caps/joins).
    pub fn stroke_icon(&self, icon: Icon, cx: f32, cy: f32, px: f32, color: u32) {
        let k = px / GRID;
        let (ox, oy) = (cx - px / 2.0, cy - px / 2.0);
        let pen = self.pen(STROKE * k, color, true);
        if pen.is_null() {
            return;
        }
        unsafe {
            let mut path: *mut GpPath = std::ptr::null_mut();
            if GdipCreatePath(FillModeAlternate, &mut path) != 0 {
                GdipDeletePen(pen);
                return;
            }
            let (mut cur_x, mut cur_y) = (0.0f32, 0.0f32);
            let (mut start_x, mut start_y) = (0.0f32, 0.0f32);
            for seg in icon.segments() {
                match *seg {
                    Seg::M(x, y) => {
                        GdipStartPathFigure(path);
                        (cur_x, cur_y) = (ox + x * k, oy + y * k);
                        (start_x, start_y) = (cur_x, cur_y);
                    }
                    Seg::L(x, y) => {
                        let (nx, ny) = (ox + x * k, oy + y * k);
                        GdipAddPathLine(path, cur_x, cur_y, nx, ny);
                        (cur_x, cur_y) = (nx, ny);
                    }
                    Seg::C(x1, y1, x2, y2, x3, y3) => {
                        let (nx, ny) = (ox + x3 * k, oy + y3 * k);
                        GdipAddPathBezier(
                            path,
                            cur_x,
                            cur_y,
                            ox + x1 * k,
                            oy + y1 * k,
                            ox + x2 * k,
                            oy + y2 * k,
                            nx,
                            ny,
                        );
                        (cur_x, cur_y) = (nx, ny);
                    }
                    Seg::Z => {
                        GdipClosePathFigure(path);
                        (cur_x, cur_y) = (start_x, start_y);
                    }
                }
            }
            GdipDrawPath(self.g, pen, path);
            GdipDeletePath(path);
            GdipDeletePen(pen);
        }
    }
}

impl Drop for Canvas {
    fn drop(&mut self) {
        unsafe { GdipDeleteGraphics(self.g) };
    }
}
