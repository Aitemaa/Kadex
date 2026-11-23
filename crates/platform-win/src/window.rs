//! Native Win32 sticky & file windows using MSFTEDIT_CLASS (RichEdit), 28px GDI header, DWM dark/rounded corners, and Per-Monitor DPI v2.

#![cfg(target_os = "windows")]

use std::ffi::c_void;
use std::path::PathBuf;

use kadex_core::config::{palette, Palette, PALETTE_COUNT};
use kadex_core::storage::{Encoding, LineEnding};

use windows_sys::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE,
    DWMWCP_ROUND,
};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreateSolidBrush, DeleteObject, Ellipse, EndPaint, FillRect,
    InvalidateRect, SelectObject, SetBkMode, SetTextColor, TextOutW, CLEARTYPE_QUALITY,
    DEFAULT_CHARSET, DEFAULT_PITCH, FF_DONTCARE, FW_NORMAL, HBRUSH, HFONT, OUT_DEFAULT_PRECIS,
    PAINTSTRUCT, TRANSPARENT,
};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, LoadLibraryW};
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;

const EM_SETMODIFY: u32 = 0x00B9;
const EM_SETBKGNDCOLOR: u32 = 0x0443;
const EM_SETCHARFORMAT: u32 = 0x0444;
const EM_SETEVENTMASK: u32 = 0x0445;
const EM_SETZOOM: u32 = 0x04E1;
const EM_EXGETSEL: u32 = 0x0434;
const EM_EXSETSEL: u32 = 0x0437;
const ENM_CHANGE: u32 = 0x0000_0001;
const CFM_COLOR: u32 = 0x4000_0000;
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
    GetWindowTextLengthW, GetWindowTextW, KillTimer, MoveWindow, SendMessageW,
    SetLayeredWindowAttributes, SetTimer, SetWindowLongPtrW, SetWindowPos, SetWindowTextW,
    ShowWindow, ES_AUTOHSCROLL, ES_AUTOVSCROLL, ES_MULTILINE, ES_NOHIDESEL,
    GWLP_USERDATA, HTCAPTION, HWND_NOTOPMOST, HWND_TOPMOST, LWA_ALPHA, SWP_NOMOVE, SWP_NOSIZE,
    SW_HIDE, SW_SHOW, WM_NCLBUTTONDOWN, WM_SETFONT, WS_CHILD, WS_CLIPCHILDREN, WS_EX_LAYERED,
    WS_EX_TOPMOST, WS_OVERLAPPEDWINDOW, WS_POPUP, WS_THICKFRAME, WS_VISIBLE, WS_VSCROLL,
};

use crate::theme::{rgb_to_colorref, to_wide_null};

pub const HEADER_H_96: i32 = 28;
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
                let mut cf: CHARFORMAT2W = std::mem::zeroed();
                cf.cbSize = std::mem::size_of::<CHARFORMAT2W>() as u32;
                cf.dwMask = CFM_COLOR;
                cf.crTextColor = rgb_to_colorref(pal.fg);
                SendMessageW(
                    self.edit_hwnd,
                    EM_SETCHARFORMAT,
                    SCF_ALL as WPARAM,
                    (&cf as *const CHARFORMAT2W) as LPARAM,
                );
            }
            InvalidateRect(self.hwnd, std::ptr::null(), 1);
        }
    }

    pub fn apply_font(&mut self) {
        let dpi = unsafe { GetDpiForWindow(self.hwnd) };
        let dpi = if dpi == 0 { 96 } else { dpi };
        let height = -((self.font_size * dpi as f32 / 72.0).round() as i32);
        let face = to_wide_null(&self.font_family);
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
        let insert_after = if self.pinned { HWND_TOPMOST } else { HWND_NOTOPMOST };
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
            let hh = self.header_px();
            let pal = self.palette();

            let header_rc = RECT {
                left: 0,
                top: 0,
                right: rc.right,
                bottom: hh,
            };
            let hbr = CreateSolidBrush(rgb_to_colorref(pal.header));
            FillRect(hdc, &header_rc, hbr);
            DeleteObject(hbr as *mut c_void);

            SetBkMode(hdc, TRANSPARENT as i32);
            SetTextColor(hdc, rgb_to_colorref(pal.fg));

            // "+" button
            let plus = to_wide_null("+");
            TextOutW(hdc, 10, 5, plus.as_ptr(), 1);

            // 6 Palette dots
            for i in 0..PALETTE_COUNT {
                let dp = palette(i, self.dark);
                let cx = 36 + (i as i32) * 16;
                let cy = hh / 2;
                let r = if i == self.palette_idx { 5 } else { 4 };
                let br = CreateSolidBrush(rgb_to_colorref(dp.accent));
                let old = SelectObject(hdc, br as *mut c_void);
                Ellipse(hdc, cx - r, cy - r, cx + r, cy + r);
                SelectObject(hdc, old);
                DeleteObject(br as *mut c_void);
            }

            // Right buttons: [P] Pin, [A] Archive, [X] Hide
            let pin_txt = if self.pinned { "*" } else { "o" };
            let pin_w = to_wide_null(pin_txt);
            TextOutW(hdc, rc.right - 68, 5, pin_w.as_ptr(), 1);
            let arch_w = to_wide_null("A");
            TextOutW(hdc, rc.right - 46, 5, arch_w.as_ptr(), 1);
            let close_w = to_wide_null("x");
            TextOutW(hdc, rc.right - 22, 5, close_w.as_ptr(), 1);

            EndPaint(self.hwnd, &ps);
        }
    }

    pub fn hit_test_header(&self, pt: POINT) -> HeaderHit {
        if !self.is_sticky() || pt.y >= self.header_px() {
            return HeaderHit::None;
        }
        unsafe {
            let mut rc: RECT = std::mem::zeroed();
            GetClientRect(self.hwnd, &mut rc);
            let w = rc.right;
            if pt.x >= w - 28 {
                return HeaderHit::Close;
            }
            if pt.x >= w - 52 {
                return HeaderHit::Archive;
            }
            if pt.x >= w - 76 {
                return HeaderHit::Pin;
            }
            if pt.x >= 4 && pt.x <= 26 {
                return HeaderHit::New;
            }
            for i in 0..PALETTE_COUNT {
                let cx = 36 + (i as i32) * 16;
                if (pt.x - cx).abs() <= 7 {
                    return HeaderHit::Palette(i);
                }
            }
            HeaderHit::Drag
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
            if !self.hwnd.is_null() {
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
        let mut edit_style = WS_CHILD | WS_VISIBLE | WS_VSCROLL | ES_MULTILINE as u32
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
        });

        let ptr = (&mut *note as *mut NoteWindow) as isize;
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, ptr);

        note.apply_font();
        note.apply_theme();
        note.apply_opacity();
        note.layout_children();
        note.set_text(initial_text);

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
