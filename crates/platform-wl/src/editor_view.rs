//! Window & editor view state, input handling, and Cairo/Pango rendering for Wayland.

use std::ffi::{c_char, CString};
use std::path::PathBuf;
use std::time::Instant;

use kadex_core::buffer::GapBuffer;
use kadex_core::config::{palette, Palette, PALETTE_COUNT, RED};
use kadex_core::icons::{Icon, Seg, GRID, STROKE};
use kadex_core::search;
use kadex_core::storage::{Encoding, LineEnding};

use crate::ffi::*;
use crate::fonts::UI_FAMILY;

pub const HEADER_H: f64 = 28.0;
pub const STATUS_H: f64 = 22.0;
pub const PAD_X: f64 = 12.0;
pub const PAD_Y: f64 = 10.0;
const LARGE_FILE_THRESHOLD: usize = 32 * 1024;

// Kavox family geometry (logical px).
/// Icon button (24 x 24, radius 7) inside the 28 px header; the glyph is 16 px (stroke 2 / 24 grid).
const BTN: f64 = 24.0;
const BTN_RADIUS: f64 = 7.0;
const ICON_PX: f64 = 16.0;
/// Overlay panel radius (Kavox `RADIUS_PANEL`).
const RADIUS_PANEL: f64 = 20.0;
/// Transparent margin around the overlay panel that holds its drop shadow.
pub const OVERLAY_INSET: i32 = 32;
/// Header chrome text (Archivo Medium) in pt; 9 pt = 12 px.
const UI_PT: f64 = 9.0;
const STATUS_PT: f64 = 8.5;
/// Alpha (0-255) of the red hover tint behind the danger button (Kavox: 30).
const DANGER_TINT: f64 = 30.0 / 255.0;

/// A clickable control in the header bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HdrBtn {
    New,
    Dot(usize),
    Archive,
    Close,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WindowMode {
    Sticky {
        note_id: String,
    },
    File {
        path: PathBuf,
        encoding: Encoding,
        line_ending: LineEnding,
    },
    Overlay {
        note_id: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindFocus {
    None,
    Find,
    Replace,
}

#[derive(Debug, Clone)]
pub enum ViewAction {
    None,
    NewSticky,
    ArchiveSticky(String),
    CloseOrHide,
    SaveNow,
    PaletteChanged(usize),
    StartDrag,
}

pub struct EditorWindow {
    pub id: u64,
    pub mode: WindowMode,
    pub handle: *mut KdxWin,
    pub buf: GapBuffer,
    pub width: i32,
    pub height: i32,
    pub scale_120: u32,
    pub palette_idx: usize,
    pub dark: bool,
    pub font_family: String,
    pub font_size: f32,
    pub default_font_size: f32,
    pub word_wrap: bool,
    pub opacity: f32,

    pub focused: bool,
    pub visible: bool,
    pub needs_redraw: bool,
    pub caret_visible: bool,
    pub autosave_due: Option<Instant>,

    pub scroll_y: f64,
    pub content_height: f64,
    pub line_height: f64,
    pub dragging_text: bool,
    pub hover_x: f64,
    pub hover_y: f64,
    /// Header control under the pointer (hover state).
    pub hover: Option<HdrBtn>,
    /// Sticky header title (first line of the note), cached per buffer version.
    title_cache: Option<(u64, String)>,
    pub last_click_time: Option<Instant>,
    pub click_streak: u8,

    // Find & Replace bar state
    pub find_open: bool,
    pub replace_open: bool,
    pub find_focus: FindFocus,
    pub find_query: String,
    pub replace_query: String,
    pub case_sensitive: bool,
    pub match_count: usize,
    pub current_match: usize,

    // Windowed slice offset for huge (>32 KB) files
    pub view_slice_start: usize,
    pub view_slice_end: usize,
    pub total_lines_cached: usize,
    pub cached_version: u64,
    /// Set when the caret moved and should be scrolled into view on the next paint.
    pub follow_caret: bool,
}

impl EditorWindow {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: u64,
        mode: WindowMode,
        handle: *mut KdxWin,
        initial_text: &str,
        width: i32,
        height: i32,
        palette_idx: usize,
        dark: bool,
        font_family: String,
        font_size: f32,
        word_wrap: bool,
        opacity: f32,
    ) -> Self {
        let mut buf = GapBuffer::from_text(initial_text);
        if matches!(mode, WindowMode::Sticky { .. } | WindowMode::Overlay { .. }) {
            let end = buf.len();
            buf.set_cursor(end, false);
        }
        let c_font = CString::new(font_family.as_str())
            .unwrap_or_else(|_| CString::new("monospace").unwrap());
        let line_height = unsafe { kdx_measure_line_height(c_font.as_ptr(), font_size as f64) };
        let total_lines = initial_text.bytes().filter(|&b| b == b'\n').count() + 1;
        Self {
            id,
            mode,
            handle,
            buf,
            width,
            height,
            scale_120: 120,
            palette_idx: palette_idx % PALETTE_COUNT,
            dark,
            font_family,
            font_size,
            default_font_size: font_size,
            word_wrap,
            opacity,
            focused: false,
            visible: true,
            needs_redraw: true,
            caret_visible: true,
            autosave_due: None,
            scroll_y: 0.0,
            content_height: 0.0,
            line_height,
            dragging_text: false,
            hover_x: 0.0,
            hover_y: 0.0,
            hover: None,
            title_cache: None,
            last_click_time: None,
            click_streak: 0,
            find_open: false,
            replace_open: false,
            find_focus: FindFocus::None,
            find_query: String::new(),
            replace_query: String::new(),
            case_sensitive: false,
            match_count: 0,
            current_match: 0,
            view_slice_start: 0,
            view_slice_end: 0,
            total_lines_cached: total_lines,
            cached_version: u64::MAX,
            follow_caret: true,
        }
    }

    pub fn is_sticky(&self) -> bool {
        matches!(self.mode, WindowMode::Sticky { .. })
    }

    pub fn is_overlay(&self) -> bool {
        matches!(self.mode, WindowMode::Overlay { .. })
    }

    pub fn has_header(&self) -> bool {
        matches!(
            self.mode,
            WindowMode::Sticky { .. } | WindowMode::Overlay { .. }
        )
    }

    pub fn has_status_bar(&self) -> bool {
        matches!(self.mode, WindowMode::File { .. })
    }

    pub fn header_height(&self) -> f64 {
        if self.has_header() {
            HEADER_H
        } else {
            0.0
        }
    }

    pub fn find_bar_height(&self) -> f64 {
        if !self.find_open {
            0.0
        } else if self.replace_open {
            62.0
        } else {
            32.0
        }
    }

    pub fn status_height(&self) -> f64 {
        if self.has_status_bar() {
            STATUS_H
        } else {
            0.0
        }
    }

    pub fn text_top(&self) -> f64 {
        self.header_height() + self.find_bar_height()
    }

    /// Margin between the surface edge and the visible panel (overlay drop shadow only).
    fn inset(&self) -> f64 {
        if self.is_overlay() {
            OVERLAY_INSET as f64
        } else {
            0.0
        }
    }

    /// Size of the visible panel in logical px (the surface minus the shadow margin).
    fn view_size(&self) -> (f64, f64) {
        let i = self.inset() * 2.0;
        (self.width as f64 - i, self.height as f64 - i)
    }

    /// Side margin of the header controls: the overlay's rounded corners need more room.
    fn header_pad(&self) -> f64 {
        if self.is_overlay() {
            10.0
        } else {
            0.0
        }
    }

    fn dots_start(&self) -> f64 {
        if self.is_sticky() {
            36.0
        } else {
            14.0 + self.header_pad()
        }
    }

    /// Hit-testing of the header controls, in panel coordinates.
    fn header_hit(&self, x: f64, y: f64) -> Option<HdrBtn> {
        if !self.has_header() || !(0.0..HEADER_H).contains(&y) {
            return None;
        }
        let (w, _) = self.view_size();
        let pad = self.header_pad();
        if x >= w - 28.0 - pad {
            return Some(HdrBtn::Close);
        }
        if self.is_sticky() {
            if x >= w - 52.0 - pad {
                return Some(HdrBtn::Archive);
            }
            if (4.0..=28.0).contains(&x) {
                return Some(HdrBtn::New);
            }
        }
        let start = self.dots_start();
        (0..PALETTE_COUNT)
            .find(|&i| (x - (start + i as f64 * 16.0)).abs() <= 7.0)
            .map(HdrBtn::Dot)
    }

    pub fn text_viewport_height(&self) -> f64 {
        (self.view_size().1 - self.text_top() - self.status_height()).max(20.0)
    }

    pub fn text_wrap_width(&self) -> i32 {
        if self.word_wrap {
            ((self.view_size().0 - PAD_X * 2.0 - 8.0).max(80.0)) as i32
        } else {
            -1
        }
    }

    pub fn palette(&self) -> Palette {
        palette(self.palette_idx, self.dark)
    }

    pub fn window_title(&self) -> String {
        match &self.mode {
            WindowMode::Sticky { .. } => {
                let first = self.buf.first_non_empty_line();
                if first == "(empty note)" {
                    String::from("Kadex Note")
                } else {
                    format!("{first} — Kadex")
                }
            }
            WindowMode::Overlay { .. } => String::from("Kadex Scratchpad"),
            WindowMode::File { path, .. } => {
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("Untitled");
                if self.buf.is_dirty() {
                    format!("*{name} — Kadex")
                } else {
                    format!("{name} — Kadex")
                }
            }
        }
    }

    pub fn update_title(&self) {
        if self.handle.is_null() {
            return;
        }
        if let Ok(c) = CString::new(self.window_title()) {
            unsafe { kdx_win_set_title(self.handle, c.as_ptr()) };
        }
    }

    pub fn schedule_autosave(&mut self, delay_ms: u64) {
        if matches!(
            self.mode,
            WindowMode::Sticky { .. } | WindowMode::Overlay { .. }
        ) {
            self.autosave_due = Some(Instant::now() + std::time::Duration::from_millis(delay_ms));
        }
    }

    fn refresh_line_height(&mut self) {
        let c_font = CString::new(self.font_family.as_str())
            .unwrap_or_else(|_| CString::new("monospace").unwrap());
        self.line_height =
            unsafe { kdx_measure_line_height(c_font.as_ptr(), self.font_size as f64) };
    }

    pub fn zoom(&mut self, delta: f32) {
        self.font_size = (self.font_size + delta).clamp(7.0, 40.0);
        self.refresh_line_height();
        self.needs_redraw = true;
    }

    pub fn zoom_reset(&mut self) {
        self.font_size = self.default_font_size;
        self.refresh_line_height();
        self.needs_redraw = true;
    }

    fn sync_primary_selection(&self, app: *mut KdxApp) {
        if let Some(sel) = self.buf.selected_text() {
            if !sel.is_empty() {
                unsafe {
                    kdx_clipboard_set(app, sel.as_ptr() as *const c_char, sel.len(), true);
                }
            }
        }
    }

    fn copy_to_clipboard(&self, app: *mut KdxApp) {
        if let Some(sel) = self.buf.selected_text() {
            unsafe {
                kdx_clipboard_set(app, sel.as_ptr() as *const c_char, sel.len(), false);
            }
        }
    }

    fn paste_from_clipboard(&mut self, app: *mut KdxApp, primary: bool, autosave_ms: u64) {
        let mut len: usize = 0;
        let ptr = unsafe { kdx_clipboard_get(app, primary, &mut len) };
        if ptr.is_null() {
            return;
        }
        if len > 0 {
            let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, len) };
            let text = String::from_utf8_lossy(bytes)
                .replace("\r\n", "\n")
                .replace('\r', "\n");
            if self.find_open && self.find_focus == FindFocus::Find {
                self.find_query.push_str(&text.replace('\n', " "));
                self.refresh_find_matches(true);
            } else if self.find_open && self.find_focus == FindFocus::Replace {
                self.replace_query.push_str(&text.replace('\n', " "));
            } else {
                self.buf.insert_str(&text);
                self.schedule_autosave(autosave_ms);
                self.update_title();
                self.ensure_cursor_visible();
            }
            self.needs_redraw = true;
        }
        unsafe { kdx_free_bytes(ptr) };
    }

    fn refresh_find_matches(&mut self, jump_to_next: bool) {
        if self.find_query.is_empty() {
            self.match_count = 0;
            self.current_match = 0;
            return;
        }
        let text = self.buf.to_string_lossy();
        let all = search::find_all(&text, &self.find_query, self.case_sensitive);
        self.match_count = all.len();
        if all.is_empty() {
            self.current_match = 0;
            return;
        }
        if jump_to_next {
            let from = self
                .buf
                .selection()
                .map(|(s, _)| s)
                .unwrap_or(self.buf.cursor());
            if let Some(m) = search::find_next(&text, &self.find_query, from, self.case_sensitive) {
                self.buf.select_range(m.start, m.end);
                self.current_match = all
                    .iter()
                    .position(|x| x.start == m.start)
                    .map(|i| i + 1)
                    .unwrap_or(1);
                self.ensure_cursor_visible();
            }
        }
    }

    fn find_step(&mut self, forward: bool) {
        if self.find_query.is_empty() {
            return;
        }
        let text = self.buf.to_string_lossy();
        let all = search::find_all(&text, &self.find_query, self.case_sensitive);
        self.match_count = all.len();
        if all.is_empty() {
            self.current_match = 0;
            return;
        }
        let found = if forward {
            let from = self
                .buf
                .selection()
                .map(|(_, e)| e)
                .unwrap_or(self.buf.cursor());
            search::find_next(&text, &self.find_query, from, self.case_sensitive)
        } else {
            let before = self
                .buf
                .selection()
                .map(|(s, _)| s)
                .unwrap_or(self.buf.cursor());
            search::find_prev(&text, &self.find_query, before, self.case_sensitive)
        };
        if let Some(m) = found {
            self.buf.select_range(m.start, m.end);
            self.current_match = all
                .iter()
                .position(|x| x.start == m.start)
                .map(|i| i + 1)
                .unwrap_or(1);
            self.ensure_cursor_visible();
            self.needs_redraw = true;
        }
    }

    fn replace_current(&mut self, autosave_ms: u64) {
        if self.find_query.is_empty() {
            return;
        }
        if let Some((s, e)) = self.buf.selection() {
            let selected = self.buf.slice_string(s, e);
            let matches = if self.case_sensitive {
                selected == self.find_query
            } else {
                selected.to_lowercase() == self.find_query.to_lowercase()
            };
            if matches {
                let rep = self.replace_query.clone();
                self.buf.insert_str(&rep);
                self.schedule_autosave(autosave_ms);
                self.update_title();
            }
        }
        self.find_step(true);
        self.needs_redraw = true;
    }

    fn replace_all_matches(&mut self, autosave_ms: u64) {
        if self.find_query.is_empty() {
            return;
        }
        let text = self.buf.to_string_lossy();
        let (replaced, count) = search::replace_all(
            &text,
            &self.find_query,
            &self.replace_query,
            self.case_sensitive,
        );
        if count > 0 {
            self.buf.replace_all_text(&replaced);
            self.schedule_autosave(autosave_ms);
            self.update_title();
            self.match_count = 0;
            self.current_match = 0;
            self.needs_redraw = true;
        }
    }

    pub fn handle_ime_commit(&mut self, text: &str, autosave_ms: u64) {
        if text.is_empty() {
            return;
        }
        if self.find_open && self.find_focus == FindFocus::Find {
            self.find_query.push_str(text);
            self.refresh_find_matches(true);
        } else if self.find_open && self.find_focus == FindFocus::Replace {
            self.replace_query.push_str(text);
        } else {
            self.buf.insert_str(text);
            self.schedule_autosave(autosave_ms);
            self.update_title();
            self.ensure_cursor_visible();
        }
        self.caret_visible = true;
        self.needs_redraw = true;
    }

    pub fn handle_key(
        &mut self,
        app: *mut KdxApp,
        keysym: u32,
        mods: u32,
        utf8: &str,
        autosave_ms: u64,
    ) -> ViewAction {
        let ctrl = (mods & KDX_MOD_CTRL) != 0;
        let shift = (mods & KDX_MOD_SHIFT) != 0;
        let alt = (mods & KDX_MOD_ALT) != 0;

        self.caret_visible = true;

        // Global shortcuts
        if ctrl && !alt {
            if keysym == XKB_KEY_RETURN || keysym == XKB_KEY_KP_ENTER {
                if self.find_open && self.replace_open {
                    if shift {
                        self.replace_all_matches(autosave_ms);
                    } else {
                        self.replace_current(autosave_ms);
                    }
                    return ViewAction::None;
                }
                if self.buf.toggle_checkbox_at_cursor() {
                    self.schedule_autosave(autosave_ms);
                    self.needs_redraw = true;
                    return ViewAction::None;
                }
            } else if keysym < 128 {
                match keysym as u8 {
                    b'n' | b'N' => return ViewAction::NewSticky,
                    b'w' | b'W' => return ViewAction::CloseOrHide,
                    b's' | b'S' => return ViewAction::SaveNow,
                    b'f' | b'F' => {
                        self.find_open = true;
                        self.replace_open = false;
                        self.find_focus = FindFocus::Find;
                        if let Some(sel) = self.buf.selected_text() {
                            if !sel.contains('\n') && sel.len() < 128 {
                                self.find_query = sel;
                            }
                        }
                        self.refresh_find_matches(false);
                        self.needs_redraw = true;
                        return ViewAction::None;
                    }
                    b'h' | b'H' => {
                        self.find_open = true;
                        self.replace_open = true;
                        self.find_focus = FindFocus::Find;
                        self.refresh_find_matches(false);
                        self.needs_redraw = true;
                        return ViewAction::None;
                    }
                    b'=' | b'+' => {
                        self.zoom(1.0);
                        return ViewAction::None;
                    }
                    b'-' | b'_' => {
                        self.zoom(-1.0);
                        return ViewAction::None;
                    }
                    b'0' => {
                        self.zoom_reset();
                        return ViewAction::None;
                    }
                    b'c' | b'C' => {
                        self.copy_to_clipboard(app);
                        return ViewAction::None;
                    }
                    b'x' | b'X' => {
                        if self.find_focus == FindFocus::None {
                            self.copy_to_clipboard(app);
                            if self.buf.delete_selection().is_some() {
                                self.schedule_autosave(autosave_ms);
                                self.update_title();
                                self.needs_redraw = true;
                            }
                        }
                        return ViewAction::None;
                    }
                    b'v' | b'V' => {
                        self.paste_from_clipboard(app, false, autosave_ms);
                        return ViewAction::None;
                    }
                    b'a' | b'A' => {
                        if self.find_focus == FindFocus::None {
                            self.buf.select_all();
                            self.sync_primary_selection(app);
                            self.needs_redraw = true;
                        }
                        return ViewAction::None;
                    }
                    b'z' | b'Z' => {
                        if self.find_focus == FindFocus::None {
                            let changed = if shift {
                                self.buf.redo()
                            } else {
                                self.buf.undo()
                            };
                            if changed {
                                self.schedule_autosave(autosave_ms);
                                self.update_title();
                                self.ensure_cursor_visible();
                                self.needs_redraw = true;
                            }
                        }
                        return ViewAction::None;
                    }
                    b'y' | b'Y' => {
                        if self.find_focus == FindFocus::None && self.buf.redo() {
                            self.schedule_autosave(autosave_ms);
                            self.update_title();
                            self.ensure_cursor_visible();
                            self.needs_redraw = true;
                        }
                        return ViewAction::None;
                    }
                    _ => {}
                }
            }
        }

        if alt && !ctrl && (keysym == b'z' as u32 || keysym == b'Z' as u32) {
            self.word_wrap = !self.word_wrap;
            self.needs_redraw = true;
            return ViewAction::None;
        }

        if keysym == XKB_KEY_F3 {
            self.find_step(!shift);
            return ViewAction::None;
        }

        if keysym == XKB_KEY_ESCAPE {
            if self.find_open {
                self.find_open = false;
                self.replace_open = false;
                self.find_focus = FindFocus::None;
                self.needs_redraw = true;
                return ViewAction::None;
            }
            if self.is_overlay() {
                return ViewAction::CloseOrHide;
            }
            self.buf.clear_selection();
            self.needs_redraw = true;
            return ViewAction::None;
        }

        // Handle Find / Replace bar input when focused
        if self.find_open && self.find_focus != FindFocus::None {
            match keysym {
                XKB_KEY_TAB => {
                    if self.replace_open {
                        self.find_focus = if self.find_focus == FindFocus::Find {
                            FindFocus::Replace
                        } else {
                            FindFocus::Find
                        };
                        self.needs_redraw = true;
                    }
                    return ViewAction::None;
                }
                XKB_KEY_RETURN | XKB_KEY_KP_ENTER => {
                    if self.find_focus == FindFocus::Replace {
                        self.replace_current(autosave_ms);
                    } else {
                        self.find_step(!shift);
                    }
                    return ViewAction::None;
                }
                XKB_KEY_BACKSPACE => {
                    if self.find_focus == FindFocus::Find {
                        self.find_query.pop();
                        self.refresh_find_matches(true);
                    } else {
                        self.replace_query.pop();
                    }
                    self.needs_redraw = true;
                    return ViewAction::None;
                }
                _ => {
                    if !ctrl && !alt && !utf8.is_empty() && utf8.chars().all(|c| !c.is_control()) {
                        if self.find_focus == FindFocus::Find {
                            self.find_query.push_str(utf8);
                            self.refresh_find_matches(true);
                        } else {
                            self.replace_query.push_str(utf8);
                        }
                        self.needs_redraw = true;
                    }
                    return ViewAction::None;
                }
            }
        }

        // Main editor keys
        match keysym {
            XKB_KEY_BACKSPACE => {
                if ctrl {
                    self.buf.backspace_word();
                } else {
                    self.buf.backspace();
                }
                self.schedule_autosave(autosave_ms);
                self.update_title();
            }
            XKB_KEY_DELETE => {
                if ctrl {
                    self.buf.delete_word_forward();
                } else {
                    self.buf.delete_forward();
                }
                self.schedule_autosave(autosave_ms);
                self.update_title();
            }
            XKB_KEY_RETURN | XKB_KEY_KP_ENTER => {
                self.buf.insert_str("\n");
                self.schedule_autosave(autosave_ms);
                self.update_title();
            }
            XKB_KEY_TAB => {
                self.buf.insert_str("    ");
                self.schedule_autosave(autosave_ms);
                self.update_title();
            }
            XKB_KEY_LEFT => {
                if ctrl {
                    self.buf.move_word_left(shift);
                } else {
                    self.buf.move_left(shift);
                }
                if shift {
                    self.sync_primary_selection(app);
                }
            }
            XKB_KEY_RIGHT => {
                if ctrl {
                    self.buf.move_word_right(shift);
                } else {
                    self.buf.move_right(shift);
                }
                if shift {
                    self.sync_primary_selection(app);
                }
            }
            XKB_KEY_UP => {
                self.buf.move_up(shift);
                if shift {
                    self.sync_primary_selection(app);
                }
            }
            XKB_KEY_DOWN => {
                self.buf.move_down(shift);
                if shift {
                    self.sync_primary_selection(app);
                }
            }
            XKB_KEY_HOME => {
                if ctrl {
                    self.buf.set_cursor(0, shift);
                } else {
                    self.buf.move_line_start(shift);
                }
                if shift {
                    self.sync_primary_selection(app);
                }
            }
            XKB_KEY_END => {
                if ctrl {
                    let end = self.buf.len();
                    self.buf.set_cursor(end, shift);
                } else {
                    self.buf.move_line_end(shift);
                }
                if shift {
                    self.sync_primary_selection(app);
                }
            }
            XKB_KEY_PAGE_UP => {
                let lines = ((self.text_viewport_height() / self.line_height) as usize).max(1);
                for _ in 0..lines {
                    self.buf.move_up(shift);
                }
            }
            XKB_KEY_PAGE_DOWN => {
                let lines = ((self.text_viewport_height() / self.line_height) as usize).max(1);
                for _ in 0..lines {
                    self.buf.move_down(shift);
                }
            }
            _ => {
                if !ctrl && !alt && !utf8.is_empty() && utf8.chars().all(|c| !c.is_control()) {
                    self.buf.insert_str(utf8);
                    self.schedule_autosave(autosave_ms);
                    self.update_title();
                } else {
                    return ViewAction::None;
                }
            }
        }

        self.ensure_cursor_visible();
        self.needs_redraw = true;
        ViewAction::None
    }

    pub fn handle_pointer_motion(&mut self, app: *mut KdxApp, x: f64, y: f64) {
        // The shim reports (-1e6, -1e6) when the pointer leaves the surface.
        if x < -1000.0 {
            if self.hover.take().is_some() {
                self.needs_redraw = true;
            }
            return;
        }
        let (x, y) = (x - self.inset(), y - self.inset());
        self.hover_x = x;
        self.hover_y = y;
        let hover = self.header_hit(x, y);
        if hover != self.hover {
            self.hover = hover;
            self.needs_redraw = true;
        }

        let cursor_kind = if y < self.header_height() {
            KDX_CURSOR_POINTER
        } else if y < self.text_top() {
            KDX_CURSOR_POINTER
        } else if y < self.view_size().1 - self.status_height() {
            KDX_CURSOR_TEXT
        } else {
            KDX_CURSOR_DEFAULT
        };
        unsafe { kdx_win_set_cursor(app, cursor_kind) };

        if self.dragging_text {
            let idx = self.hit_test_editor(x, y);
            self.buf.set_cursor(idx, true);
            self.sync_primary_selection(app);
            self.needs_redraw = true;
        }
    }

    pub fn handle_pointer_button(
        &mut self,
        app: *mut KdxApp,
        button: u32,
        pressed: bool,
        mods: u32,
        x: f64,
        y: f64,
        autosave_ms: u64,
    ) -> ViewAction {
        let (x, y) = (x - self.inset(), y - self.inset());
        let (view_w, view_h) = self.view_size();
        if !pressed {
            if button == BTN_LEFT {
                self.dragging_text = false;
            }
            return ViewAction::None;
        }
        if self.is_overlay() && (x < 0.0 || y < 0.0 || x >= view_w || y >= view_h) {
            return ViewAction::None; // click on the shadow margin
        }

        // Middle-click: Wayland Primary Selection paste!
        if button == BTN_MIDDLE && y >= self.text_top() {
            let idx = self.hit_test_editor(x, y);
            self.buf.set_cursor(idx, false);
            self.paste_from_clipboard(app, true, autosave_ms);
            return ViewAction::None;
        }

        if button != BTN_LEFT {
            return ViewAction::None;
        }

        // 1. Header bar clicks (Sticky & Overlay modes)
        if self.has_header() && y < HEADER_H {
            return match self.header_hit(x, y) {
                Some(HdrBtn::Close) => ViewAction::CloseOrHide,
                Some(HdrBtn::Archive) => match &self.mode {
                    WindowMode::Sticky { note_id } => ViewAction::ArchiveSticky(note_id.clone()),
                    _ => ViewAction::None,
                },
                Some(HdrBtn::New) => ViewAction::NewSticky,
                Some(HdrBtn::Dot(i)) => {
                    self.palette_idx = i;
                    self.needs_redraw = true;
                    ViewAction::PaletteChanged(i)
                }
                None => ViewAction::StartDrag,
            };
        }

        // 2. Find/Replace bar clicks
        if self.find_open && y >= self.header_height() && y < self.text_top() {
            let fy = y - self.header_height();
            let w = view_w;
            if fy < 32.0 {
                if x >= w - 28.0 {
                    self.find_open = false;
                    self.replace_open = false;
                    self.find_focus = FindFocus::None;
                } else if x >= w - 58.0 {
                    self.case_sensitive = !self.case_sensitive;
                    self.refresh_find_matches(true);
                } else {
                    self.find_focus = FindFocus::Find;
                }
            } else {
                if x >= w - 72.0 {
                    self.replace_all_matches(autosave_ms);
                } else if x >= w - 134.0 {
                    self.replace_current(autosave_ms);
                } else {
                    self.find_focus = FindFocus::Replace;
                }
            }
            self.needs_redraw = true;
            return ViewAction::None;
        }

        // 3. Text editor click
        if y >= self.text_top() && y < view_h - self.status_height() {
            self.find_focus = FindFocus::None;
            let idx = self.hit_test_editor(x, y);
            let now = Instant::now();
            let is_multi = self
                .last_click_time
                .map(|t| now.duration_since(t).as_millis() < 380)
                .unwrap_or(false);
            self.click_streak = if is_multi {
                (self.click_streak % 3) + 1
            } else {
                1
            };
            self.last_click_time = Some(now);

            let shift = (mods & KDX_MOD_SHIFT) != 0;
            if self.click_streak == 2 {
                let ws = self
                    .buf
                    .prev_word_boundary(self.buf.next_char_boundary(idx));
                let we = self.buf.next_word_boundary(idx);
                self.buf.select_range(ws, we);
                self.sync_primary_selection(app);
            } else if self.click_streak == 3 {
                let ls = self.buf.line_start(idx);
                let le = self.buf.line_end(idx);
                self.buf.select_range(ls, le);
                self.sync_primary_selection(app);
            } else {
                self.buf.set_cursor(idx, shift);
                self.dragging_text = true;
            }
            self.caret_visible = true;
            self.needs_redraw = true;
        }

        ViewAction::None
    }

    pub fn handle_scroll(&mut self, _dx: f64, dy: f64, mods: u32) {
        if (mods & KDX_MOD_CTRL) != 0 {
            if dy < 0.0 {
                self.zoom(1.0);
            } else if dy > 0.0 {
                self.zoom(-1.0);
            }
            return;
        }
        let max_scroll = (self.content_height - self.text_viewport_height() + PAD_Y * 2.0).max(0.0);
        self.scroll_y = (self.scroll_y + dy * 3.0).clamp(0.0, max_scroll);
        self.needs_redraw = true;
    }

    fn compute_view_slice(&mut self) -> (String, usize, f64) {
        let len = self.buf.len();
        if len <= LARGE_FILE_THRESHOLD {
            self.view_slice_start = 0;
            self.view_slice_end = len;
            return (self.buf.to_string_lossy(), 0, 0.0);
        }

        // Large-file fast path (>32 KB): compute visible line window around scroll_y
        if self.cached_version != self.buf.version() {
            let mut lines = 1usize;
            for i in 0..len {
                if self.buf.byte_at(i) == Some(b'\n') {
                    lines += 1;
                }
            }
            self.total_lines_cached = lines;
            self.cached_version = self.buf.version();
        }
        self.content_height = (self.total_lines_cached as f64) * self.line_height;
        let first_line = ((self.scroll_y / self.line_height).floor() as usize).saturating_sub(4);
        let visible_lines = ((self.text_viewport_height() / self.line_height).ceil() as usize) + 12;
        let last_line = first_line + visible_lines;

        let mut cur_line = 0usize;
        let mut start_byte = 0usize;
        let mut end_byte = len;
        let mut found_start = first_line == 0;
        for i in 0..len {
            if self.buf.byte_at(i) == Some(b'\n') {
                cur_line += 1;
                if !found_start && cur_line == first_line {
                    start_byte = i + 1;
                    found_start = true;
                }
                if cur_line >= last_line {
                    end_byte = i;
                    break;
                }
            }
        }
        self.view_slice_start = start_byte;
        self.view_slice_end = end_byte;
        let y_offset = (first_line as f64) * self.line_height;
        (
            self.buf.slice_string(start_byte, end_byte),
            start_byte,
            y_offset,
        )
    }

    fn hit_test_editor(&mut self, x: f64, y: f64) -> usize {
        let (slice, slice_start, y_offset) = self.compute_view_slice();
        let lx = (x - PAD_X).max(0.0);
        let ly = (y - self.text_top() - PAD_Y + self.scroll_y - y_offset).max(0.0);
        let c_font = CString::new(self.font_family.as_str())
            .unwrap_or_else(|_| CString::new("monospace").unwrap());
        let wrap = if self.buf.len() > LARGE_FILE_THRESHOLD {
            -1
        } else {
            self.text_wrap_width()
        };
        let rel = unsafe {
            kdx_hit_test(
                c_font.as_ptr(),
                self.font_size as f64,
                slice.as_ptr() as *const c_char,
                slice.len(),
                wrap,
                lx,
                ly,
            )
        };
        let abs = slice_start + (rel.max(0) as usize);
        self.buf.clamp_char_boundary(abs)
    }

    pub fn ensure_cursor_visible(&mut self) {
        self.follow_caret = true;
        let vp_h = self.text_viewport_height();
        if self.buf.len() > LARGE_FILE_THRESHOLD {
            let cur = self.buf.cursor();
            let mut line = 0usize;
            for i in 0..cur {
                if self.buf.byte_at(i) == Some(b'\n') {
                    line += 1;
                }
            }
            let cy = (line as f64) * self.line_height;
            if cy < self.scroll_y {
                self.scroll_y = cy;
            } else if cy + self.line_height + PAD_Y * 2.0 > self.scroll_y + vp_h {
                self.scroll_y = (cy + self.line_height + PAD_Y * 2.0 - vp_h).max(0.0);
            }
            return;
        }
        // Small/normal notes: scroll adjustment is also refined during paint via exact Pango caret pos
    }

    pub fn paint(&mut self) {
        if self.handle.is_null() || !self.visible {
            return;
        }
        let mut canvas = KdxCanvas {
            cr: std::ptr::null_mut(),
            width: 0,
            height: 0,
            scale_120: 120,
        };
        let ok = unsafe { kdx_win_begin_paint(self.handle, &mut canvas) };
        if !ok {
            return;
        }
        self.draw(canvas.cr, canvas.width as f64, canvas.height as f64);
        unsafe { kdx_win_end_paint(self.handle, &mut canvas) };
        self.needs_redraw = false;
        self.follow_caret = false;
    }

    /// Draws the whole window onto `cr` (a `w` x `h` logical-px canvas). Shared by the real
    /// `wl_shm` path and the offscreen snapshot tests.
    pub(crate) fn draw(&mut self, cr: *mut std::ffi::c_void, full_w: f64, full_h: f64) {
        let pal = self.palette();
        let alpha = if self.is_sticky() || self.is_overlay() {
            self.opacity as f64
        } else {
            1.0
        };
        let ui = Ui::new(cr);

        // Overlay: drop shadow in the margin, then everything below is clipped to the panel.
        let inset = self.inset();
        let (w, h) = (full_w - inset * 2.0, full_h - inset * 2.0);
        if self.is_overlay() {
            unsafe { kdx_translate(cr, inset, inset) };
            draw_panel_shadow(cr, w, h, self.dark);
            unsafe {
                kdx_save(cr);
                kdx_clip_round_rect(cr, 0.0, 0.0, w, h, RADIUS_PANEL);
            }
        }

        // Background (Kavox `panel` step)
        unsafe { kdx_draw_rect(cr, 0.0, 0.0, w, h, pal.bg, alpha) };

        let c_mono_font = CString::new(self.font_family.as_str())
            .unwrap_or_else(|_| CString::new("monospace").unwrap());

        // 1. Header bar (Sticky & Overlay)
        if self.has_header() {
            let title = self.header_title();
            self.draw_header(&ui, &pal, w, alpha, &title);
        }

        // 2. Find & Replace bar
        if self.find_open {
            self.draw_find_bar(&ui, &pal, w);
        }

        // 3. Text Editor Area
        let top = self.text_top();
        let vp_h = self.text_viewport_height();
        let is_large = self.buf.len() > LARGE_FILE_THRESHOLD;
        let (text_slice, slice_start, y_offset) = self.compute_view_slice();
        let wrap = if is_large { -1 } else { self.text_wrap_width() };

        unsafe {
            kdx_save(cr);
            kdx_clip_rect(cr, 0.0, top, w, vp_h);
        }

        let layout = unsafe {
            kdx_layout_new(
                cr,
                c_mono_font.as_ptr(),
                self.font_size as f64,
                text_slice.as_ptr() as *const c_char,
                text_slice.len(),
                wrap,
            )
        };

        if !is_large {
            let mut lw = 0.0;
            let mut lh = 0.0;
            unsafe { kdx_layout_get_size(layout, &mut lw, &mut lh) };
            self.content_height = lh;

            // Scroll the caret into view only after it moved, not on every repaint,
            // otherwise mouse-wheel scrolling snaps straight back to the caret.
            if self.follow_caret {
                let cur = self.buf.cursor().min(text_slice.len());
                let mut cx = 0.0;
                let mut cy = 0.0;
                let mut ch = self.line_height;
                unsafe { kdx_layout_index_to_pos(layout, cur as i32, &mut cx, &mut cy, &mut ch) };
                if cy < self.scroll_y {
                    self.scroll_y = cy;
                } else if cy + ch + PAD_Y * 2.0 > self.scroll_y + vp_h {
                    self.scroll_y = (cy + ch + PAD_Y * 2.0 - vp_h).max(0.0);
                }
            }
            let max_scroll = (self.content_height - vp_h + PAD_Y * 2.0).max(0.0);
            if self.scroll_y > max_scroll {
                self.scroll_y = max_scroll;
            }
        }

        let origin_x = PAD_X;
        let origin_y = top + PAD_Y - self.scroll_y + y_offset;

        // Selection highlight: Kavox `lit` at alpha 56 (dark) / 40 (light)
        if let Some((sel_s, sel_e)) = self.buf.selection() {
            let slice_end = slice_start + text_slice.len();
            if sel_e > slice_start && sel_s < slice_end {
                let rs = sel_s.saturating_sub(slice_start) as i32;
                let re = (sel_e.min(slice_end) - slice_start) as i32;
                unsafe {
                    kdx_layout_draw_selection(
                        cr,
                        layout,
                        origin_x,
                        origin_y,
                        rs,
                        re,
                        pal.lit,
                        pal.selection_alpha as f64 / 255.0,
                    );
                }
            }
        }

        // Empty-note placeholder hint
        if self.buf.is_empty() {
            let ph = "Type a note...";
            let pl = unsafe {
                kdx_layout_new(
                    cr,
                    c_mono_font.as_ptr(),
                    self.font_size as f64,
                    ph.as_ptr() as *const c_char,
                    ph.len(),
                    -1,
                )
            };
            unsafe {
                kdx_layout_draw(cr, pl, origin_x, origin_y, pal.muted, 0.65);
                kdx_layout_free(pl);
            }
        } else {
            unsafe { kdx_layout_draw(cr, layout, origin_x, origin_y, pal.fg, 1.0) };
        }

        // Caret: Kavox red in every palette
        if self.focused && self.caret_visible && self.find_focus == FindFocus::None {
            let cur = self.buf.cursor();
            let slice_end = slice_start + text_slice.len();
            if cur >= slice_start && cur <= slice_end {
                let rel = (cur - slice_start) as i32;
                let mut cx = 0.0;
                let mut cy = 0.0;
                let mut ch = self.line_height;
                unsafe { kdx_layout_index_to_pos(layout, rel, &mut cx, &mut cy, &mut ch) };
                let caret_h = if ch > 4.0 { ch } else { self.line_height };
                unsafe {
                    kdx_draw_round_rect(
                        cr,
                        origin_x + cx,
                        origin_y + cy + 1.0,
                        1.75,
                        caret_h - 2.0,
                        0.875,
                        pal.caret(),
                        1.0,
                    );
                }
            }
        }

        unsafe { kdx_layout_free(layout) };

        // Subtle scrollbar thumb
        let total_h = self.content_height + PAD_Y * 2.0;
        if total_h > vp_h + 2.0 {
            let thumb_h = ((vp_h / total_h) * vp_h).clamp(24.0, vp_h - 8.0);
            let max_s = (total_h - vp_h).max(1.0);
            let thumb_y = top + 4.0 + (self.scroll_y / max_s) * (vp_h - thumb_h - 8.0);
            unsafe {
                kdx_draw_round_rect(cr, w - 7.0, thumb_y, 4.0, thumb_h, 2.0, pal.muted, 0.38);
            }
        }

        unsafe { kdx_restore(cr) };

        // 4. Status bar (File Mode)
        if self.has_status_bar() {
            self.draw_status_bar(&ui, &pal, w, h);
        }

        // Outer border: 1 px `line` (Kavox panel/card outline)
        if self.is_overlay() {
            unsafe {
                kdx_restore(cr); // drop the panel clip
                kdx_stroke_round_rect(
                    cr,
                    0.5,
                    0.5,
                    w - 1.0,
                    h - 1.0,
                    RADIUS_PANEL - 0.5,
                    1.0,
                    pal.border,
                    1.0,
                );
            }
        } else if self.is_sticky() {
            unsafe {
                kdx_stroke_round_rect(cr, 0.5, 0.5, w - 1.0, h - 1.0, 0.0, 1.0, pal.border, 1.0);
            }
        }
    }

    /// First line of the note for the sticky header (empty for an empty note).
    fn header_title(&mut self) -> String {
        if !self.is_sticky() {
            return String::new();
        }
        let version = self.buf.version();
        if let Some((v, t)) = &self.title_cache {
            if *v == version {
                return t.clone();
            }
        }
        let first = self.buf.first_non_empty_line();
        let title = if first == "(empty note)" {
            String::new()
        } else {
            first
        };
        self.title_cache = Some((version, title.clone()));
        title
    }

    fn draw_header(&self, ui: &Ui, pal: &Palette, w: f64, alpha: f64, title: &str) {
        let cr = ui.cr;
        let cy = HEADER_H * 0.5;
        let pad = self.header_pad();
        unsafe {
            kdx_draw_rect(cr, 0.0, 0.0, w, HEADER_H, pal.header, alpha);
            kdx_draw_rect(cr, 0.0, HEADER_H - 1.0, w, 1.0, pal.border, 1.0);
        }

        // Icon button: hover = `hover` fill, danger hover = red tint + red text.
        let button = |cx: f64, icon: Icon, which: HdrBtn| {
            let hovered = self.hover == Some(which);
            let danger = which == HdrBtn::Close;
            let mut color = pal.muted;
            if hovered {
                unsafe {
                    if danger {
                        kdx_draw_round_rect(
                            cr,
                            cx - BTN / 2.0,
                            cy - BTN / 2.0,
                            BTN,
                            BTN,
                            BTN_RADIUS,
                            RED,
                            DANGER_TINT,
                        );
                    } else {
                        kdx_draw_round_rect(
                            cr,
                            cx - BTN / 2.0,
                            cy - BTN / 2.0,
                            BTN,
                            BTN,
                            BTN_RADIUS,
                            pal.hover,
                            1.0,
                        );
                    }
                }
                color = if danger { pal.red_text } else { pal.fg };
            }
            ui.icon(icon, cx, cy, ICON_PX, color, 1.0);
        };

        if self.is_sticky() {
            button(16.0, Icon::Plus, HdrBtn::New);
            button(w - 40.0 - pad, Icon::Archive, HdrBtn::Archive);
        }
        button(w - 14.0 - pad, Icon::Close, HdrBtn::Close);

        // Palette dots: accent fill with a 1 px `line_strong` ring; the active one gets a `text` halo.
        let start = self.dots_start();
        for i in 0..PALETTE_COUNT {
            let dot = palette(i, self.dark);
            let cx = start + i as f64 * 16.0;
            unsafe {
                kdx_draw_circle(cr, cx, cy, 4.5, dot.accent, 1.0);
                kdx_stroke_circle(cr, cx, cy, 4.5, 1.0, pal.border_strong, 1.0);
                if i == self.palette_idx {
                    kdx_stroke_circle(cr, cx, cy, 7.0, 1.5, pal.fg, 1.0);
                } else if self.hover == Some(HdrBtn::Dot(i)) {
                    kdx_stroke_circle(cr, cx, cy, 7.0, 1.0, pal.muted, 1.0);
                }
            }
        }

        if self.is_sticky() && !title.is_empty() {
            // Between the dots and the archive button, ellipsised, in `muted`.
            let x = start + (PALETTE_COUNT as f64) * 16.0 + 4.0;
            let room = w - 52.0 - 6.0 - x;
            if room > 40.0 {
                let text = ui.ellipsize(title, UI_PT, room);
                ui.text(&text, UI_PT, x, cy, pal.muted, 1.0);
            }
        }

        if self.is_overlay() {
            let x = start + (PALETTE_COUNT as f64) * 16.0 + 4.0;
            let a = ui.text("Kadex Scratchpad", UI_PT, x, cy, pal.muted, 1.0);
            unsafe { kdx_draw_circle(cr, x + a + 9.0, cy, 1.2, pal.faint, 1.0) };
            ui.text("Esc to hide", UI_PT, x + a + 18.0, cy, pal.muted, 1.0);
        }
    }

    fn draw_find_bar(&self, ui: &Ui, pal: &Palette, w: f64) {
        let cr = ui.cr;
        let fy = self.header_height();
        let fh = self.find_bar_height();
        unsafe {
            kdx_draw_rect(cr, 0.0, fy, w, fh, pal.header, 1.0);
            kdx_draw_rect(cr, 0.0, fy + fh - 1.0, w, 1.0, pal.border, 1.0);
        }
        // Inputs: recessed `field` with a 1 px `line` outline, `text` outline when focused.
        let input = |x: f64, y: f64, bw: f64, focused: bool| unsafe {
            kdx_draw_round_rect(
                cr,
                x,
                y,
                bw,
                24.0,
                7.0,
                if focused { pal.fg } else { pal.border },
                1.0,
            );
            kdx_draw_round_rect(cr, x + 1.0, y + 1.0, bw - 2.0, 22.0, 6.0, pal.field, 1.0);
        };

        let box_w = (w - 130.0).max(80.0);
        input(8.0, fy + 4.0, box_w, self.find_focus == FindFocus::Find);
        let (disp, col) = if self.find_query.is_empty() {
            ("Find...", pal.faint)
        } else {
            (self.find_query.as_str(), pal.fg)
        };
        ui.text(disp, UI_PT, 14.0, fy + 16.0, col, 1.0);

        // Match count, Aa toggle (active = `lit` pill), close
        let count_str = format!("{}/{}", self.current_match, self.match_count);
        ui.text(&count_str, UI_PT, w - 114.0, fy + 16.0, pal.muted, 1.0);
        if self.case_sensitive {
            unsafe { kdx_draw_round_rect(cr, w - 57.0, fy + 5.0, 26.0, 22.0, 7.0, pal.lit, 1.0) };
        }
        ui.text(
            "Aa",
            UI_PT,
            w - 52.0,
            fy + 16.0,
            if self.case_sensitive {
                pal.on_lit
            } else {
                pal.muted
            },
            1.0,
        );
        ui.icon(Icon::Close, w - 14.0, fy + 16.0, 14.0, pal.muted, 1.0);

        if self.replace_open {
            let ry = fy + 32.0;
            let rbox_w = (w - 150.0).max(80.0);
            input(8.0, ry + 3.0, rbox_w, self.find_focus == FindFocus::Replace);
            let (rdisp, rcol) = if self.replace_query.is_empty() {
                ("Replace...", pal.faint)
            } else {
                (self.replace_query.as_str(), pal.fg)
            };
            ui.text(rdisp, UI_PT, 14.0, ry + 15.0, rcol, 1.0);

            // Replace (secondary, `hover` fill) / All (primary, `lit` fill)
            unsafe {
                kdx_draw_round_rect(cr, w - 134.0, ry + 4.0, 56.0, 22.0, 7.0, pal.hover, 1.0);
                kdx_draw_round_rect(cr, w - 72.0, ry + 4.0, 62.0, 22.0, 7.0, pal.lit, 1.0);
            }
            let (rw, _) = ui.size("Replace", UI_PT);
            ui.text(
                "Replace",
                UI_PT,
                w - 134.0 + (56.0 - rw) / 2.0,
                ry + 15.5,
                pal.fg,
                1.0,
            );
            let (aw, _) = ui.size("All", UI_PT);
            ui.text(
                "All",
                UI_PT,
                w - 72.0 + (62.0 - aw) / 2.0,
                ry + 15.5,
                pal.on_lit,
                1.0,
            );
        }
    }

    fn draw_status_bar(&self, ui: &Ui, pal: &Palette, w: f64, h: f64) {
        let WindowMode::File {
            encoding,
            line_ending,
            ..
        } = &self.mode
        else {
            return;
        };
        let cr = ui.cr;
        let sy = h - STATUS_H;
        unsafe {
            kdx_draw_rect(cr, 0.0, sy, w, STATUS_H, pal.header, 1.0);
            kdx_draw_rect(cr, 0.0, sy, w, 1.0, pal.border, 1.0);
        }
        let cy = sy + 1.0 + (STATUS_H - 1.0) * 0.5;
        let (ln, col) = self.cursor_line_col();
        let zoom_pct = ((self.font_size / self.default_font_size) * 100.0).round() as i32;
        const GAP: f64 = 9.0; // text edge to separator dot centre

        // Left: position, then the modified marker.
        let pos = format!("Ln {ln}, Col {col}");
        let mut x = 12.0;
        x += ui.text(&pos, STATUS_PT, x, cy, pal.muted, 1.0);
        if self.buf.is_dirty() {
            unsafe { kdx_draw_circle(cr, x + GAP, cy, 1.2, pal.faint, 1.0) };
            ui.text("Modified", STATUS_PT, x + GAP * 2.0, cy, pal.fg, 1.0);
        }

        // Right: encoding, EOL, wrap, zoom (right-aligned, faint middle dots between them).
        let zoom = format!("{zoom_pct}%");
        let items = [
            encoding.label(),
            line_ending.label(),
            if self.word_wrap { "Wrap" } else { "NoWrap" },
            zoom.as_str(),
        ];
        let mut rx = w - 12.0;
        for (i, item) in items.iter().enumerate().rev() {
            let (iw, _) = ui.size(item, STATUS_PT);
            ui.text(item, STATUS_PT, rx - iw, cy, pal.muted, 1.0);
            rx -= iw;
            if i > 0 {
                unsafe { kdx_draw_circle(cr, rx - GAP, cy, 1.2, pal.faint, 1.0) };
                rx -= GAP * 2.0;
            }
        }
    }

    fn cursor_line_col(&self) -> (usize, usize) {
        let cur = self.buf.cursor();
        // Fast bounded scan for huge files
        if cur > 256 * 1024 {
            let ls = self.buf.line_start(cur);
            return (1, self.buf.slice_string(ls, cur).chars().count() + 1);
        }
        let mut line = 1usize;
        let mut last_nl = 0usize;
        for i in 0..cur {
            if self.buf.byte_at(i) == Some(b'\n') {
                line += 1;
                last_nl = i + 1;
            }
        }
        (
            line,
            self.buf.slice_string(last_nl, cur).chars().count() + 1,
        )
    }
}

/// UI chrome helpers: Archivo Medium text and lucide-style stroked icons.
struct Ui {
    cr: *mut std::ffi::c_void,
    family: CString,
}

impl Ui {
    fn new(cr: *mut std::ffi::c_void) -> Self {
        Self {
            cr,
            family: CString::new(UI_FAMILY).unwrap_or_default(),
        }
    }

    fn layout(&self, text: &str, pt: f64) -> *mut KdxLayout {
        unsafe {
            let l = kdx_layout_new(
                self.cr,
                self.family.as_ptr(),
                pt,
                text.as_ptr() as *const c_char,
                text.len(),
                -1,
            );
            kdx_layout_set_subpixel(l);
            l
        }
    }

    fn size(&self, text: &str, pt: f64) -> (f64, f64) {
        let l = self.layout(text, pt);
        let (mut w, mut h) = (0.0, 0.0);
        unsafe {
            kdx_layout_get_size(l, &mut w, &mut h);
            kdx_layout_free(l);
        }
        (w, h)
    }

    /// Shortens `text` with a trailing "…" so it fits `max_w` px.
    fn ellipsize(&self, text: &str, pt: f64, max_w: f64) -> String {
        if self.size(text, pt).0 <= max_w {
            return text.to_string();
        }
        let chars: Vec<char> = text.chars().collect();
        let (mut lo, mut hi) = (0usize, chars.len());
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            let cand: String = chars[..mid].iter().collect::<String>() + "\u{2026}";
            if self.size(&cand, pt).0 <= max_w {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        chars[..lo]
            .iter()
            .collect::<String>()
            .trim_end()
            .to_string()
            + "\u{2026}"
    }

    /// Draws `text` with its line box vertically centred on `cy`; returns the advance width.
    fn text(&self, text: &str, pt: f64, x: f64, cy: f64, rgb: u32, alpha: f64) -> f64 {
        let l = self.layout(text, pt);
        let (mut w, mut h) = (0.0, 0.0);
        unsafe {
            kdx_layout_get_size(l, &mut w, &mut h);
            kdx_layout_draw(self.cr, l, x, (cy - h / 2.0).round(), rgb, alpha);
            kdx_layout_free(l);
        }
        w
    }

    /// Strokes `icon` (24-unit grid, stroke 2 units, round caps/joins) centred on `(cx, cy)`.
    fn icon(&self, icon: Icon, cx: f64, cy: f64, px: f64, rgb: u32, alpha: f64) {
        let cr = self.cr;
        let k = px / GRID as f64;
        unsafe {
            kdx_save(cr);
            kdx_translate(cr, cx - px / 2.0, cy - px / 2.0);
            kdx_scale(cr, k, k);
            kdx_path_begin(cr);
            for seg in icon.segments() {
                match *seg {
                    Seg::M(x, y) => kdx_path_move(cr, x as f64, y as f64),
                    Seg::L(x, y) => kdx_path_line(cr, x as f64, y as f64),
                    Seg::C(a, b, c, d, e, f) => kdx_path_curve(
                        cr, a as f64, b as f64, c as f64, d as f64, e as f64, f as f64,
                    ),
                    Seg::Z => kdx_path_close(cr),
                }
            }
            kdx_path_stroke(cr, STROKE as f64, rgb, alpha);
            kdx_restore(cr);
        }
    }
}

/// Soft drop shadow of the overlay panel: stacked rounded rects approximate a blur (Kavox popover
/// shadow: black, 12 px down, wide blur; lighter on light themes).
fn draw_panel_shadow(cr: *mut std::ffi::c_void, w: f64, h: f64, dark: bool) {
    const LAYERS: i32 = 14;
    const BLUR: f64 = 28.0;
    const OFFSET_Y: f64 = 8.0;
    let peak = if dark { 0.5 } else { 0.2 };
    for i in 0..LAYERS {
        let t = (i as f64 + 0.5) / LAYERS as f64; // 0..1 from the outside in
        let grow = BLUR / 2.0 - t * BLUR;
        let a = peak / LAYERS as f64 * 1.9;
        unsafe {
            kdx_draw_round_rect(
                cr,
                -grow,
                -grow + OFFSET_Y,
                w + grow * 2.0,
                h + grow * 2.0,
                RADIUS_PANEL + grow,
                0x000000,
                a,
            );
        }
    }
}
