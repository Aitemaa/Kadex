//! Offscreen snapshots of the real drawing code (no Wayland connection, no window).
//!
//! `KADEX_SNAPSHOT_DIR=/some/dir cargo test -p kadex-platform-wl snapshots -- --nocapture`
//! renders the sticky note, file mode status bar, find bar and overlay for several palettes into
//! PNG files. Without the variable the test is a no-op, so plain `cargo test` writes nothing.

use std::ffi::{c_char, c_double, c_int, c_void, CString};
use std::path::Path;

use kadex_core::storage::{Encoding, LineEnding};

use crate::editor_view::{EditorWindow, HdrBtn, WindowMode, OVERLAY_INSET};
use crate::fonts;

extern "C" {
    fn cairo_image_surface_create(format: c_int, w: c_int, h: c_int) -> *mut c_void;
    fn cairo_create(surf: *mut c_void) -> *mut c_void;
    fn cairo_destroy(cr: *mut c_void);
    fn cairo_surface_destroy(surf: *mut c_void);
    fn cairo_scale(cr: *mut c_void, sx: c_double, sy: c_double);
    fn cairo_translate(cr: *mut c_void, tx: c_double, ty: c_double);
    fn cairo_set_source_rgb(cr: *mut c_void, r: c_double, g: c_double, b: c_double);
    fn cairo_set_source_surface(cr: *mut c_void, s: *mut c_void, x: c_double, y: c_double);
    fn cairo_paint(cr: *mut c_void);
    fn cairo_surface_write_to_png(surf: *mut c_void, path: *const c_char) -> c_int;
}

const FORMAT_ARGB32: c_int = 0;
const NOTE: &str = "Groceries and plans\n\nMilk, eggs, and a loaf of rye bread\n- [x] Call the dentist\n- [ ] Book train tickets for Friday\n\nThe quick brown fox jumps over the lazy dog.";

struct Shot {
    surf: *mut c_void,
    w: i32,
    h: i32,
}

impl Shot {
    fn save(&self, path: &Path) {
        let c = CString::new(path.to_str().unwrap()).unwrap();
        assert_eq!(
            unsafe { cairo_surface_write_to_png(self.surf, c.as_ptr()) },
            0
        );
    }
}

impl Drop for Shot {
    fn drop(&mut self) {
        unsafe { cairo_surface_destroy(self.surf) };
    }
}

fn window(mode: WindowMode, w: i32, h: i32, pal: usize, dark: bool, text: &str) -> EditorWindow {
    let mut win = EditorWindow::new(
        1,
        mode,
        std::ptr::null_mut(),
        text,
        w,
        h,
        pal,
        dark,
        String::from("monospace"),
        11.5,
        true,
        1.0,
    );
    win.focused = true;
    win.caret_visible = true;
    win
}

/// Renders `win` at `scale` x on an opaque `backdrop` (what the compositor would show behind it).
fn render(win: &mut EditorWindow, scale: f64, backdrop: (f64, f64, f64)) -> Shot {
    let (w, h) = (win.width, win.height);
    let (pw, ph) = ((w as f64 * scale) as i32, (h as f64 * scale) as i32);
    unsafe {
        let surf = cairo_image_surface_create(FORMAT_ARGB32, pw, ph);
        let cr = cairo_create(surf);
        cairo_set_source_rgb(cr, backdrop.0, backdrop.1, backdrop.2);
        cairo_paint(cr);
        cairo_scale(cr, scale, scale);
        win.draw(cr, w as f64, h as f64);
        cairo_destroy(cr);
        Shot { surf, w: pw, h: ph }
    }
}

fn sticky(pal: usize, dark: bool) -> EditorWindow {
    let mut win = window(
        WindowMode::Sticky {
            note_id: "x".into(),
        },
        360,
        320,
        pal,
        dark,
        NOTE,
    );
    let start = NOTE.find("brown").unwrap();
    win.buf.select_range(start, start + "brown fox jumps".len());
    win
}

fn montage(shots: &[Shot], cols: usize, gap: i32, bg: (f64, f64, f64)) -> Shot {
    let cw = shots.iter().map(|s| s.w).max().unwrap();
    let ch = shots.iter().map(|s| s.h).max().unwrap();
    let rows = shots.len().div_ceil(cols) as i32;
    let (tw, th) = (cols as i32 * (cw + gap) + gap, rows * (ch + gap) + gap);
    unsafe {
        let surf = cairo_image_surface_create(FORMAT_ARGB32, tw, th);
        let cr = cairo_create(surf);
        cairo_set_source_rgb(cr, bg.0, bg.1, bg.2);
        cairo_paint(cr);
        for (i, s) in shots.iter().enumerate() {
            let (x, y) = (
                (i % cols) as i32 * (cw + gap) + gap,
                (i / cols) as i32 * (ch + gap) + gap,
            );
            cairo_translate(cr, x as f64, y as f64);
            cairo_set_source_surface(cr, s.surf, 0.0, 0.0);
            cairo_paint(cr);
            cairo_translate(cr, -x as f64, -y as f64);
        }
        cairo_destroy(cr);
        Shot { surf, w: tw, h: th }
    }
}

#[test]
fn snapshots() {
    let Some(dir) = std::env::var_os("KADEX_SNAPSHOT_DIR") else {
        return;
    };
    let dir = Path::new(&dir);
    std::fs::create_dir_all(dir).unwrap();
    let have_font = fonts::register_ui_font();
    eprintln!("embedded UI font registered: {have_font}");

    let dark_bg = (0.08, 0.08, 0.08);
    let light_bg = (0.80, 0.79, 0.76);

    // All six palettes, sticky note, dark and light (1x, as on screen).
    for (dark, name, bg) in [(true, "dark", dark_bg), (false, "light", light_bg)] {
        let shots: Vec<Shot> = (0..6)
            .map(|p| {
                let mut w = sticky(p, dark);
                // hover states on a few so they can be inspected
                w.hover = match p {
                    0 => Some(HdrBtn::Close),
                    1 => Some(HdrBtn::Archive),
                    2 => Some(HdrBtn::New),
                    3 => Some(HdrBtn::Dot(4)),
                    _ => None,
                };
                render(&mut w, 1.0, bg)
            })
            .collect();
        montage(&shots, 3, 16, bg).save(&dir.join(format!("sticky-all-{name}.png")));
    }

    // Detail crops at 2x: graphite dark/light with hovers, amber and cobalt.
    for (pal, dark, hover, file) in [
        (
            0,
            true,
            Some(HdrBtn::Close),
            "sticky-graphite-dark-close-hover@2x.png",
        ),
        (
            0,
            false,
            Some(HdrBtn::Archive),
            "sticky-graphite-light-archive-hover@2x.png",
        ),
        (
            1,
            true,
            Some(HdrBtn::New),
            "sticky-amber-dark-new-hover@2x.png",
        ),
        (3, false, None, "sticky-cobalt-light@2x.png"),
    ] {
        let mut w = sticky(pal, dark);
        w.hover = hover;
        render(&mut w, 2.0, if dark { dark_bg } else { light_bg }).save(&dir.join(file));
    }

    // Long first line: the header title is ellipsised; narrow note drops it entirely.
    let long = "A very long first line that can never fit into the header bar\nBody text";
    let mut shots = Vec::new();
    for (w, dark) in [(360, true), (360, false), (230, true)] {
        let mut win = window(
            WindowMode::Sticky {
                note_id: "x".into(),
            },
            w,
            120,
            0,
            dark,
            long,
        );
        win.focused = false;
        shots.push(render(&mut win, 2.0, if dark { dark_bg } else { light_bg }));
    }
    montage(&shots, 1, 12, dark_bg).save(&dir.join("sticky-long-title@2x.png"));

    // File mode with status bar (modified) + find/replace bar, dark graphite, light sage.
    let mut file_shots = Vec::new();
    for (pal, dark) in [(0, true), (0, false), (2, true), (4, false)] {
        let mut w = window(
            WindowMode::File {
                path: "/tmp/notes.md".into(),
                encoding: Encoding::Utf8,
                line_ending: LineEnding::Lf,
            },
            640,
            260,
            pal,
            dark,
            NOTE,
        );
        w.default_font_size = 11.5;
        w.font_size = 11.5;
        let start = NOTE.find("rye").unwrap();
        w.buf.select_range(start, start + 3);
        w.buf.set_cursor(start + 3, false);
        w.buf.insert_str(" sour");
        w.find_open = true;
        w.replace_open = pal == 0 && dark;
        w.find_query = "bread".into();
        w.match_count = 2;
        w.current_match = 1;
        w.case_sensitive = pal == 2;
        w.find_focus = if pal == 0 {
            crate::editor_view::FindFocus::Find
        } else {
            crate::editor_view::FindFocus::None
        };
        file_shots.push(render(&mut w, 1.5, if dark { dark_bg } else { light_bg }));
    }
    montage(&file_shots, 2, 16, dark_bg).save(&dir.join("file-mode.png"));

    // File mode, plain (no find bar), 2x crop of the status bar area.
    let mut w = window(
        WindowMode::File {
            path: "/tmp/a.txt".into(),
            encoding: Encoding::Utf16Le,
            line_ending: LineEnding::CrLf,
        },
        520,
        120,
        0,
        true,
        "Hello",
    );
    w.word_wrap = false;
    render(&mut w, 2.0, dark_bg).save(&dir.join("status-graphite-dark@2x.png"));

    // Overlay panel: graphite dark/light, iris dark; with the shadow margin on a mid-tone backdrop.
    let ow = 560 + 2 * OVERLAY_INSET;
    let oh = 380 + 2 * OVERLAY_INSET;
    let mut ov = Vec::new();
    for (pal, dark, bg) in [
        (0, true, (0.22, 0.24, 0.27)),
        (0, false, (0.72, 0.74, 0.77)),
        (5, true, (0.22, 0.24, 0.27)),
    ] {
        let mut w = window(
            WindowMode::Overlay {
                note_id: "scratchpad".into(),
            },
            ow,
            oh,
            pal,
            dark,
            NOTE,
        );
        w.opacity = 0.96;
        w.hover = if pal == 5 { Some(HdrBtn::Close) } else { None };
        ov.push(render(&mut w, 1.0, bg));
    }
    for (i, s) in ov.iter().enumerate() {
        s.save(&dir.join(format!("overlay-{i}.png")));
    }
}
