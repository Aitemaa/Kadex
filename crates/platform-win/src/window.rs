//! Native Win32 sticky & file windows using MSFTEDIT_CLASS (RichEdit), 28px GDI header, DWM dark/rounded corners, and Per-Monitor DPI v2.

#![cfg(target_os = "windows")]

use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::atomic::{AtomicPtr, Ordering};

use kadex_core::config::{palette, Palette, PALETTE_COUNT, RED};
use kadex_core::icons::Icon;
use kadex_core::storage::{Encoding, LineEnding};

use windows_sys::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_CAPTION_COLOR, DWMWA_TEXT_COLOR,
    DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
};
use windows_sys::Win32::Graphics::Gdi::{
    AddFontMemResourceEx, BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC,
    CreateFontW, CreateSolidBrush, DeleteDC, DeleteObject, DrawTextW, EndPaint, FillRect,
    InvalidateRect, RemoveFontMemResourceEx, SelectObject, SetBkMode, SetTextColor,
    CLEARTYPE_QUALITY, DEFAULT_CHARSET, DEFAULT_PITCH, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX,
    DT_SINGLELINE, DT_VCENTER, FF_DONTCARE, FW_NORMAL, HBRUSH, HFONT, OUT_DEFAULT_PRECIS,
    PAINTSTRUCT, SRCCOPY, TRANSPARENT,
};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, LoadLibraryW};
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    SetFocus, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT,
};

const EM_SETMODIFY: u32 = 0x00B9;
const EM_SETBKGNDCOLOR: u32 = 0x0443;
const EM_SETCHARFORMAT: u32 = 0x0444;
const EM_SETEVENTMASK: u32 = 0x0445;
const EM_SETZOOM: u32 = 0x04E1;
const EM_EXGETSEL: u32 = 0x0434;
const EM_EXSETSEL: u32 = 0x0437;
const ENM_CHANGE: u32 = 0x0000_0001;
const CFM_COLOR: u32 = 0x4000_0000;
const SCF_DEFAULT: u32 = 0x0000;
const SCF_ALL: u32 = 0x0004;

#[repr(C)]
#[allow(non_snake_case)]
struct CHARFORMAT2W {
    cbSize: u32,
    dwMask: u32,
    dwEffects: u32,
    yHeight: i32,
    yOffset: i32,
    crTextColor: COLORREF,
    bCharSet: u8,
    bPitchAndFamily: u8,
    szFaceName: [u16; 32],
    wWeight: u16,
    sSpacing: i16,
    crBackColor: COLORREF,
    lcid: u32,
    dwReserved: u32,
    sStyle: i16,
    wKerning: u16,
    bUnderlineType: u8,
    bAnimation: u8,
    bRevAuthor: u8,
    bUnderlineColor: u8,
}
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, GetClientRect, GetWindowLongPtrW, GetWindowRect,
    GetWindowTextLengthW, GetWindowTextW, IsWindowVisible, KillTimer, MoveWindow, SendMessageW,
    SetForegroundWindow, SetLayeredWindowAttributes, SetTimer, SetWindowLongPtrW, SetWindowPos,
    SetWindowTextW, ShowWindow, ES_AUTOHSCROLL, ES_AUTOVSCROLL, ES_MULTILINE, ES_NOHIDESEL,
    GWLP_USERDATA, HTCAPTION, HWND_NOTOPMOST, HWND_TOPMOST, LWA_ALPHA, SWP_NOMOVE, SWP_NOSIZE,
    SW_HIDE, SW_SHOW, WM_NCLBUTTONDOWN, WM_SETFONT, WS_CHILD, WS_CLIPCHILDREN, WS_EX_LAYERED,
    WS_EX_TOPMOST, WS_OVERLAPPEDWINDOW, WS_POPUP, WS_THICKFRAME, WS_VISIBLE, WS_VSCROLL,
};

use crate::gdip::{argb, Canvas};
use crate::theme::{rgb_to_colorref, to_wide_null};

pub const HEADER_H_96: i32 = 28;

// Kavox family header geometry, in 96-DPI px (scaled by the window DPI at paint and hit-test time).
const BTN: f32 = 24.0;
const BTN_RADIUS: f32 = 7.0;
const ICON_PX: f32 = 16.0;
/// Alpha of the red hover tint behind the danger button (Kavox: 30).
const DANGER_TINT: u8 = 30;

/// Archivo Medium (SIL OFL 1.1, Latin subset), embedded and loaded privately for header text.
const ARCHIVO_MEDIUM: &[u8] = include_bytes!("../../../assets/fonts/Archivo-Medium.ttf");
/// GDI face name of the embedded font (its legacy family name).
const UI_FACE: &str = "Archivo Medium";
/// Used when the embedded font could not be loaded.
const UI_FACE_FALLBACK: &str = "Segoe UI";

static UI_FONT_HANDLE: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

/// Makes the embedded font available to GDI for this process only (nothing is installed).
pub fn install_ui_font() {
    let mut count = 0u32;
    let handle = unsafe {
        AddFontMemResourceEx(
            ARCHIVO_MEDIUM.as_ptr() as *const c_void,
            ARCHIVO_MEDIUM.len() as u32,
            std::ptr::null(),
            std::ptr::addr_of_mut!(count), // windows-sys types the out-parameter as `*const`
        )
    };
    if count > 0 {
        UI_FONT_HANDLE.store(handle, Ordering::Relaxed);
    }
}

/// Releases the private font; call once on exit.
pub fn remove_ui_font() {
    let handle = UI_FONT_HANDLE.swap(std::ptr::null_mut(), Ordering::Relaxed);
    if !handle.is_null() {
        unsafe { RemoveFontMemResourceEx(handle) };
    }
}

fn ui_face() -> &'static str {
    if UI_FONT_HANDLE.load(Ordering::Relaxed).is_null() {
        UI_FACE_FALLBACK
    } else {
        UI_FACE
    }
}

/// The note's title for the header: its first non-empty line.
fn first_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(|l| l.chars().take(80).collect())
        .unwrap_or_default()
}
pub const TIMER_AUTOSAVE: usize = 1;
pub const NOTE_CLASS_NAME: &str = "KadexNoteWindow";

#[derive(Debug, Clone)]
pub enum WinMode {
    Sticky {
        note_id: String,
    },
    File {
        path: PathBuf,
        encoding: Encoding,
        line_ending: LineEnding,
    },
}

pub struct NoteWindow {
    pub hwnd: HWND,
    pub edit_hwnd: HWND,
    pub mode: WinMode,
    pub palette_idx: usize,
    pub dark: bool,
    pub pinned: bool,
    #[allow(dead_code)]
    pub word_wrap: bool,
    pub dirty: bool,
    pub font_family: String,
    pub font_size: f32,
    pub zoom_num: i32,
    pub opacity: f32,
    pub font_handle: HFONT,
    pub bg_brush: HBRUSH,
    /// Header chrome font (Archivo Medium).
    pub ui_font: HFONT,
    /// Header control under the pointer.
    pub hover: HeaderHit,
    /// A `WM_MOUSELEAVE` is pending for this window.
    pub tracking: bool,
    /// Header title: the note's first line.
    pub title: String,
}

impl NoteWindow {
    pub fn is_sticky(&self) -> bool {
        matches!(self.mode, WinMode::Sticky { .. })
    }

    pub fn palette(&self) -> Palette {
        palette(self.palette_idx, self.dark)
    }

    pub fn header_px(&self) -> i32 {
        if !self.is_sticky() {
            return 0;
        }
        let dpi = unsafe { GetDpiForWindow(self.hwnd) };
        let scale = if dpi == 0 { 96 } else { dpi };
        (HEADER_H_96 * scale as i32) / 96
    }

    /// Window DPI / 96.
    fn dpi_scale(&self) -> f32 {
        let dpi = unsafe { GetDpiForWindow(self.hwnd) };
        (if dpi == 0 { 96 } else { dpi }) as f32 / 96.0
    }

    pub fn apply_theme(&mut self) {
        let pal = self.palette();
        unsafe {
            let dark_val: i32 = if self.dark { 1 } else { 0 };
            let _ = DwmSetWindowAttribute(
                self.hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE as u32,
                (&dark_val as *const i32) as *const c_void,
                std::mem::size_of::<i32>() as u32,
            );
            let corner = DWMWCP_ROUND;
            let _ = DwmSetWindowAttribute(
                self.hwnd,
                DWMWA_WINDOW_CORNER_PREFERENCE as u32,
                (&corner as *const i32) as *const c_void,
                std::mem::size_of::<i32>() as u32,
            );

            // Windows 11 (build 22000+): caption, caption text and border follow the palette.
            // Older builds reject these attributes, which is fine.
            for (attr, rgb) in [
                (DWMWA_CAPTION_COLOR, pal.header),
                (DWMWA_TEXT_COLOR, pal.fg),
                (DWMWA_BORDER_COLOR, pal.border),
            ] {
                let color: COLORREF = rgb_to_colorref(rgb);
                let _ = DwmSetWindowAttribute(
                    self.hwnd,
                    attr as u32,
                    (&color as *const COLORREF) as *const c_void,
                    std::mem::size_of::<COLORREF>() as u32,
                );
            }

            if !self.bg_brush.is_null() {
                DeleteObject(self.bg_brush as *mut c_void);
            }
            self.bg_brush = CreateSolidBrush(rgb_to_colorref(pal.bg));

            if !self.edit_hwnd.is_null() {
                SendMessageW(
                    self.edit_hwnd,
                    EM_SETBKGNDCOLOR,
                    0,
                    rgb_to_colorref(pal.bg) as LPARAM,
                );
            }
            self.apply_text_color();
            InvalidateRect(self.hwnd, std::ptr::null(), 1);
        }
    }

    /// Sets the text colour on the existing text *and* the default format, otherwise text
    /// typed into an empty note uses RichEdit's default black, unreadable on dark palettes.
    fn apply_text_color(&self) {
        if self.edit_hwnd.is_null() {
            return;
        }
        let pal = self.palette();
        unsafe {
            let mut cf: CHARFORMAT2W = std::mem::zeroed();
            cf.cbSize = std::mem::size_of::<CHARFORMAT2W>() as u32;
            cf.dwMask = CFM_COLOR;
            cf.crTextColor = rgb_to_colorref(pal.fg);
            for scope in [SCF_DEFAULT, SCF_ALL] {
                SendMessageW(
                    self.edit_hwnd,
                    EM_SETCHARFORMAT,
                    scope as WPARAM,
                    (&cf as *const CHARFORMAT2W) as LPARAM,
                );
            }
        }
    }

    pub fn apply_font(&mut self) {
        let dpi = unsafe { GetDpiForWindow(self.hwnd) };
        let dpi = if dpi == 0 { 96 } else { dpi };
        let height = -((self.font_size * dpi as f32 / 72.0).round() as i32);
        // Generic fontconfig names (the Linux defaults) aren't real Windows fonts.
        let family = match self.font_family.to_ascii_lowercase().as_str() {
            "monospace" | "mono" => "Consolas",
            "sans-serif" | "sans" => "Segoe UI",
            "serif" => "Georgia",
            _ => self.font_family.as_str(),
        };
        let face = to_wide_null(family);
        unsafe {
            let hfont = CreateFontW(
                height,
                0,
                0,
                0,
                FW_NORMAL as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET as u32,
                OUT_DEFAULT_PRECIS as u32,
                0,
                CLEARTYPE_QUALITY as u32,
                (DEFAULT_PITCH | FF_DONTCARE) as u32,
                face.as_ptr(),
            );
            if !hfont.is_null() {
                if !self.font_handle.is_null() {
                    DeleteObject(self.font_handle as *mut c_void);
                }
                self.font_handle = hfont;
                if !self.edit_hwnd.is_null() {
                    SendMessageW(self.edit_hwnd, WM_SETFONT, hfont as WPARAM, 1);
                }
            }
        }
        // WM_SETFONT resets the character format, colour included.
        self.apply_text_color();

        // Header chrome font: Archivo Medium, 9 pt (12 px at 96 DPI).
        let ui_height = -((9.0 * dpi as f32 / 72.0).round() as i32);
        let ui_face = to_wide_null(ui_face());
        unsafe {
            let hfont = CreateFontW(
                ui_height,
                0,
                0,
                0,
                FW_NORMAL as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET as u32,
                OUT_DEFAULT_PRECIS as u32,
                0,
                CLEARTYPE_QUALITY as u32,
                (DEFAULT_PITCH | FF_DONTCARE) as u32,
                ui_face.as_ptr(),
            );
            if !hfont.is_null() {
                if !self.ui_font.is_null() {
                    DeleteObject(self.ui_font);
                }
                self.ui_font = hfont;
            }
        }
    }

    pub fn apply_opacity(&self) {
        if self.is_sticky() {
            let alpha = (self.opacity.clamp(0.3, 1.0) * 255.0).round() as u8;
            unsafe {
                SetLayeredWindowAttributes(self.hwnd, 0, alpha, LWA_ALPHA);
            }
        }
    }

    pub fn toggle_pin(&mut self) {
        self.pinned = !self.pinned;
        let insert_after = if self.pinned {
            HWND_TOPMOST
        } else {
            HWND_NOTOPMOST
        };
        unsafe {
            SetWindowPos(self.hwnd, insert_after, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE);
            InvalidateRect(self.hwnd, std::ptr::null(), 0);
        }
    }

    pub fn zoom(&mut self, delta: i32) {
        self.zoom_num = (self.zoom_num + delta).clamp(50, 300);
        unsafe {
            SendMessageW(self.edit_hwnd, EM_SETZOOM, self.zoom_num as WPARAM, 100);
        }
    }

    pub fn reset_zoom(&mut self) {
        self.zoom_num = 100;
        unsafe {
            SendMessageW(self.edit_hwnd, EM_SETZOOM, 0, 0);
        }
    }

    pub fn layout_children(&self) {
        unsafe {
            let mut rc: RECT = std::mem::zeroed();
            GetClientRect(self.hwnd, &mut rc);
            let top = self.header_px();
            let w = (rc.right - rc.left).max(1);
            let h = (rc.bottom - rc.top - top).max(1);
            if !self.edit_hwnd.is_null() {
                MoveWindow(self.edit_hwnd, 0, top, w, h, 1);
            }
        }
    }

    pub fn set_text(&mut self, text: &str) {
        let crlf = text.replace('\n', "\r\n");
        let wide = to_wide_null(&crlf);
        unsafe {
            SetWindowTextW(self.edit_hwnd, wide.as_ptr());
            SendMessageW(self.edit_hwnd, EM_SETMODIFY, 0, 0);
        }
        self.dirty = false;
        self.refresh_title(text);
    }

    /// Updates the header title from the note text and repaints the header if it changed.
    pub fn refresh_title(&mut self, text: &str) {
        if !self.is_sticky() {
            return;
        }
        let title = first_line(text);
        if title != self.title {
            self.title = title;
            self.invalidate_header();
        }
    }

    fn invalidate_header(&self) {
        unsafe {
            let mut rc: RECT = std::mem::zeroed();
            GetClientRect(self.hwnd, &mut rc);
            rc.bottom = self.header_px();
            InvalidateRect(self.hwnd, &rc, 0);
        }
    }

    pub fn get_text(&self) -> String {
        unsafe {
            let len = GetWindowTextLengthW(self.edit_hwnd);
            if len <= 0 {
                return String::new();
            }
            let mut buf = vec![0u16; (len + 1) as usize];
            let copied = GetWindowTextW(self.edit_hwnd, buf.as_mut_ptr(), len + 1);
            String::from_utf16_lossy(&buf[..copied as usize]).replace("\r\n", "\n")
        }
    }

    pub fn schedule_autosave(&self, delay_ms: u64) {
        if self.is_sticky() {
            unsafe {
                SetTimer(self.hwnd, TIMER_AUTOSAVE, delay_ms as u32, None);
            }
        }
    }

    pub fn cancel_autosave(&self) {
        unsafe {
            KillTimer(self.hwnd, TIMER_AUTOSAVE);
        }
    }

    pub fn rect(&self) -> (i32, i32, u32, u32) {
        unsafe {
            let mut rc: RECT = std::mem::zeroed();
            GetWindowRect(self.hwnd, &mut rc);
            (
                rc.left,
                rc.top,
                (rc.right - rc.left).max(220) as u32,
                (rc.bottom - rc.top).max(160) as u32,
            )
        }
    }

    pub fn is_visible(&self) -> bool {
        unsafe { IsWindowVisible(self.hwnd) != 0 }
    }

    pub fn focus(&self) {
        unsafe {
            SetForegroundWindow(self.hwnd);
            if !self.edit_hwnd.is_null() {
                SetFocus(self.edit_hwnd);
            }
        }
    }

    pub fn show(&self, visible: bool) {
        unsafe {
            ShowWindow(self.hwnd, if visible { SW_SHOW } else { SW_HIDE });
        }
    }

    pub fn paint_header(&self) {
        if !self.is_sticky() {
            return;
        }
        unsafe {
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let hdc = BeginPaint(self.hwnd, &mut ps);
            let mut rc: RECT = std::mem::zeroed();
            GetClientRect(self.hwnd, &mut rc);
            let (w, hh) = (rc.right.max(1), self.header_px().max(1));
            let k = self.dpi_scale();
            let pal = self.palette();

            // Double-buffer: flicker-free hover repaints.
            let mem = CreateCompatibleDC(hdc);
            let bmp = CreateCompatibleBitmap(hdc, w, hh);
            let old_bmp = SelectObject(mem, bmp);

            // `surface` fill and a 1 px `line` bottom border.
            let fill = |r: RECT, rgb: u32| {
                let br = CreateSolidBrush(rgb_to_colorref(rgb));
                FillRect(mem, &r, br);
                DeleteObject(br);
            };
            fill(
                RECT {
                    left: 0,
                    top: 0,
                    right: w,
                    bottom: hh,
                },
                pal.header,
            );
            fill(
                RECT {
                    left: 0,
                    top: hh - 1,
                    right: w,
                    bottom: hh,
                },
                pal.border,
            );

            // Title: first line of the note, Archivo Medium, `muted`, ellipsised.
            let wf = w as f32 / k;
            let title_left = (self.dots_start() + PALETTE_COUNT as f32 * 16.0 + 4.0) * k;
            let title_right = (wf - 76.0 - 6.0) * k;
            if !self.title.is_empty()
                && title_right - title_left > 40.0 * k
                && !self.ui_font.is_null()
            {
                let text = to_wide_null(&self.title);
                let mut tr = RECT {
                    left: title_left as i32,
                    top: 0,
                    right: title_right as i32,
                    bottom: hh - 1,
                };
                let old_font = SelectObject(mem, self.ui_font);
                SetBkMode(mem, TRANSPARENT as i32);
                SetTextColor(mem, rgb_to_colorref(pal.muted));
                DrawTextW(
                    mem,
                    text.as_ptr(),
                    -1,
                    &mut tr,
                    DT_SINGLELINE | DT_VCENTER | DT_LEFT | DT_END_ELLIPSIS | DT_NOPREFIX,
                );
                SelectObject(mem, old_font);
            }

            // Vector chrome (GDI+, anti-aliased).
            if let Some(g) = Canvas::new(mem) {
                let cy = hh as f32 * 0.5;
                let button = |cx_96: f32, icon: Icon, which: HeaderHit, active: bool| {
                    let cx = cx_96 * k;
                    let (x, y, size, r) = (
                        cx - BTN * k / 2.0,
                        cy - BTN * k / 2.0,
                        BTN * k,
                        BTN_RADIUS * k,
                    );
                    let hovered = self.hover == which;
                    let danger = which == HeaderHit::Close;
                    let mut color = pal.muted;
                    if active {
                        g.fill_round_rect(x, y, size, size, r, argb(pal.lit, 255));
                        color = pal.on_lit;
                    } else if hovered && danger {
                        g.fill_round_rect(x, y, size, size, r, argb(RED, DANGER_TINT));
                        color = pal.red_text;
                    } else if hovered {
                        g.fill_round_rect(x, y, size, size, r, argb(pal.hover, 255));
                        color = pal.fg;
                    }
                    g.stroke_icon(icon, cx, cy, ICON_PX * k, argb(color, 255));
                };
                button(16.0, Icon::Plus, HeaderHit::New, false);
                button(wf - 64.0, Icon::Pin, HeaderHit::Pin, self.pinned);
                button(wf - 40.0, Icon::Archive, HeaderHit::Archive, false);
                button(wf - 14.0, Icon::Close, HeaderHit::Close, false);

                // Palette dots: accent fill, 1 px `line_strong` ring; active = `text` halo.
                let ring = k.max(1.0);
                for i in 0..PALETTE_COUNT {
                    let dp = palette(i, self.dark);
                    let cx = (self.dots_start() + i as f32 * 16.0) * k;
                    g.fill_circle(cx, cy, 4.5 * k, argb(dp.accent, 255));
                    g.stroke_circle(cx, cy, 4.5 * k, ring, argb(pal.border_strong, 255));
                    if i == self.palette_idx {
                        g.stroke_circle(cx, cy, 7.0 * k, 1.5 * k, argb(pal.fg, 255));
                    } else if self.hover == HeaderHit::Palette(i) {
                        g.stroke_circle(cx, cy, 7.0 * k, ring, argb(pal.muted, 255));
                    }
                }
            }

            BitBlt(hdc, 0, 0, w, hh, mem, 0, 0, SRCCOPY);
            SelectObject(mem, old_bmp);
            DeleteObject(bmp);
            DeleteDC(mem);
            EndPaint(self.hwnd, &ps);
        }
    }

    /// First palette dot, in 96-DPI px.
    fn dots_start(&self) -> f32 {
        36.0
    }

    pub fn hit_test_header(&self, pt: POINT) -> HeaderHit {
        if !self.is_sticky() || pt.y >= self.header_px() {
            return HeaderHit::None;
        }
        // Same zones as always, expressed in 96-DPI px so they follow the window DPI.
        let k = self.dpi_scale();
        let x = pt.x as f32 / k;
        unsafe {
            let mut rc: RECT = std::mem::zeroed();
            GetClientRect(self.hwnd, &mut rc);
            let w = rc.right as f32 / k;
            if x >= w - 28.0 {
                return HeaderHit::Close;
            }
            if x >= w - 52.0 {
                return HeaderHit::Archive;
            }
            if x >= w - 76.0 {
                return HeaderHit::Pin;
            }
            if (4.0..=26.0).contains(&x) {
                return HeaderHit::New;
            }
            for i in 0..PALETTE_COUNT {
                let cx = self.dots_start() + i as f32 * 16.0;
                if (x - cx).abs() <= 7.0 {
                    return HeaderHit::Palette(i);
                }
            }
            HeaderHit::Drag
        }
    }

    /// Tracks the pointer over the header for hover states.
    pub fn on_mouse_move(&mut self, pt: POINT) {
        if !self.is_sticky() {
            return;
        }
        if !self.tracking {
            let mut tme = TRACKMOUSEEVENT {
                cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: self.hwnd,
                dwHoverTime: 0,
            };
            self.tracking = unsafe { TrackMouseEvent(&mut tme) } != 0;
        }
        let hover = match self.hit_test_header(pt) {
            HeaderHit::Drag | HeaderHit::None => HeaderHit::None,
            hit => hit,
        };
        if hover != self.hover {
            self.hover = hover;
            self.invalidate_header();
        }
    }

    pub fn on_mouse_leave(&mut self) {
        self.tracking = false;
        if self.hover != HeaderHit::None {
            self.hover = HeaderHit::None;
            self.invalidate_header();
        }
    }

    pub fn begin_drag(&self) {
        unsafe {
            SendMessageW(self.hwnd, WM_NCLBUTTONDOWN, HTCAPTION as WPARAM, 0);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderHit {
    None,
    New,
    Palette(usize),
    Pin,
    Archive,
    Close,
    Drag,
}

impl Drop for NoteWindow {
    fn drop(&mut self) {
        unsafe {
            if !self.font_handle.is_null() {
                DeleteObject(self.font_handle as *mut c_void);
            }
            if !self.bg_brush.is_null() {
                DeleteObject(self.bg_brush as *mut c_void);
            }
            if !self.ui_font.is_null() {
                DeleteObject(self.ui_font);
            }
            if !self.hwnd.is_null() {
                // Detach first so messages sent during destruction don't reach freed state.
                SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, 0);
                DestroyWindow(self.hwnd);
            }
        }
    }
}

pub fn load_richedit_dll() {
    let dll = to_wide_null("Msftedit.dll");
    unsafe {
        LoadLibraryW(dll.as_ptr());
    }
}

#[allow(clippy::too_many_arguments)]
pub fn create_note_window(
    mode: WinMode,
    title: &str,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    palette_idx: usize,
    dark: bool,
    pinned: bool,
    word_wrap: bool,
    font_family: String,
    font_size: f32,
    opacity: f32,
    initial_text: &str,
) -> Option<Box<NoteWindow>> {
    let is_sticky = matches!(mode, WinMode::Sticky { .. });
    let class_w = to_wide_null(NOTE_CLASS_NAME);
    let title_w = to_wide_null(title);

    let style = if is_sticky {
        WS_POPUP | WS_THICKFRAME | WS_CLIPCHILDREN | WS_VISIBLE
    } else {
        WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN | WS_VISIBLE
    };
    let mut ex_style = 0u32;
    if is_sticky {
        ex_style |= WS_EX_LAYERED;
    }
    if pinned {
        ex_style |= WS_EX_TOPMOST;
    }

    unsafe {
        let hinst = GetModuleHandleW(std::ptr::null());
        let hwnd = CreateWindowExW(
            ex_style,
            class_w.as_ptr(),
            title_w.as_ptr(),
            style,
            x,
            y,
            w,
            h,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            hinst,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            return None;
        }

        let edit_class = to_wide_null("RICHEDIT50W");
        let mut edit_style = WS_CHILD
            | WS_VISIBLE
            | WS_VSCROLL
            | ES_MULTILINE as u32
            | ES_AUTOVSCROLL as u32
            | ES_NOHIDESEL as u32;
        if !word_wrap {
            edit_style |= ES_AUTOHSCROLL as u32;
        }

        let edit_hwnd = CreateWindowExW(
            0,
            edit_class.as_ptr(),
            std::ptr::null(),
            edit_style,
            0,
            0,
            w,
            h,
            hwnd,
            std::ptr::null_mut(),
            hinst,
            std::ptr::null(),
        );

        SendMessageW(edit_hwnd, EM_SETEVENTMASK, 0, ENM_CHANGE as LPARAM);

        let mut note = Box::new(NoteWindow {
            hwnd,
            edit_hwnd,
            mode,
            palette_idx: palette_idx % PALETTE_COUNT,
            dark,
            pinned,
            word_wrap,
            dirty: false,
            font_family,
            font_size,
            zoom_num: 100,
            opacity,
            font_handle: std::ptr::null_mut(),
            bg_brush: std::ptr::null_mut(),
            ui_font: std::ptr::null_mut(),
            hover: HeaderHit::None,
            tracking: false,
            title: String::new(),
        });

        note.apply_font();
        note.apply_theme();
        note.apply_opacity();
        note.layout_children();
        note.set_text(initial_text);

        // Attach the window state only after initialisation, so the EN_CHANGE that RichEdit
        // fires for the initial text isn't treated as an edit.
        let ptr = (&mut *note as *mut NoteWindow) as isize;
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, ptr);

        Some(note)
    }
}

pub unsafe fn window_from_hwnd(hwnd: HWND) -> Option<&'static mut NoteWindow> {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut NoteWindow;
    if ptr.is_null() {
        None
    } else {
        Some(&mut *ptr)
    }
}

// Keep unused imports clean for future Find/Replace RichEdit selection integration
#[allow(dead_code)]
fn _keep_richedit_consts() -> (u32, u32, COLORREF, LRESULT) {
    (EM_EXGETSEL, EM_EXSETSEL, 0, 0)
}
