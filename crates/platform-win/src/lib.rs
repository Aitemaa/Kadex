//! Pure Win32 + RichEdit + DWM + Tray + Global Hotkey + Named Mutex backend for Kadex.

mod theme;
#[cfg(target_os = "windows")]
mod window;

#[cfg(not(target_os = "windows"))]
use kadex_core::ipc::IpcCommand;

#[cfg(not(target_os = "windows"))]
pub fn try_send_ipc(_cmd: &IpcCommand) -> Option<String> {
    None
}

#[cfg(not(target_os = "windows"))]
pub fn run_daemon(_initial_cmd: IpcCommand) -> Result<(), String> {
    Err(String::from("platform-win is only available on Windows"))
}

#[cfg(target_os = "windows")]
mod win_impl {
    use std::cell::RefCell;
    use std::ffi::c_void;
    use std::path::PathBuf;

    use kadex_core::config::NoteWindowState;
    use kadex_core::ipc::IpcCommand;
    use kadex_core::storage::{load_text_file, save_text_file, Encoding, LineEnding, Storage};

    use windows_sys::Win32::Foundation::{
        GetLastError, ERROR_ALREADY_EXISTS, HWND, LPARAM, LRESULT, POINT, WPARAM,
    };
    use windows_sys::Win32::System::DataExchange::COPYDATASTRUCT;
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::System::Threading::CreateMutexW;
    use windows_sys::Win32::UI::HiDpi::{
        SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    };
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetKeyState, RegisterHotKey, UnregisterHotKey, MOD_ALT, MOD_CONTROL, VK_CONTROL,
    };
    use windows_sys::Win32::UI::Shell::{
        Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu,
        DispatchMessageW, FindWindowExW, GetCursorPos, GetMessageW, LoadIconW, PostQuitMessage,
        RegisterClassW, SendMessageW, SetForegroundWindow, TrackPopupMenu, TranslateMessage,
        EN_CHANGE, HWND_MESSAGE, IDI_APPLICATION, MF_STRING, MSG, TPM_RETURNCMD, TPM_RIGHTBUTTON,
        WM_APP, WM_CLOSE, WM_COMMAND, WM_COPYDATA, WM_DESTROY, WM_DPICHANGED, WM_ENDSESSION,
        WM_HOTKEY, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_PAINT, WM_RBUTTONUP,
        WM_SETTINGCHANGE, WM_SIZE, WM_TIMER, WNDCLASSW,
    };

    use crate::theme::{detect_dark_mode, to_wide_null};
    use crate::window::{
        create_note_window, load_richedit_dll, window_from_hwnd, HeaderHit, NoteWindow, WinMode,
        NOTE_CLASS_NAME, TIMER_AUTOSAVE,
    };

    const MUTEX_NAME: &str = "Local\\KadexSingleInstanceMutex";
    const IPC_CLASS_NAME: &str = "KadexIpcHostWindow";
    const HOTKEY_TOGGLE_ID: i32 = 1;
    const WM_TRAY_ICON: u32 = WM_APP + 1;
    const COPYDATA_MAGIC: usize = 0x4B44_5831; // "KDX1"

    struct WinDaemon {
        storage: Storage,
        dark: bool,
        sticky_visible: bool,
        windows: Vec<Box<NoteWindow>>,
    }

    thread_local! {
        static DAEMON: RefCell<Option<WinDaemon>> = const { RefCell::new(None) };
    }

    pub fn try_send_ipc(cmd: &IpcCommand) -> Option<String> {
        let class_w = to_wide_null(IPC_CLASS_NAME);
        // Message-only windows are only visible to FindWindowExW with HWND_MESSAGE as the parent.
        let target = unsafe {
            FindWindowExW(
                HWND_MESSAGE,
                std::ptr::null_mut(),
                class_w.as_ptr(),
                std::ptr::null(),
            )
        };
        if target.is_null() {
            return None;
        }
        let wire = cmd.encode();
        let cds = COPYDATASTRUCT {
            dwData: COPYDATA_MAGIC,
            cbData: wire.len() as u32,
            lpData: wire.as_ptr() as *mut c_void,
        };
        unsafe {
            SendMessageW(
                target,
                WM_COPYDATA,
                0,
                (&cds as *const COPYDATASTRUCT) as LPARAM,
            );
        }
        Some(String::from("OK\n"))
    }

    pub fn run_daemon(initial_cmd: IpcCommand) -> Result<(), String> {
        unsafe {
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        }
        load_richedit_dll();

        let mutex_w = to_wide_null(MUTEX_NAME);
        let _mutex = unsafe { CreateMutexW(std::ptr::null(), 1, mutex_w.as_ptr()) };
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            let _ = try_send_ipc(&initial_cmd);
            return Ok(());
        }

        let storage = Storage::open().map_err(|e| format!("storage init failed: {e}"))?;
        let dark = detect_dark_mode(storage.config.theme);

        unsafe {
            let hinst = GetModuleHandleW(std::ptr::null());

            let ipc_cls = to_wide_null(IPC_CLASS_NAME);
            let wc_ipc = WNDCLASSW {
                lpfnWndProc: Some(ipc_wnd_proc),
                hInstance: hinst,
                lpszClassName: ipc_cls.as_ptr(),
                ..std::mem::zeroed()
            };
            RegisterClassW(&wc_ipc);

            let note_cls = to_wide_null(NOTE_CLASS_NAME);
            let wc_note = WNDCLASSW {
                lpfnWndProc: Some(note_wnd_proc),
                hInstance: hinst,
                lpszClassName: note_cls.as_ptr(),
                ..std::mem::zeroed()
            };
            RegisterClassW(&wc_note);

            let ipc_hwnd = CreateWindowExW(
                0,
                ipc_cls.as_ptr(),
                std::ptr::null(),
                0,
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                std::ptr::null_mut(),
                hinst,
                std::ptr::null(),
            );

            // Global hotkey: Ctrl+Alt+N toggles sticky notes
            RegisterHotKey(
                ipc_hwnd,
                HOTKEY_TOGGLE_ID,
                MOD_CONTROL | MOD_ALT,
                b'N' as u32,
            );

            // Lightweight system tray icon
            let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
            nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
            nid.hWnd = ipc_hwnd;
            nid.uID = 1;
            nid.uFlags = NIF_MESSAGE | NIF_TIP | NIF_ICON;
            nid.hIcon = LoadIconW(std::ptr::null_mut(), IDI_APPLICATION);
            nid.uCallbackMessage = WM_TRAY_ICON;
            let tip = to_wide_null("Kadex — Sticky Notes & Notepad");
            for (dst, &src) in nid.szTip.iter_mut().zip(tip.iter()) {
                *dst = src;
            }
            Shell_NotifyIconW(NIM_ADD, &nid);

            DAEMON.with(|slot| {
                *slot.borrow_mut() = Some(WinDaemon {
                    storage,
                    dark,
                    sticky_visible: true,
                    windows: Vec::new(),
                });
            });

            handle_command(initial_cmd);

            // Pure blocking Win32 GetMessageW loop — 0.0% idle CPU
            let mut msg: MSG = std::mem::zeroed();
            while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }

            flush_all();
            UnregisterHotKey(ipc_hwnd, HOTKEY_TOGGLE_ID);
            Shell_NotifyIconW(NIM_DELETE, &nid);
        }

        Ok(())
    }

    fn handle_command(cmd: IpcCommand) {
        DAEMON.with(|slot| {
            // A Win32 call made while handling a command can re-enter the window procedure,
            // so never assume the daemon is free to borrow.
            let Ok(mut guard) = slot.try_borrow_mut() else {
                return;
            };
            let Some(d) = guard.as_mut() else { return };
            match cmd {
                IpcCommand::Activate => {
                    if d.windows.iter().any(|w| w.is_sticky()) {
                        for w in &d.windows {
                            if w.is_sticky() {
                                w.show(true);
                            }
                        }
                        d.sticky_visible = true;
                    } else {
                        spawn_sticky(d, None);
                    }
                }
                IpcCommand::NewSticky => spawn_sticky(d, None),
                IpcCommand::OpenNote(id) => spawn_sticky(d, Some(id)),
                IpcCommand::OpenFile(path) => open_file(d, &path),
                IpcCommand::Toggle => {
                    d.sticky_visible = !d.sticky_visible;
                    for w in &d.windows {
                        if w.is_sticky() {
                            w.show(d.sticky_visible);
                        }
                    }
                }
                IpcCommand::Hide => {
                    d.sticky_visible = false;
                    for w in &d.windows {
                        if w.is_sticky() {
                            w.show(false);
                        }
                    }
                }
                IpcCommand::Overlay => spawn_sticky(d, Some(String::from("scratchpad"))),
                IpcCommand::Quit => unsafe { PostQuitMessage(0) },
                IpcCommand::List | IpcCommand::Ping => {}
            }
        });
    }

    fn spawn_sticky(d: &mut WinDaemon, note_id: Option<String>) {
        let id = note_id.unwrap_or_else(|| d.storage.new_note_id());
        let text = d.storage.load_note(&id).unwrap_or_default();
        let saved = d.storage.state.notes.get(&id).cloned();
        let (x, y, w, h, pal, pinned) = match saved {
            Some(s) => (
                s.x,
                s.y,
                s.width as i32,
                s.height as i32,
                s.palette,
                s.pinned,
            ),
            None => (
                120 + (d.windows.len() as i32) * 28,
                120 + (d.windows.len() as i32) * 28,
                d.storage.config.sticky_width as i32,
                d.storage.config.sticky_height as i32,
                d.storage.config.default_palette,
                false,
            ),
        };
        if let Some(win) = create_note_window(
            WinMode::Sticky {
                note_id: id.clone(),
            },
            "Kadex Note",
            x,
            y,
            w,
            h,
            pal,
            d.dark,
            pinned,
            d.storage.config.word_wrap,
            d.storage.config.font_family.clone(),
            d.storage.config.font_size,
            d.storage.config.opacity,
            &text,
        ) {
            let _ = d.storage.save_note(&id, &text);
            d.storage
                .state
                .notes
                .entry(id.clone())
                .or_insert_with(|| NoteWindowState::new(id, pal, w as u32, h as u32));
            let _ = d.storage.save_state();
            d.windows.push(win);
            d.sticky_visible = true;
        }
    }

    fn open_file(d: &mut WinDaemon, path_str: &str) {
        let path = PathBuf::from(path_str);
        let (text, encoding, line_ending) = if path.exists() {
            match load_text_file(&path) {
                Ok(l) => (l.text, l.encoding, l.line_ending),
                Err(_) => (String::new(), Encoding::Utf8, LineEnding::CrLf),
            }
        } else {
            (String::new(), Encoding::Utf8, LineEnding::CrLf)
        };
        let title = format!(
            "{} — Kadex",
            path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("Untitled")
        );
        if let Some(win) = create_note_window(
            WinMode::File {
                path,
                encoding,
                line_ending,
            },
            &title,
            160,
            120,
            d.storage.config.editor_width as i32,
            d.storage.config.editor_height as i32,
            0,
            d.dark,
            false,
            d.storage.config.word_wrap,
            d.storage.config.font_family.clone(),
            d.storage.config.font_size,
            1.0,
            &text,
        ) {
            d.windows.push(win);
        }
    }

    fn flush_note(storage: &mut Storage, note: &mut NoteWindow) {
        note.cancel_autosave();
        let text = note.get_text();
        match &note.mode {
            WinMode::Sticky { note_id } => {
                let _ = storage.save_note(note_id, &text);
                let (x, y, w, h) = note.rect();
                storage.state.notes.insert(
                    note_id.clone(),
                    NoteWindowState {
                        id: note_id.clone(),
                        x,
                        y,
                        width: w,
                        height: h,
                        palette: note.palette_idx,
                        pinned: note.pinned,
                        open: true,
                    },
                );
                let _ = storage.save_state();
            }
            WinMode::File {
                path,
                encoding,
                line_ending,
            } => {
                if note.dirty {
                    let _ = save_text_file(path, &text, *encoding, *line_ending);
                }
            }
        }
        note.dirty = false;
    }

    fn flush_all() {
        DAEMON.with(|slot| {
            if let Some(d) = slot
                .try_borrow_mut()
                .ok()
                .as_deref_mut()
                .and_then(Option::as_mut)
            {
                for w in &mut d.windows {
                    flush_note(&mut d.storage, w);
                }
            }
        });
    }

    const TRAY_CMD_NEW: usize = 1;
    const TRAY_CMD_TOGGLE: usize = 2;
    const TRAY_CMD_QUIT: usize = 3;

    unsafe fn show_tray_menu(hwnd: HWND) {
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return;
        }
        let new_w = to_wide_null("New note");
        let toggle_w = to_wide_null("Show / hide notes");
        let quit_w = to_wide_null("Quit Kadex");
        AppendMenuW(menu, MF_STRING, TRAY_CMD_NEW, new_w.as_ptr());
        AppendMenuW(menu, MF_STRING, TRAY_CMD_TOGGLE, toggle_w.as_ptr());
        AppendMenuW(menu, MF_STRING, TRAY_CMD_QUIT, quit_w.as_ptr());
        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            pt.x,
            pt.y,
            0,
            hwnd,
            std::ptr::null(),
        );
        DestroyMenu(menu);
        match cmd as usize {
            TRAY_CMD_NEW => handle_command(IpcCommand::NewSticky),
            TRAY_CMD_TOGGLE => handle_command(IpcCommand::Toggle),
            TRAY_CMD_QUIT => handle_command(IpcCommand::Quit),
            _ => {}
        }
    }

    unsafe extern "system" fn ipc_wnd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            WM_COPYDATA => {
                let cds = &*(lparam as *const COPYDATASTRUCT);
                if cds.dwData == COPYDATA_MAGIC && !cds.lpData.is_null() && cds.cbData > 0 {
                    let bytes =
                        std::slice::from_raw_parts(cds.lpData as *const u8, cds.cbData as usize);
                    if let Ok(s) = std::str::from_utf8(bytes) {
                        if let Some(cmd) = IpcCommand::decode(s) {
                            handle_command(cmd);
                        }
                    }
                }
                1
            }
            WM_HOTKEY => {
                if wparam as i32 == HOTKEY_TOGGLE_ID {
                    handle_command(IpcCommand::Toggle);
                }
                0
            }
            WM_TRAY_ICON => {
                match lparam as u32 {
                    WM_LBUTTONUP => handle_command(IpcCommand::Toggle),
                    WM_RBUTTONUP => show_tray_menu(hwnd),
                    _ => {}
                }
                0
            }
            WM_SETTINGCHANGE => {
                DAEMON.with(|slot| {
                    if let Some(d) = slot
                        .try_borrow_mut()
                        .ok()
                        .as_deref_mut()
                        .and_then(Option::as_mut)
                    {
                        let dark = detect_dark_mode(d.storage.config.theme);
                        if dark != d.dark {
                            d.dark = dark;
                            for w in &mut d.windows {
                                w.dark = dark;
                                w.apply_theme();
                            }
                        }
                    }
                });
                0
            }
            WM_ENDSESSION => {
                flush_all();
                0
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }

    unsafe extern "system" fn note_wnd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            WM_SIZE => {
                if let Some(note) = window_from_hwnd(hwnd) {
                    note.layout_children();
                }
                0
            }
            WM_DPICHANGED => {
                if let Some(note) = window_from_hwnd(hwnd) {
                    note.apply_font();
                    note.layout_children();
                }
                0
            }
            WM_KEYDOWN => {
                let ctrl = (GetKeyState(VK_CONTROL as i32) as u16 & 0x8000) != 0;
                if ctrl {
                    if let Some(note) = window_from_hwnd(hwnd) {
                        match wparam as u32 {
                            0x54 => note.toggle_pin(), // Ctrl+T
                            0xBB => note.zoom(10),     // Ctrl+=
                            0xBD => note.zoom(-10),    // Ctrl+-
                            0x30 => note.reset_zoom(), // Ctrl+0
                            0x53 => {
                                // Ctrl+S
                                DAEMON.with(|slot| {
                                    if let Some(d) = slot
                                        .try_borrow_mut()
                                        .ok()
                                        .as_deref_mut()
                                        .and_then(Option::as_mut)
                                    {
                                        flush_note(&mut d.storage, note);
                                    }
                                });
                            }
                            _ => {}
                        }
                    }
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_PAINT => {
                if let Some(note) = window_from_hwnd(hwnd) {
                    if note.is_sticky() {
                        note.paint_header();
                        return 0;
                    }
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_LBUTTONDOWN => {
                if let Some(note) = window_from_hwnd(hwnd) {
                    let pt = POINT {
                        x: (lparam & 0xFFFF) as i16 as i32,
                        y: ((lparam >> 16) & 0xFFFF) as i16 as i32,
                    };
                    match note.hit_test_header(pt) {
                        HeaderHit::New => handle_command(IpcCommand::NewSticky),
                        HeaderHit::Palette(idx) => {
                            note.palette_idx = idx;
                            note.apply_theme();
                        }
                        HeaderHit::Pin => note.toggle_pin(),
                        HeaderHit::Archive => {
                            if let WinMode::Sticky { note_id } = &note.mode {
                                let id = note_id.clone();
                                DAEMON.with(|slot| {
                                    if let Some(d) = slot
                                        .try_borrow_mut()
                                        .ok()
                                        .as_deref_mut()
                                        .and_then(Option::as_mut)
                                    {
                                        let _ = d.storage.archive_note(&id);
                                        d.windows.retain(|w| w.hwnd != hwnd);
                                    }
                                });
                            }
                        }
                        HeaderHit::Close => note.show(false),
                        HeaderHit::Drag => note.begin_drag(),
                        HeaderHit::None => {}
                    }
                    return 0;
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_COMMAND => {
                let code = ((wparam >> 16) & 0xFFFF) as u32;
                if code == EN_CHANGE {
                    if let Some(note) = window_from_hwnd(hwnd) {
                        note.dirty = true;
                        DAEMON.with(|slot| {
                            if let Some(d) =
                                slot.try_borrow().ok().as_deref().and_then(Option::as_ref)
                            {
                                note.schedule_autosave(d.storage.config.autosave_ms);
                            }
                        });
                    }
                }
                0
            }
            WM_TIMER => {
                if wparam == TIMER_AUTOSAVE {
                    if let Some(note) = window_from_hwnd(hwnd) {
                        DAEMON.with(|slot| {
                            if let Some(d) = slot
                                .try_borrow_mut()
                                .ok()
                                .as_deref_mut()
                                .and_then(Option::as_mut)
                            {
                                flush_note(&mut d.storage, note);
                            }
                        });
                    }
                }
                0
            }
            WM_CLOSE => {
                if let Some(note) = window_from_hwnd(hwnd) {
                    let is_sticky = note.is_sticky();
                    DAEMON.with(|slot| {
                        if let Some(d) = slot
                            .try_borrow_mut()
                            .ok()
                            .as_deref_mut()
                            .and_then(Option::as_mut)
                        {
                            flush_note(&mut d.storage, note);
                            if is_sticky {
                                note.show(false);
                            } else {
                                d.windows.retain(|w| w.hwnd != hwnd);
                                if d.windows.is_empty() {
                                    PostQuitMessage(0);
                                }
                            }
                        }
                    });
                    return 0;
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_DESTROY => 0,
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

#[cfg(target_os = "windows")]
pub use win_impl::{run_daemon, try_send_ipc};
