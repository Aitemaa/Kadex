//! Pure Win32 + RichEdit + DWM + Tray + Global Hotkey + Named Mutex backend for Kadex.

mod gdip;
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
        GetLastError, ERROR_ALREADY_EXISTS, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
    };
    use windows_sys::Win32::System::DataExchange::COPYDATASTRUCT;
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::System::Threading::CreateMutexW;
    use windows_sys::Win32::UI::HiDpi::{
        GetDpiForSystem, SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    };
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetKeyState, RegisterHotKey, UnregisterHotKey, MOD_ALT, MOD_CONTROL, VK_ADD, VK_CONTROL,
        VK_MENU, VK_OEM_MINUS, VK_OEM_PLUS, VK_SUBTRACT,
    };
    use windows_sys::Win32::UI::Shell::{
        Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        AllowSetForegroundWindow, AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW,
        DestroyMenu, DispatchMessageW, FindWindowExW, GetAncestor, GetCursorPos, GetMessageW,
        GetSystemMetrics, LoadIconW, LoadImageW, PostQuitMessage, RegisterClassW, SendMessageW,
        SetForegroundWindow, SetWindowPos, TrackPopupMenu, TranslateMessage, ASFW_ANY, EN_CHANGE,
        GA_ROOT, HICON, HWND_MESSAGE, IDI_APPLICATION, IMAGE_ICON, LR_DEFAULTCOLOR, MF_STRING, MSG,
        SM_CXICON, SM_CXSMICON, SM_CYICON, SM_CYSMICON, SWP_NOACTIVATE, SWP_NOZORDER,
        TPM_RETURNCMD, TPM_RIGHTBUTTON, WM_APP, WM_CLOSE, WM_COMMAND, WM_COPYDATA, WM_DESTROY,
        WM_DPICHANGED, WM_ENDSESSION, WM_HOTKEY, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP,
        WM_MOUSEMOVE, WM_PAINT, WM_RBUTTONUP, WM_SETTINGCHANGE, WM_SIZE, WM_TIMER, WNDCLASSW,
    };

    use crate::theme::{detect_dark_mode, to_wide_null};
    use crate::window::{
        create_note_window, install_ui_font, load_richedit_dll, remove_ui_font, window_from_hwnd,
        HeaderHit, NoteWindow, WinMode, NOTE_CLASS_NAME, TIMER_AUTOSAVE,
    };

    const MUTEX_NAME: &str = "Local\\KadexSingleInstanceMutex";
    const IPC_CLASS_NAME: &str = "KadexIpcHostWindow";
    const HOTKEY_TOGGLE_ID: i32 = 1;
    const WM_TRAY_ICON: u32 = WM_APP + 1;
    /// Sent by `TrackMouseEvent(TME_LEAVE)` when the pointer leaves the client area.
    const WM_MOUSELEAVE: u32 = 0x02A3;
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

    /// Resource ID of the app icon embedded by the root crate's build script.
    const APP_ICON_ID: usize = 1;

    /// Loads the embedded app icon at the system's large or small icon size, falling back to
    /// the stock application icon if the resource is missing (e.g. a cross-compiled build).
    unsafe fn app_icon(hinst: *mut c_void, small: bool) -> HICON {
        let (cx, cy) = if small {
            (GetSystemMetrics(SM_CXSMICON), GetSystemMetrics(SM_CYSMICON))
        } else {
            (GetSystemMetrics(SM_CXICON), GetSystemMetrics(SM_CYICON))
        };
        let icon = LoadImageW(
            hinst,
            APP_ICON_ID as *const u16,
            IMAGE_ICON,
            cx,
            cy,
            LR_DEFAULTCOLOR,
        );
        if icon.is_null() {
            LoadIconW(std::ptr::null_mut(), IDI_APPLICATION)
        } else {
            icon
        }
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
        // Let the running instance bring its window to the front.
        unsafe { AllowSetForegroundWindow(ASFW_ANY) };
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
        install_ui_font();

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
                hIcon: app_icon(hinst, false),
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
            nid.hIcon = app_icon(hinst, true);
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
                if msg.message == WM_KEYDOWN && handle_shortcut(&msg) {
                    continue;
                }
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }

            flush_all();
            UnregisterHotKey(ipc_hwnd, HOTKEY_TOGGLE_ID);
            Shell_NotifyIconW(NIM_DELETE, &nid);
            crate::gdip::shutdown();
            remove_ui_font();
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
                IpcCommand::Activate => activate(d),
                IpcCommand::NewSticky => spawn_sticky(d, None),
                IpcCommand::OpenNote(id) => spawn_sticky(d, Some(id)),
                IpcCommand::OpenFile(path) => open_file(d, &path),
                IpcCommand::Toggle => {
                    if d.windows.iter().any(|w| w.is_sticky() && w.is_visible()) {
                        hide_all_sticky(d);
                    } else {
                        activate(d);
                    }
                }
                IpcCommand::Hide => hide_all_sticky(d),
                IpcCommand::Overlay => spawn_sticky(d, Some(String::from("scratchpad"))),
                IpcCommand::Quit => unsafe { PostQuitMessage(0) },
                IpcCommand::List | IpcCommand::Ping => {}
            }
        });
    }

    /// Shows the sticky notes, restoring the ones left open last session (or creating a
    /// fresh note when there are none), and brings them to the front.
    fn activate(d: &mut WinDaemon) {
        if !d.windows.iter().any(|w| w.is_sticky()) {
            let open_ids: Vec<String> = d
                .storage
                .state
                .notes
                .values()
                .filter(|n| n.open && d.storage.paths.note_path(&n.id).exists())
                .map(|n| n.id.clone())
                .collect();
            if open_ids.is_empty() {
                spawn_sticky(d, None);
            } else {
                for id in open_ids {
                    spawn_sticky(d, Some(id));
                }
            }
            return;
        }
        for w in &d.windows {
            if w.is_sticky() {
                w.show(true);
            }
        }
        if let Some(w) = d.windows.iter().rev().find(|w| w.is_sticky()) {
            w.focus();
        }
        d.sticky_visible = true;
    }

    fn hide_all_sticky(d: &mut WinDaemon) {
        for w in &mut d.windows {
            if w.is_sticky() {
                flush_note(&mut d.storage, w);
                w.show(false);
            }
        }
        d.sticky_visible = false;
    }

    /// Scales a size in 96-DPI pixels to the system DPI (windows use physical pixels).
    fn scale_px(px: u32) -> i32 {
        let dpi = unsafe { GetDpiForSystem() }.max(96);
        (px as i64 * dpi as i64 / 96) as i32
    }

    fn spawn_sticky(d: &mut WinDaemon, note_id: Option<String>) {
        // Never open a second window on the same note: both would autosave over each other.
        if let Some(id) = &note_id {
            let existing = d
                .windows
                .iter()
                .find(|w| matches!(&w.mode, WinMode::Sticky { note_id } if note_id == id));
            if let Some(w) = existing {
                w.show(true);
                w.focus();
                d.sticky_visible = true;
                return;
            }
        }
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
                scale_px(120) + (d.windows.len() as i32) * scale_px(28),
                scale_px(120) + (d.windows.len() as i32) * scale_px(28),
                scale_px(d.storage.config.sticky_width),
                scale_px(d.storage.config.sticky_height),
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
            win.focus();
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
            scale_px(160),
            scale_px(120),
            scale_px(d.storage.config.editor_width),
            scale_px(d.storage.config.editor_height),
            0,
            d.dark,
            false,
            d.storage.config.word_wrap,
            d.storage.config.font_family.clone(),
            d.storage.config.font_size,
            1.0,
            &text,
        ) {
            win.focus();
            d.windows.push(win);
        }
    }

    fn flush_note(storage: &mut Storage, note: &mut NoteWindow) {
        note.cancel_autosave();
        let text = note.get_text();
        note.refresh_title(&text);
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

    fn on_theme_change() {
        DAEMON.with(|slot| {
            let Ok(mut guard) = slot.try_borrow_mut() else {
                return;
            };
            let Some(d) = guard.as_mut() else { return };
            let dark = detect_dark_mode(d.storage.config.theme);
            if dark != d.dark {
                d.dark = dark;
                for w in &mut d.windows {
                    w.dark = dark;
                    w.apply_theme();
                }
            }
        });
    }

    fn save_note_window(hwnd: HWND) {
        let Some(note) = (unsafe { window_from_hwnd(hwnd) }) else {
            return;
        };
        DAEMON.with(|slot| {
            if let Ok(mut guard) = slot.try_borrow_mut() {
                if let Some(d) = guard.as_mut() {
                    flush_note(&mut d.storage, note);
                }
            }
        });
    }

    /// Keystrokes go to the focused RichEdit child, not to the note window, so Ctrl shortcuts
    /// are picked out of the message loop before dispatch. Returns true when handled.
    unsafe fn handle_shortcut(msg: &MSG) -> bool {
        let ctrl = (GetKeyState(VK_CONTROL as i32) as u16 & 0x8000) != 0;
        let alt = (GetKeyState(VK_MENU as i32) as u16 & 0x8000) != 0;
        if !ctrl || alt || msg.hwnd.is_null() {
            return false;
        }
        let root = GetAncestor(msg.hwnd, GA_ROOT);
        let Some(note) = window_from_hwnd(root) else {
            return false;
        };
        let vk = msg.wParam as u16;
        match vk {
            0x4E => handle_command(IpcCommand::NewSticky), // Ctrl+N
            0x53 => save_note_window(root),                // Ctrl+S
            0x54 => note.toggle_pin(),                     // Ctrl+T
            0x57 => {
                // Ctrl+W
                SendMessageW(root, WM_CLOSE, 0, 0);
            }
            0x30 | 0x60 => note.reset_zoom(), // Ctrl+0
            _ if vk == VK_OEM_PLUS || vk == VK_ADD => note.zoom(10),
            _ if vk == VK_OEM_MINUS || vk == VK_SUBTRACT => note.zoom(-10),
            _ => return false,
        }
        true
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
                let rc = &*(lparam as *const RECT);
                SetWindowPos(
                    hwnd,
                    std::ptr::null_mut(),
                    rc.left,
                    rc.top,
                    rc.right - rc.left,
                    rc.bottom - rc.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                if let Some(note) = window_from_hwnd(hwnd) {
                    note.apply_font();
                    note.layout_children();
                }
                0
            }
            // Broadcasts reach top-level windows only, never the message-only IPC window.
            WM_SETTINGCHANGE => {
                on_theme_change();
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_ENDSESSION => {
                if wparam != 0 {
                    flush_all();
                }
                0
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
            WM_MOUSEMOVE => {
                if let Some(note) = window_from_hwnd(hwnd) {
                    note.on_mouse_move(POINT {
                        x: (lparam & 0xFFFF) as i16 as i32,
                        y: ((lparam >> 16) & 0xFFFF) as i16 as i32,
                    });
                }
                0
            }
            WM_MOUSELEAVE => {
                if let Some(note) = window_from_hwnd(hwnd) {
                    note.on_mouse_leave();
                }
                0
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
                                        flush_note(&mut d.storage, note);
                                        let _ = d.storage.archive_note(&id);
                                        d.windows.retain(|w| w.hwnd != hwnd);
                                    }
                                });
                            }
                        }
                        HeaderHit::Close => {
                            SendMessageW(hwnd, WM_CLOSE, 0, 0);
                        }
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

/// The release build is a GUI-subsystem app with no console, so CLI output (`--help`, `list`)
/// would vanish. Attach to the console of the terminal that launched us, if there is one.
#[cfg(target_os = "windows")]
pub fn attach_parent_console() {
    use windows_sys::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
    unsafe {
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}
