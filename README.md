# Kadex

**Kadex** (*Ka* from Karel + *codex*, Latin for notebook) is a single-binary, zero-bloat, native Rust **Notepad & Sticky Notes** app built for **Linux (Wayland / Niri)** and **Windows**.

No Electron. No WebView2. No XWayland. No GPU context. Pure native OS rendering with a shared zero-dependency Rust core.

---

## Measured Footprint (Linux / CachyOS + Niri)

Measured live on CachyOS + Niri (`WAYLAND_DISPLAY=wayland-1`, zero XWayland):

| Metric | Measured | PRD Target |
| :--- | :--- | :--- |
| **Private Dirty RAM (1 sticky note)** | **5.0 MB** (`4.3 MB` anon) | ≤ 10 MB *(stretch ≤ 5 MB)* |
| **Private Dirty RAM (5 sticky notes)** | **12.6 MB** *(untiled)* / **~9 MB** *(360×320 floating)* | ≤ 16 MB |
| **Cold Start to Ready** | **21 ms** | < 25 ms |
| **IPC `kadex toggle` Latency** | **14 ms** | < 15 ms |
| **Idle CPU Usage** | **0.0%** (`0` ticks / sec in `poll()`) | 0.0% |
| **Binary Size (stable Rust, stripped)** | **~485 KB** *(~170 KB with nightly `-Z build-std`)* | ≤ 350 KB / ≤ 200 KB |

---

## Features

- **Dual Mode:**
  - **Sticky Mode** (`app_id: kadex.sticky`): Compact 28 px header bar with 6 curated dark/light palettes, `+` new note, `A` archive, `×` hide, drag-to-move, and crash-safe debounced auto-save (500 ms + immediate flush on focus loss and `SIGTERM`/`SIGINT`/`WM_ENDSESSION`).
  - **File Mode** (`app_id: kadex.editor`): Tiles natively in Niri columns (respects `prefer-no-csd`), preserves UTF-8 / UTF-8 BOM / UTF-16 LE/BE and `LF`/`CRLF`, dirty `*` title indicator, and bottom status bar (`Ln/Col`, encoding, EOL, wrap, zoom).
  - **Overlay Scratchpad** (`kadex overlay`): Centered Wayland `zwlr_layer_shell_v1` top-layer scratchpad floating above all Niri workspaces; dismisses on `Esc`.
- **Editor Essentials:**
  - Full Unicode / UTF-8 + IME (`zwp_text_input_v3`), multi-level Undo/Redo (`Ctrl+Z`/`Ctrl+Y`), Find & Replace (`Ctrl+F`/`Ctrl+H`, `F3`/`Shift+F3`), Word Wrap (`Alt+Z`), Font Zoom (`Ctrl+=`/`-`/`0` & `Ctrl+Wheel`), Clipboard + Wayland Primary Selection (middle-click paste), Per-Monitor Fractional DPI (`wp_fractional_scale_v1` + `wp_viewporter` / Win32 DPI v2), Markdown checkbox toggle (`Ctrl+Enter` toggles `- [ ] ` ↔ `- [x] `).
  - Smooth handling of large (5 MB+) `.log` files via windowed viewport slicing.
- **Portable Mode:** Drop an empty `portable.ini` next to the `kadex` binary to store `config.ini`, `state.ini`, and `notes/` beside the executable.

---

## CLI & IPC Commands

```sh
kadex                  # Show sticky notes (or create one)
kadex path/to/file.txt # Open a file in File Mode
kadex new              # Spawn a new sticky note
kadex toggle           # Show / hide all sticky notes
kadex hide             # Hide all sticky notes
kadex list             # Print `id<TAB>first line` (pipe into fuzzel/rofi)
kadex open <id>        # Open / focus a sticky note by ID
kadex overlay          # Toggle layer-shell overlay scratchpad
kadex quit             # Flush all notes atomically and stop daemon
```

---

## Niri Integration

See [`packaging/niri/kadex.kdl`](packaging/niri/kadex.kdl):

```kdl
window-rule {
    match app-id=r#"^kadex\.sticky$"#
    open-floating true
    default-column-width { fixed 360; }
    default-window-height { fixed 320; }
    opacity 0.96
}

window-rule {
    match app-id=r#"^kadex\.editor$"#
    open-floating false
}

binds {
    Mod+Shift+N { spawn "kadex" "toggle"; }
    Mod+Alt+N   { spawn "kadex" "new"; }
    Mod+Ctrl+N  { spawn "kadex" "overlay"; }
    Mod+Shift+K { spawn "sh" "-c" "id=$(kadex list | fuzzel --dmenu --prompt='Kadex> ' | cut -f1); [ -n \"$id\" ] && kadex open \"$id\""; }
}
```

---

## Building

### Linux (Arch / CachyOS)

Dependencies (already installed on standard Wayland desktops):
`wayland`, `libxkbcommon`, `pango`, `cairo`, `glib2`, `pkgconf`, `rust`

```sh
cargo build --release
# Binary: target/release/kadex
```

Ultra-lean binary (~170 KB) using nightly `build-std`:
```sh
cargo +nightly build --release \
  -Z build-std=std,panic_abort \
  -Z build-std-features=panic_immediate_abort \
  --target x86_64-unknown-linux-gnu
```

Or build an Arch/CachyOS package:
```sh
cd packaging/arch && makepkg -si
```

### Windows

Download `kadex-<version>-windows-x64-setup.exe` from the [Releases](https://github.com/Aitemaa/Kadex/releases) page and run it. No admin rights are needed: it installs for your user into `%LOCALAPPDATA%\Programs\Kadex`. Choose "for all users" on the first page to install into Program Files instead. It adds:

- a Start menu entry, so Kadex shows up in Windows search
- optional: a desktop shortcut, starting at sign-in (keeps `Ctrl+Alt+N` ready), and an "Open with" entry for `.txt`, `.md`, and `.log` files
- an uninstaller in **Settings → Apps**. Uninstalling never deletes your notes in `%APPDATA%\kadex`.

The plain `kadex.exe` and a portable zip are attached to each release as well. To build it yourself:

```sh
cargo build --release --target x86_64-pc-windows-msvc
```

---

## Keyboard Shortcuts

| Shortcut | Action |
| :--- | :--- |
| `Ctrl+N` | New sticky note |
| `Ctrl+S` | Save file / flush sticky note immediately |
| `Ctrl+W` | Hide sticky note / close file window |
| `Ctrl+F` / `Ctrl+H` | Find / Find & Replace |
| `F3` / `Shift+F3` | Next / Previous match |
| `Alt+Z` | Toggle word wrap |
| `Ctrl+=` / `Ctrl+-` / `Ctrl+0` | Zoom in / out / reset *(or `Ctrl+Wheel`)* |
| `Ctrl+Enter` | Toggle markdown checkbox (`- [ ] ` ↔ `- [x] `) |
| `Ctrl+T` *(Windows)* | Toggle always-on-top pin |
| `Esc` | Close Find bar / dismiss Overlay scratchpad |

---

## Storage Layout

| Purpose | Linux (XDG) | Windows |
| :--- | :--- | :--- |
| Config | `$XDG_CONFIG_HOME/kadex/config.ini` | `%APPDATA%\kadex\config.ini` |
| Notes | `$XDG_DATA_HOME/kadex/notes/*.txt` | `%APPDATA%\kadex\notes\*.txt` |
| Archive | `$XDG_DATA_HOME/kadex/notes/archive/` | `%APPDATA%\kadex\notes\archive\` |
| State | `$XDG_STATE_HOME/kadex/state.ini` | `%APPDATA%\kadex\state.ini` |
| IPC | `$XDG_RUNTIME_DIR/kadex.sock` | Named Mutex + `WM_COPYDATA` |

---

## License

MIT — Ignatius Karel
