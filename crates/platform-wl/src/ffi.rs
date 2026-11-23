#![allow(dead_code, non_camel_case_types)]

use std::ffi::{c_char, c_double, c_int, c_void};

#[repr(C)]
pub struct KdxApp {
    _opaque: [u8; 0],
}

#[repr(C)]
pub struct KdxWin {
    _opaque: [u8; 0],
}

#[repr(C)]
pub struct KdxLayout {
    _opaque: [u8; 0],
}

pub const KDX_WIN_XDG: c_int = 0;
pub const KDX_WIN_LAYER_OVERLAY: c_int = 1;

pub const KDX_MOD_SHIFT: u32 = 1 << 0;
pub const KDX_MOD_CTRL: u32 = 1 << 1;
pub const KDX_MOD_ALT: u32 = 1 << 2;
pub const KDX_MOD_SUPER: u32 = 1 << 3;

pub const KDX_CURSOR_DEFAULT: c_int = 0;
pub const KDX_CURSOR_TEXT: c_int = 1;
pub const KDX_CURSOR_POINTER: c_int = 2;

// Linux input button codes (linux/input-event-codes.h)
pub const BTN_LEFT: u32 = 0x110;
pub const BTN_RIGHT: u32 = 0x111;
pub const BTN_MIDDLE: u32 = 0x112;

// Common XKB keysyms
pub const XKB_KEY_BACKSPACE: u32 = 0xff08;
pub const XKB_KEY_TAB: u32 = 0xff09;
pub const XKB_KEY_RETURN: u32 = 0xff0d;
pub const XKB_KEY_ESCAPE: u32 = 0xff1b;
pub const XKB_KEY_DELETE: u32 = 0xffff;
pub const XKB_KEY_HOME: u32 = 0xff50;
pub const XKB_KEY_LEFT: u32 = 0xff51;
pub const XKB_KEY_UP: u32 = 0xff52;
pub const XKB_KEY_RIGHT: u32 = 0xff53;
pub const XKB_KEY_DOWN: u32 = 0xff54;
pub const XKB_KEY_PAGE_UP: u32 = 0xff55;
pub const XKB_KEY_PAGE_DOWN: u32 = 0xff56;
pub const XKB_KEY_END: u32 = 0xff57;
pub const XKB_KEY_KP_ENTER: u32 = 0xff8d;
pub const XKB_KEY_F3: u32 = 0xffc0;

#[repr(C)]
pub struct KdxCallbacks {
    pub userdata: *mut c_void,
    pub on_configure: Option<extern "C" fn(*mut c_void, u64, i32, i32, u32)>,
    pub on_close: Option<extern "C" fn(*mut c_void, u64)>,
    pub on_focus: Option<extern "C" fn(*mut c_void, u64, bool)>,
    pub on_key: Option<extern "C" fn(*mut c_void, u64, u32, u32, *const c_char, usize, bool)>,
    pub on_ime_commit: Option<extern "C" fn(*mut c_void, u64, *const c_char, usize)>,
    pub on_pointer_motion: Option<extern "C" fn(*mut c_void, u64, c_double, c_double)>,
    pub on_pointer_button:
        Option<extern "C" fn(*mut c_void, u64, u32, bool, u32, c_double, c_double)>,
    pub on_pointer_scroll: Option<extern "C" fn(*mut c_void, u64, c_double, c_double, u32)>,
    pub on_repeat_info: Option<extern "C" fn(*mut c_void, i32, i32)>,
}

#[repr(C)]
pub struct KdxCanvas {
    pub cr: *mut c_void,
    pub width: i32,
    pub height: i32,
    pub scale_120: u32,
}

extern "C" {
    pub fn kdx_app_new(cbs: *const KdxCallbacks) -> *mut KdxApp;
    pub fn kdx_app_destroy(app: *mut KdxApp);
    pub fn kdx_app_get_fd(app: *mut KdxApp) -> c_int;
    pub fn kdx_app_prepare_read(app: *mut KdxApp) -> c_int;
    pub fn kdx_app_read_events(app: *mut KdxApp) -> c_int;
    pub fn kdx_app_cancel_read(app: *mut KdxApp);
    pub fn kdx_app_dispatch_pending(app: *mut KdxApp) -> c_int;
    pub fn kdx_app_flush(app: *mut KdxApp) -> c_int;
    pub fn kdx_app_has_layer_shell(app: *mut KdxApp) -> bool;

    pub fn kdx_win_create(
        app: *mut KdxApp,
        win_id: u64,
        kind: c_int,
        app_id: *const c_char,
        title: *const c_char,
        width: i32,
        height: i32,
    ) -> *mut KdxWin;
    pub fn kdx_win_destroy(win: *mut KdxWin);
    pub fn kdx_win_set_title(win: *mut KdxWin, title: *const c_char);
    pub fn kdx_win_start_move(win: *mut KdxWin);
    pub fn kdx_win_set_cursor(app: *mut KdxApp, cursor_kind: c_int);

    pub fn kdx_clipboard_set(app: *mut KdxApp, utf8: *const c_char, len: usize, primary: bool);
    pub fn kdx_clipboard_get(app: *mut KdxApp, primary: bool, out_len: *mut usize) -> *mut c_char;
    pub fn kdx_free_bytes(ptr: *mut c_char);

    pub fn kdx_win_begin_paint(win: *mut KdxWin, out_canvas: *mut KdxCanvas) -> bool;
    pub fn kdx_win_end_paint(win: *mut KdxWin, canvas: *mut KdxCanvas);

    pub fn kdx_draw_rect(
        cr: *mut c_void,
        x: c_double,
        y: c_double,
        w: c_double,
        h: c_double,
        rgb: u32,
        alpha: c_double,
    );
    pub fn kdx_draw_round_rect(
        cr: *mut c_void,
        x: c_double,
        y: c_double,
        w: c_double,
        h: c_double,
        r: c_double,
        rgb: u32,
        alpha: c_double,
    );
    pub fn kdx_draw_circle(
        cr: *mut c_void,
        cx: c_double,
        cy: c_double,
        r: c_double,
        rgb: u32,
        alpha: c_double,
    );
    pub fn kdx_draw_line(
        cr: *mut c_void,
        x1: c_double,
        y1: c_double,
        x2: c_double,
        y2: c_double,
        lw: c_double,
        rgb: u32,
        alpha: c_double,
    );
    pub fn kdx_clip_rect(cr: *mut c_void, x: c_double, y: c_double, w: c_double, h: c_double);
    pub fn kdx_save(cr: *mut c_void);
    pub fn kdx_restore(cr: *mut c_void);
    pub fn kdx_translate(cr: *mut c_void, tx: c_double, ty: c_double);
    pub fn kdx_scale(cr: *mut c_void, sx: c_double, sy: c_double);
    pub fn kdx_stroke_round_rect(
        cr: *mut c_void,
        x: c_double,
        y: c_double,
        w: c_double,
        h: c_double,
        r: c_double,
        lw: c_double,
        rgb: u32,
        alpha: c_double,
    );
    pub fn kdx_clip_round_rect(
        cr: *mut c_void,
        x: c_double,
        y: c_double,
        w: c_double,
        h: c_double,
        r: c_double,
    );
    pub fn kdx_stroke_circle(
        cr: *mut c_void,
        cx: c_double,
        cy: c_double,
        r: c_double,
        lw: c_double,
        rgb: u32,
        alpha: c_double,
    );
    pub fn kdx_path_begin(cr: *mut c_void);
    pub fn kdx_path_move(cr: *mut c_void, x: c_double, y: c_double);
    pub fn kdx_path_line(cr: *mut c_void, x: c_double, y: c_double);
    pub fn kdx_path_curve(
        cr: *mut c_void,
        x1: c_double,
        y1: c_double,
        x2: c_double,
        y2: c_double,
        x3: c_double,
        y3: c_double,
    );
    pub fn kdx_path_close(cr: *mut c_void);
    pub fn kdx_path_stroke(cr: *mut c_void, lw: c_double, rgb: u32, alpha: c_double);
    pub fn kdx_font_register(path: *const c_char) -> bool;

    pub fn kdx_layout_new(
        cr: *mut c_void,
        font_family: *const c_char,
        font_size_pt: c_double,
        text: *const c_char,
        text_len: usize,
        wrap_width_px: i32,
    ) -> *mut KdxLayout;
    pub fn kdx_layout_free(layout: *mut KdxLayout);
    pub fn kdx_layout_set_subpixel(layout: *mut KdxLayout);
    pub fn kdx_layout_get_size(layout: *mut KdxLayout, out_w: *mut c_double, out_h: *mut c_double);
    pub fn kdx_layout_draw(
        cr: *mut c_void,
        layout: *mut KdxLayout,
        x: c_double,
        y: c_double,
        rgb: u32,
        alpha: c_double,
    );
    pub fn kdx_layout_index_to_pos(
        layout: *mut KdxLayout,
        byte_idx: c_int,
        out_x: *mut c_double,
        out_y: *mut c_double,
        out_h: *mut c_double,
    );
    pub fn kdx_layout_xy_to_index(layout: *mut KdxLayout, x: c_double, y: c_double) -> c_int;
    pub fn kdx_layout_draw_selection(
        cr: *mut c_void,
        layout: *mut KdxLayout,
        ox: c_double,
        oy: c_double,
        start_byte: c_int,
        end_byte: c_int,
        rgb: u32,
        alpha: c_double,
    );
    pub fn kdx_hit_test(
        font_family: *const c_char,
        font_size_pt: c_double,
        text: *const c_char,
        text_len: usize,
        wrap_width_px: i32,
        x: c_double,
        y: c_double,
    ) -> c_int;
    pub fn kdx_measure_line_height(font_family: *const c_char, font_size_pt: c_double) -> c_double;
}

// Minimal POSIX externs (poll + timerfd + signal) — zero libc crate needed
#[repr(C)]
#[derive(Clone, Copy)]
pub struct PollFd {
    pub fd: c_int,
    pub events: i16,
    pub revents: i16,
}

pub const POLLIN: i16 = 0x0001;
pub const CLOCK_MONOTONIC: c_int = 1;
pub const TFD_NONBLOCK: c_int = 0o4000;
pub const TFD_CLOEXEC: c_int = 0o2000000;
pub const SIGINT: c_int = 2;
pub const SIGTERM: c_int = 15;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TimeSpec {
    pub tv_sec: i64,
    pub tv_nsec: i64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ITimerSpec {
    pub it_interval: TimeSpec,
    pub it_value: TimeSpec,
}

extern "C" {
    pub fn poll(fds: *mut PollFd, nfds: u64, timeout: c_int) -> c_int;
    pub fn timerfd_create(clockid: c_int, flags: c_int) -> c_int;
    pub fn timerfd_settime(
        fd: c_int,
        flags: c_int,
        new_value: *const ITimerSpec,
        old_value: *mut ITimerSpec,
    ) -> c_int;
    pub fn read(fd: c_int, buf: *mut c_void, count: usize) -> isize;
    pub fn close(fd: c_int) -> c_int;
    pub fn signal(sig: c_int, handler: extern "C" fn(c_int)) -> usize;
}
