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
│   │   ├── config.rs  # Hand-rolled INI parser/writer + 6 dark/light palettes (Kavox design tokens)
│   │   ├── icons.rs   # Lucide-style stroked icon paths (24 grid), shared by both frontends
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

## 5. Visual Design (Kavox family)

Kadex belongs to the same product family as Kavox and Kapix and copies its design system rather than inventing one:

- **Palettes:** 6 palettes x dark/light, indexed `0..5` (stored as `palette = N`, indices never change meaning): **Graphite** (Kavox neutral tokens verbatim), **Amber**, **Sage**, **Cobalt**, **Rose**, **Iris** (same lightness steps with a faint hue wash; body text >= 7:1, secondary text >= 4.5:1 in every palette). Note body = Kavox `panel`, header/status bar = `surface`, hover = `surface_2`, hairlines = `line`.
- **Caret and selection:** caret is Kavox red `#E8262D` in every palette; selection is `lit` at alpha 56 (dark) / 40 (light).
- **Chrome typography:** Archivo Medium (SIL OFL 1.1, Latin subset, ~23 KB) is embedded and used for header title, find bar and status bar only. Note text keeps the user's `font_family` / `font_size` (default `monospace`, `Consolas` on Windows). Linux registers the font with fontconfig for this process only (written once to `$XDG_RUNTIME_DIR/kadex/`); Windows uses `AddFontMemResourceEx`. If loading fails, the UI falls back to the system sans-serif (Segoe UI on Windows).
- **Chrome:** 28 px header (`surface`, 1 px `line` bottom border), 24 px icon buttons (radius 7, hover `surface_2`, danger hover = red tint + `red_text`, active/pinned = `lit` fill), stroked lucide icons (stroke 2/24, round caps), palette dot = accent fill with a `line_strong` ring. The Wayland overlay is a `panel` card, radius 20, with a soft shadow.
- **Binary size:** the design system costs about +31 KB on Linux (embedded font, icon paths, shadow/clip helpers): ~517 KB for the stable release build (was ~486 KB). The stable build was already above the 350 KB MVP budget in section 2; the nightly `build-std` build remains the route to it.
