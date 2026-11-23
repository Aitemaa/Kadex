# Kadex — PRD v2: Native Rust Notepad & Sticky Notes (Windows + Linux / Wayland / Niri)

## 1. Product Overview

**Kadex** (*Ka* from Karel + *codex*, Latin for notebook) is a single-binary, zero-bloat, native Rust Notepad & Sticky Notes application with a shared platform-agnostic core and thin native OS frontends:
- **Windows:** Pure Win32 + `MSFTEDIT_CLASS` (RichEdit) + GDI + DWM — no WebView2, no XAML, no GPU context.
- **Linux (Wayland / Niri):** Pure native Wayland (`wl_shm` software rendering + Pango/Cairo + `xkbcommon`, zero XWayland, zero GPU context) designed as a first-class citizen in scrollable-tiling compositors via distinct `app_id`s (`kadex.sticky` / `kadex.editor`), instant CLI-over-Unix-socket IPC, and optional `zwlr_layer_shell_v1` overlay scratchpad mode.

## 2. Hard Performance & Footprint Budgets

| Metric | Windows Target (MVP / Stretch) | Linux / Wayland Target (MVP / Stretch) |
| :--- | :--- | :--- |
| **Private RAM (1 note)** | ≤ 6 MB / ≤ 3 MB | ≤ 10 MB / ≤ 5 MB |
| **Private RAM (5 sticky notes)** | ≤ 12 MB / ≤ 6 MB | ≤ 16 MB / ≤ 8 MB |
| **Binary Size** | ≤ 250 KB / ≤ 128 KB | ≤ 350 KB / ≤ 200 KB *(dynamically linked to OS libs)* |
| **Cold Start to Caret** | < 30 ms / < 15 ms | < 25 ms / < 15 ms |
| **Idle CPU Usage** | 0.0% | 0.0% |
| **Display Protocol** | Native Win32 + DWM | Native Wayland (zero XWayland) |

## 3. Workspace Architecture

```text
Kadex/
├── crates/
│   ├── core/          # 100% OS-agnostic, zero external deps
│   │   ├── buffer.rs  # UTF-8 gap buffer + multi-level undo/redo + checkbox toggle
│   │   ├── storage.rs # UTF-8/UTF-16/EOL detection, atomic save, notes/archive, portable.ini
│   │   ├── config.rs  # Hand-rolled INI parser/writer + 6 dark/light palettes
│   │   ├── search.rs  # Plain-text case-sensitive/insensitive Find & Replace
│   │   └── ipc.rs     # Cross-platform line-based IPC protocol
│   ├── platform-wl/   # Pure Wayland (wl_shm + xdg-shell + xdg-decoration + fractional-scale + viewporter + primary-selection + text-input-v3 + wlr-layer-shell + Pango/Cairo)
│   └── platform-win/  # Pure Win32 (windows-sys + Msftedit.dll + GDI + DWM + Named Mutex + WM_COPYDATA + Tray + Global Hotkey)
├── packaging/
│   ├── arch/PKGBUILD
│   ├── niri/kadex.kdl
│   └── kadex.desktop
└── src/main.rs        # Thin CLI dispatcher + single-instance entrypoint
```

## 4. CLI & IPC Interface

- `kadex` — show existing sticky notes (or create one)
- `kadex <file>` — open a plain-text file in File Mode (`kadex.editor`)
- `kadex new` — spawn a new sticky note (`kadex.sticky`)
- `kadex toggle` — show/hide all sticky notes
- `kadex hide` — hide all sticky notes
- `kadex list` — print `id\tfirst line` to stdout (for `fuzzel --dmenu` / `rofi` / `walker`)
- `kadex open <id>` — open/focus a specific sticky note
- `kadex overlay` — toggle centered `zwlr_layer_shell_v1` overlay scratchpad
- `kadex quit` — flush dirty notes atomically and exit daemon
