//! Native Wayland (`wl_shm` + Pango/Cairo + xkbcommon) backend for Kadex.

mod editor_view;
mod ffi;
mod ipc_unix;

use std::ffi::{c_char, c_double, c_int, c_void, CString};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use kadex_core::config::{NoteWindowState, ThemeMode};
use kadex_core::ipc::IpcCommand;
use kadex_core::storage::{load_text_file, save_text_file, AppPaths, Encoding, LineEnding, Storage};

use crate::editor_view::{EditorWindow, ViewAction, WindowMode};
use crate::ffi::*;
use crate::ipc_unix::IpcServer;

static QUIT_REQUESTED: AtomicBool = AtomicBool::new(false);

extern "C" fn handle_signal(_sig: c_int) {
    QUIT_REQUESTED.store(true, Ordering::SeqCst);
}

/// Fast-path IPC send to an existing Kadex daemon.
pub fn try_send_ipc(cmd: &IpcCommand) -> Option<String> {
    let sock = AppPaths::default_socket_path();
    ipc_unix::send_command(&sock, cmd).ok()
}

/// Standalone `kadex list` fallback when no daemon is running.
pub fn list_notes_offline() -> Result<String, String> {
    let storage = Storage::open().map_err(|e| e.to_string())?;
    let entries = storage.list_notes().map_err(|e| e.to_string())?;
    let mut out = String::new();
    for e in entries {
        out.push_str(&e.id);
        out.push('\t');
        out.push_str(&e.title);
        out.push('\n');
    }
    Ok(out)
}

#[derive(Clone)]
struct RepeatKey {
    win_id: u64,
    keysym: u32,
    mods: u32,
    utf8: String,
    next_fire: Instant,
}

struct Daemon {
    app: *mut KdxApp,
    storage: Storage,
    windows: Vec<EditorWindow>,
    next_win_id: u64,
    dark: bool,
    sticky_visible: bool,
    running: bool,
    repeat_rate: i32,
    repeat_delay_ms: i32,
    active_repeat: Option<RepeatKey>,
    caret_next_blink: Option<Instant>,
}

impl Daemon {
    fn detect_dark_theme(mode: ThemeMode) -> bool {
        match mode {
            ThemeMode::Dark => true,
            ThemeMode::Light => false,
            ThemeMode::Auto => {
                // Lightweight check without spinning up D-Bus/GTK
                if let Ok(t) = std::env::var("GTK_THEME") {
                    if t.to_ascii_lowercase().contains("light") {
                        return false;
                    }
                }
                true
            }
        }
    }

    fn alloc_id(&mut self) -> u64 {
        let id = self.next_win_id;
        self.next_win_id += 1;
        id
    }

    fn find_win_mut(&mut self, id: u64) -> Option<&mut EditorWindow> {
        self.windows.iter_mut().find(|w| w.id == id)
    }

    fn create_native_surface(&self, id: u64, mode: &WindowMode, title: &str, w: i32, h: i32) -> *mut KdxWin {
        let (kind, app_id) = match mode {
            WindowMode::Sticky { .. } => (KDX_WIN_XDG, "kadex.sticky"),
            WindowMode::File { .. } => (KDX_WIN_XDG, "kadex.editor"),
            WindowMode::Overlay { .. } => (KDX_WIN_LAYER_OVERLAY, "kadex.overlay"),
        };
        let c_app_id = CString::new(app_id).unwrap();
        let c_title = CString::new(title).unwrap_or_else(|_| CString::new("Kadex").unwrap());
        unsafe { kdx_win_create(self.app, id, kind, c_app_id.as_ptr(), c_title.as_ptr(), w, h) }
    }

    fn spawn_sticky(&mut self, note_id: Option<String>) -> u64 {
        let id_str = note_id.unwrap_or_else(|| self.storage.new_note_id());
        // If already tracked, show it
        if let Some(idx) = self.windows.iter().position(|w| {
            matches!(&w.mode, WindowMode::Sticky { note_id } if note_id == &id_str)
        }) {
            if !self.windows[idx].visible || self.windows[idx].handle.is_null() {
                let wid = self.windows[idx].id;
                let title = self.windows[idx].window_title();
                let w = self.windows[idx].width;
                let h = self.windows[idx].height;
                let handle = self.create_native_surface(wid, &self.windows[idx].mode, &title, w, h);
                self.windows[idx].handle = handle;
                self.windows[idx].visible = true;
                self.windows[idx].needs_redraw = true;
            }
            self.sticky_visible = true;
            return self.windows[idx].id;
        }

        let text = self.storage.load_note(&id_str).unwrap_or_default();
        let saved = self.storage.state.notes.get(&id_str).cloned();
        let width = saved.as_ref().map(|s| s.width).unwrap_or(self.storage.config.sticky_width) as i32;
        let height = saved.as_ref().map(|s| s.height).unwrap_or(self.storage.config.sticky_height) as i32;
        let pal_idx = saved.as_ref().map(|s| s.palette).unwrap_or(self.storage.config.default_palette);

        let win_id = self.alloc_id();
        let mode = WindowMode::Sticky { note_id: id_str.clone() };
        let handle = self.create_native_surface(win_id, &mode, "Kadex Note", width, height);

        let win = EditorWindow::new(
            win_id,
            mode,
            handle,
            &text,
            width,
            height,
            pal_idx,
            self.dark,
            self.storage.config.font_family.clone(),
            self.storage.config.font_size,
            self.storage.config.word_wrap,
            self.storage.config.opacity,
        );
        win.update_title();

        // Persist initial note file & state entry
        let _ = self.storage.save_note(&id_str, &text);
        self.storage.state.notes.entry(id_str.clone()).or_insert_with(|| {
            NoteWindowState::new(id_str, pal_idx, width as u32, height as u32)
        });
        let _ = self.storage.save_state();

        self.windows.push(win);
        self.sticky_visible = true;
        win_id
    }

    fn open_file(&mut self, path_str: &str) {
        let path = PathBuf::from(path_str);
        let (text, encoding, line_ending) = if path.exists() {
            match load_text_file(&path) {
                Ok(loaded) => (loaded.text, loaded.encoding, loaded.line_ending),
                Err(_) => (String::new(), Encoding::Utf8, LineEnding::platform_default()),
            }
        } else {
            (String::new(), Encoding::Utf8, LineEnding::platform_default())
        };

        let win_id = self.alloc_id();
        let mode = WindowMode::File {
            path: path.clone(),
            encoding,
            line_ending,
        };
        let w = self.storage.config.editor_width as i32;
        let h = self.storage.config.editor_height as i32;
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("Untitled");
        let title = format!("{name} — Kadex");
        let handle = self.create_native_surface(win_id, &mode, &title, w, h);

        let win = EditorWindow::new(
            win_id,
            mode,
            handle,
            &text,
            w,
            h,
            0,
            self.dark,
            self.storage.config.font_family.clone(),
            self.storage.config.font_size,
            self.storage.config.word_wrap,
            1.0,
        );
        self.windows.push(win);
    }

    fn toggle_overlay(&mut self) {
        if let Some(pos) = self.windows.iter().position(|w| w.is_overlay()) {
            self.flush_window(pos);
            let w = self.windows.remove(pos);
            if !w.handle.is_null() {
                unsafe { kdx_win_destroy(w.handle) };
            }
            return;
        }
        let note_id = String::from("scratchpad");
        let text = self.storage.load_note(&note_id).unwrap_or_default();
        let win_id = self.alloc_id();
        let mode = WindowMode::Overlay { note_id };
        let w = 560;
        let h = 380;
        let handle = self.create_native_surface(win_id, &mode, "Kadex Scratchpad", w, h);
        let win = EditorWindow::new(
            win_id,
            mode,
            handle,
            &text,
            w,
            h,
            self.storage.config.default_palette,
            self.dark,
            self.storage.config.font_family.clone(),
            self.storage.config.font_size,
            self.storage.config.word_wrap,
            self.storage.config.opacity,
        );
        self.windows.push(win);
    }

    fn activate_sticky(&mut self) {
        let has_any_sticky = self.windows.iter().any(|w| w.is_sticky());
        if !has_any_sticky {
            // Re-open previously open sticky notes from state.ini, or spawn a fresh one
            let open_ids: Vec<String> = self
                .storage
                .state
                .notes
                .values()
                .filter(|n| n.open && self.storage.paths.note_path(&n.id).exists())
                .map(|n| n.id.clone())
                .collect();
            if open_ids.is_empty() {
                self.spawn_sticky(None);
            } else {
                for id in open_ids {
                    self.spawn_sticky(Some(id));
                }
            }
        } else if !self.sticky_visible {
            self.show_all_sticky();
        }
    }

    fn hide_all_sticky(&mut self) {
        for i in 0..self.windows.len() {
            if self.windows[i].is_sticky() {
                self.flush_window(i);
                if !self.windows[i].handle.is_null() {
                    unsafe { kdx_win_destroy(self.windows[i].handle) };
                    self.windows[i].handle = std::ptr::null_mut();
                }
                self.windows[i].visible = false;
                self.windows[i].focused = false;
            }
        }
        self.sticky_visible = false;
        self.active_repeat = None;
    }

    fn show_all_sticky(&mut self) {
        let ids: Vec<(usize, u64, i32, i32, String)> = self
            .windows
            .iter()
            .enumerate()
            .filter(|(_, w)| w.is_sticky() && (!w.visible || w.handle.is_null()))
            .map(|(i, w)| (i, w.id, w.width, w.height, w.window_title()))
            .collect();
        if ids.is_empty() && !self.windows.iter().any(|w| w.is_sticky()) {
            self.activate_sticky();
            return;
        }
        for (idx, wid, w, h, title) in ids {
            let handle = self.create_native_surface(wid, &self.windows[idx].mode, &title, w, h);
            self.windows[idx].handle = handle;
            self.windows[idx].visible = true;
            self.windows[idx].needs_redraw = true;
        }
        self.sticky_visible = true;
    }

    fn toggle_sticky(&mut self) {
        let any_visible = self.windows.iter().any(|w| w.is_sticky() && w.visible);
        if any_visible {
            self.hide_all_sticky();
        } else {
            self.show_all_sticky();
        }
    }

    fn flush_window(&mut self, idx: usize) {
        if idx >= self.windows.len() {
            return;
        }
        let win = &mut self.windows[idx];
        win.autosave_due = None;
        if !win.buf.is_dirty() {
            return;
        }
        let text = win.buf.to_string_lossy();
        match &win.mode {
            WindowMode::Sticky { note_id } | WindowMode::Overlay { note_id } => {
                if self.storage.save_note(note_id, &text).is_ok() {
                    win.buf.mark_clean();
                }
            }
            WindowMode::File {
                path,
                encoding,
                line_ending,
            } => {
                if save_text_file(path, &text, *encoding, *line_ending).is_ok() {
                    win.buf.mark_clean();
                    win.update_title();
                    win.needs_redraw = true;
                }
            }
        }
    }

    fn flush_all_sticky(&mut self) {
        for i in 0..self.windows.len() {
            if matches!(self.windows[i].mode, WindowMode::Sticky { .. } | WindowMode::Overlay { .. }) {
                self.flush_window(i);
            }
        }
        let _ = self.storage.save_state();
    }

    fn handle_ipc_command(&mut self, cmd: IpcCommand) -> String {
        match cmd {
            IpcCommand::Activate => {
                self.activate_sticky();
                String::from("OK\n")
            }
            IpcCommand::NewSticky => {
                self.spawn_sticky(None);
                String::from("OK\n")
            }
            IpcCommand::OpenFile(p) => {
                self.open_file(&p);
                String::from("OK\n")
            }
            IpcCommand::OpenNote(id) => {
                self.spawn_sticky(Some(id));
                String::from("OK\n")
            }
            IpcCommand::Toggle => {
                self.toggle_sticky();
                String::from("OK\n")
            }
            IpcCommand::Hide => {
                self.hide_all_sticky();
                String::from("OK\n")
            }
            IpcCommand::Overlay => {
                self.toggle_overlay();
                String::from("OK\n")
            }
            IpcCommand::List => {
                self.flush_all_sticky();
                match self.storage.list_notes() {
                    Ok(entries) => {
                        let mut out = String::new();
                        for e in entries {
                            out.push_str(&e.id);
                            out.push('\t');
                            out.push_str(&e.title);
                            out.push('\n');
                        }
                        out
                    }
                    Err(e) => format!("ERR {e}\n"),
                }
            }
            IpcCommand::Ping => String::from("OK PONG\n"),
            IpcCommand::Quit => {
                self.flush_all_sticky();
                self.running = false;
                String::from("OK\n")
            }
        }
    }

    fn apply_view_action(&mut self, win_id: u64, action: ViewAction) {
        match action {
            ViewAction::None => {}
            ViewAction::NewSticky => {
                self.spawn_sticky(None);
            }
            ViewAction::ArchiveSticky(note_id) => {
                if let Some(pos) = self.windows.iter().position(|w| w.id == win_id) {
                    self.flush_window(pos);
                    let w = self.windows.remove(pos);
                    if !w.handle.is_null() {
                        unsafe { kdx_win_destroy(w.handle) };
                    }
                }
                let _ = self.storage.archive_note(&note_id);
                self.maybe_exit_if_empty();
            }
            ViewAction::CloseOrHide => {
                self.close_or_hide_window(win_id);
            }
            ViewAction::SaveNow => {
                if let Some(pos) = self.windows.iter().position(|w| w.id == win_id) {
                    self.flush_window(pos);
                }
            }
            ViewAction::PaletteChanged(pal) => {
                if let Some(w) = self.windows.iter().find(|w| w.id == win_id) {
                    if let WindowMode::Sticky { note_id } = &w.mode {
                        if let Some(st) = self.storage.state.notes.get_mut(note_id) {
                            st.palette = pal;
                            let _ = self.storage.save_state();
                        }
                    }
                }
            }
            ViewAction::StartDrag => {
                if let Some(w) = self.windows.iter().find(|w| w.id == win_id) {
                    if !w.handle.is_null() {
                        unsafe { kdx_win_start_move(w.handle) };
                    }
                }
            }
        }
    }

    fn close_or_hide_window(&mut self, win_id: u64) {
        let Some(pos) = self.windows.iter().position(|w| w.id == win_id) else {
            return;
        };
        self.flush_window(pos);
        match &self.windows[pos].mode {
            WindowMode::Sticky { .. } => {
                // Hide sticky note surface while keeping note persisted in daemon
                if !self.windows[pos].handle.is_null() {
                    unsafe { kdx_win_destroy(self.windows[pos].handle) };
                    self.windows[pos].handle = std::ptr::null_mut();
                }
                self.windows[pos].visible = false;
                self.windows[pos].focused = false;
                self.sticky_visible = self.windows.iter().any(|w| w.is_sticky() && w.visible);
                self.active_repeat = None;
            }
            WindowMode::Overlay { .. } | WindowMode::File { .. } => {
                let w = self.windows.remove(pos);
                if !w.handle.is_null() {
                    unsafe { kdx_win_destroy(w.handle) };
                }
                self.active_repeat = None;
                self.maybe_exit_if_empty();
            }
        }
    }

    fn maybe_exit_if_empty(&mut self) {
        // If no windows at all (no hidden sticky notes either), exit cleanly
        if self.windows.is_empty() {
            self.running = false;
        }
    }

    fn next_deadline(&self) -> Option<Instant> {
        let mut earliest: Option<Instant> = None;
        let mut consider = |t: Instant| {
            earliest = Some(match earliest {
                Some(e) if e <= t => e,
                _ => t,
            });
        };
        for w in &self.windows {
            if let Some(due) = w.autosave_due {
                consider(due);
            }
        }
        if let Some(rep) = &self.active_repeat {
            consider(rep.next_fire);
        }
        if let Some(blink) = self.caret_next_blink {
            if self.windows.iter().any(|w| w.focused && w.visible) {
                consider(blink);
            }
        }
        earliest
    }

    fn process_timers(&mut self) {
        let now = Instant::now();

        // 1. Debounced autosaves
        for i in 0..self.windows.len() {
            if let Some(due) = self.windows[i].autosave_due {
                if now >= due {
                    self.flush_window(i);
                }
            }
        }

        // 2. Key repeat
        if let Some(mut rep) = self.active_repeat.clone() {
            if now >= rep.next_fire {
                let autosave_ms = self.storage.config.autosave_ms;
                let app = self.app;
                let action = if let Some(w) = self.find_win_mut(rep.win_id) {
                    w.handle_key(app, rep.keysym, rep.mods, &rep.utf8, autosave_ms)
                } else {
                    self.active_repeat = None;
                    ViewAction::None
                };
                self.apply_view_action(rep.win_id, action);
                if self.active_repeat.is_some() {
                    let interval_ms = if self.repeat_rate > 0 {
                        (1000 / self.repeat_rate).max(16) as u64
                    } else {
                        40
                    };
                    rep.next_fire = Instant::now() + Duration::from_millis(interval_ms);
                    self.active_repeat = Some(rep);
                }
            }
        }

        // 3. Caret blink (only while a window is focused)
        if let Some(blink) = self.caret_next_blink {
            if now >= blink {
                let mut any_focused = false;
                for w in &mut self.windows {
                    if w.focused && w.visible {
                        w.caret_visible = !w.caret_visible;
                        w.needs_redraw = true;
                        any_focused = true;
                    }
                }
                self.caret_next_blink = if any_focused {
                    Some(now + Duration::from_millis(530))
                } else {
                    None
                };
            }
        }
    }

    fn paint_dirty(&mut self) {
        for w in &mut self.windows {
            if w.visible && w.needs_redraw && !w.handle.is_null() {
                w.paint();
            }
        }
    }
}

// ---- C -> Rust Callbacks ----
extern "C" fn cb_configure(ud: *mut c_void, win_id: u64, width: i32, height: i32, scale_120: u32) {
    let d = unsafe { &mut *(ud as *mut Daemon) };
    if let Some(w) = d.find_win_mut(win_id) {
        w.width = width;
        w.height = height;
        w.scale_120 = scale_120;
        w.needs_redraw = true;
        if let WindowMode::Sticky { note_id } = &w.mode {
            let note_id = note_id.clone();
            if let Some(st) = d.storage.state.notes.get_mut(&note_id) {
                st.width = width as u32;
                st.height = height as u32;
            }
        }
    }
}

extern "C" fn cb_close(ud: *mut c_void, win_id: u64) {
    let d = unsafe { &mut *(ud as *mut Daemon) };
    d.close_or_hide_window(win_id);
}

extern "C" fn cb_focus(ud: *mut c_void, win_id: u64, focused: bool) {
    let d = unsafe { &mut *(ud as *mut Daemon) };
    if let Some(idx) = d.windows.iter().position(|w| w.id == win_id) {
        d.windows[idx].focused = focused;
        d.windows[idx].caret_visible = focused;
        d.windows[idx].needs_redraw = true;
        if !focused {
            d.active_repeat = None;
            // Immediate crash-safe flush on focus loss!
            d.flush_window(idx);
        } else {
            d.caret_next_blink = Some(Instant::now() + Duration::from_millis(530));
        }
    }
}

extern "C" fn cb_key(
    ud: *mut c_void,
    win_id: u64,
    keysym: u32,
    mods: u32,
    utf8_ptr: *const c_char,
    utf8_len: usize,
    pressed: bool,
) {
    let d = unsafe { &mut *(ud as *mut Daemon) };
    if !pressed {
        if d.active_repeat.as_ref().map(|r| r.keysym == keysym).unwrap_or(false) {
            d.active_repeat = None;
        }
        return;
    }

    let utf8 = if !utf8_ptr.is_null() && utf8_len > 0 {
        let bytes = unsafe { std::slice::from_raw_parts(utf8_ptr as *const u8, utf8_len) };
        String::from_utf8_lossy(bytes).into_owned()
    } else {
        String::new()
    };

    let autosave_ms = d.storage.config.autosave_ms;
    let app = d.app;
    let action = if let Some(w) = d.find_win_mut(win_id) {
        w.handle_key(app, keysym, mods, &utf8, autosave_ms)
    } else {
        ViewAction::None
    };
    d.apply_view_action(win_id, action);

    // Arm key repeat for non-modifier repeatable keys
    let is_shortcut = (mods & (KDX_MOD_CTRL | KDX_MOD_ALT | KDX_MOD_SUPER)) != 0;
    let is_modifier_sym = (0xffe1..=0xffee).contains(&keysym);
    if !is_shortcut && !is_modifier_sym && d.repeat_rate > 0 {
        let delay = d.repeat_delay_ms.max(150) as u64;
        d.active_repeat = Some(RepeatKey {
            win_id,
            keysym,
            mods,
            utf8,
            next_fire: Instant::now() + Duration::from_millis(delay),
        });
    } else {
        d.active_repeat = None;
    }
    d.caret_next_blink = Some(Instant::now() + Duration::from_millis(530));
}

extern "C" fn cb_ime_commit(ud: *mut c_void, win_id: u64, utf8_ptr: *const c_char, utf8_len: usize) {
    if utf8_ptr.is_null() || utf8_len == 0 {
        return;
    }
    let d = unsafe { &mut *(ud as *mut Daemon) };
    let bytes = unsafe { std::slice::from_raw_parts(utf8_ptr as *const u8, utf8_len) };
    let text = String::from_utf8_lossy(bytes).into_owned();
    let autosave_ms = d.storage.config.autosave_ms;
    if let Some(w) = d.find_win_mut(win_id) {
        w.handle_ime_commit(&text, autosave_ms);
    }
}

extern "C" fn cb_pointer_motion(ud: *mut c_void, win_id: u64, x: c_double, y: c_double) {
    let d = unsafe { &mut *(ud as *mut Daemon) };
    let app = d.app;
    if let Some(w) = d.find_win_mut(win_id) {
        w.handle_pointer_motion(app, x, y);
    }
}

extern "C" fn cb_pointer_button(
    ud: *mut c_void,
    win_id: u64,
    button: u32,
    pressed: bool,
    mods: u32,
    x: c_double,
    y: c_double,
) {
    let d = unsafe { &mut *(ud as *mut Daemon) };
    let app = d.app;
    let autosave_ms = d.storage.config.autosave_ms;
    let action = if let Some(w) = d.find_win_mut(win_id) {
        w.handle_pointer_button(app, button, pressed, mods, x, y, autosave_ms)
    } else {
        ViewAction::None
    };
    d.apply_view_action(win_id, action);
}

extern "C" fn cb_pointer_scroll(ud: *mut c_void, win_id: u64, dx: c_double, dy: c_double, mods: u32) {
    let d = unsafe { &mut *(ud as *mut Daemon) };
    if let Some(w) = d.find_win_mut(win_id) {
        w.handle_scroll(dx, dy, mods);
    }
}

extern "C" fn cb_repeat_info(ud: *mut c_void, rate: i32, delay_ms: i32) {
    let d = unsafe { &mut *(ud as *mut Daemon) };
    d.repeat_rate = rate;
    d.repeat_delay_ms = delay_ms;
}

fn arm_timerfd(tfd: c_int, deadline: Option<Instant>) {
    let spec = match deadline {
        None => ITimerSpec {
            it_interval: TimeSpec { tv_sec: 0, tv_nsec: 0 },
            it_value: TimeSpec { tv_sec: 0, tv_nsec: 0 },
        },
        Some(t) => {
            let now = Instant::now();
            let dur = if t > now {
                t.duration_since(now)
            } else {
                Duration::from_millis(1)
            };
            ITimerSpec {
                it_interval: TimeSpec { tv_sec: 0, tv_nsec: 0 },
                it_value: TimeSpec {
                    tv_sec: dur.as_secs() as i64,
                    tv_nsec: (dur.subsec_nanos() as i64).max(1_000_000),
                },
            }
        }
    };
    unsafe { timerfd_settime(tfd, 0, &spec, std::ptr::null_mut()) };
}

pub fn run_daemon(initial_cmd: IpcCommand) -> Result<(), String> {
    let storage = Storage::open().map_err(|e| format!("storage init failed: {e}"))?;
    let ipc = IpcServer::bind(&storage.paths.socket_path)
        .map_err(|e| format!("failed to bind IPC socket: {e}"))?;

    unsafe {
        signal(SIGINT, handle_signal);
        signal(SIGTERM, handle_signal);
    }

    let dark = Daemon::detect_dark_theme(storage.config.theme);
    let mut daemon = Box::new(Daemon {
        app: std::ptr::null_mut(),
        storage,
        windows: Vec::new(),
        next_win_id: 1,
        dark,
        sticky_visible: false,
        running: true,
        repeat_rate: 25,
        repeat_delay_ms: 400,
        active_repeat: None,
        caret_next_blink: None,
    });

    let cbs = KdxCallbacks {
        userdata: (&mut *daemon as *mut Daemon) as *mut c_void,
        on_configure: Some(cb_configure),
        on_close: Some(cb_close),
        on_focus: Some(cb_focus),
        on_key: Some(cb_key),
        on_ime_commit: Some(cb_ime_commit),
        on_pointer_motion: Some(cb_pointer_motion),
        on_pointer_button: Some(cb_pointer_button),
        on_pointer_scroll: Some(cb_pointer_scroll),
        on_repeat_info: Some(cb_repeat_info),
    };

    let app = unsafe { kdx_app_new(&cbs) };
    if app.is_null() {
        return Err(String::from(
            "failed to connect to Wayland display (is WAYLAND_DISPLAY set?)",
        ));
    }
    daemon.app = app;

    let tfd = unsafe { timerfd_create(CLOCK_MONOTONIC, TFD_NONBLOCK | TFD_CLOEXEC) };
    if tfd < 0 {
        unsafe { kdx_app_destroy(app) };
        return Err(String::from("failed to create timerfd"));
    }

    // Execute initial CLI command
    let _ = daemon.handle_ipc_command(initial_cmd);

    let wl_fd = unsafe { kdx_app_get_fd(app) };
    let ipc_fd = ipc.raw_fd();

    // Pure blocking poll() event loop — 0.0% idle CPU
    while daemon.running && !QUIT_REQUESTED.load(Ordering::Relaxed) {
        daemon.paint_dirty();

        unsafe {
            while kdx_app_prepare_read(app) != 0 {
                if kdx_app_dispatch_pending(app) < 0 {
                    daemon.running = false;
                    break;
                }
            }
            if !daemon.running {
                kdx_app_cancel_read(app);
                break;
            }
            kdx_app_flush(app);
        }

        arm_timerfd(tfd, daemon.next_deadline());

        let mut fds = [
            PollFd { fd: wl_fd, events: POLLIN, revents: 0 },
            PollFd { fd: ipc_fd, events: POLLIN, revents: 0 },
            PollFd { fd: tfd, events: POLLIN, revents: 0 },
        ];

        let r = unsafe { poll(fds.as_mut_ptr(), fds.len() as u64, -1) };
        if r < 0 {
            unsafe { kdx_app_cancel_read(app) };
            continue;
        }

        if (fds[0].revents & POLLIN) != 0 {
            if unsafe { kdx_app_read_events(app) } < 0 {
                daemon.running = false;
                break;
            }
            if unsafe { kdx_app_dispatch_pending(app) } < 0 {
                daemon.running = false;
                break;
            }
        } else {
            unsafe { kdx_app_cancel_read(app) };
        }

        if (fds[1].revents & POLLIN) != 0 {
            ipc.accept_all(|cmd| daemon.handle_ipc_command(cmd));
        }

        if (fds[2].revents & POLLIN) != 0 {
            let mut expirations: u64 = 0;
            let _ = unsafe { read(tfd, (&mut expirations as *mut u64) as *mut c_void, 8) };
            daemon.process_timers();
        }
    }

    daemon.flush_all_sticky();
    unsafe {
        close(tfd);
        kdx_app_destroy(app);
    }
    Ok(())
}
