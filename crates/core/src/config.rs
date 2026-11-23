//! Hand-rolled INI/TSV configuration, window state persistence, and color palettes.

use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeMode {
    Auto,
    Dark,
    Light,
}

impl ThemeMode {
    pub fn as_str(self) -> &'static str {
        match self {
            ThemeMode::Auto => "auto",
            ThemeMode::Dark => "dark",
            ThemeMode::Light => "light",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "dark" => ThemeMode::Dark,
            "light" => ThemeMode::Light,
            _ => ThemeMode::Auto,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub font_family: String,
    pub font_size: f32,
    pub word_wrap: bool,
    pub theme: ThemeMode,
    pub default_palette: usize,
    pub opacity: f32,
    pub autosave_ms: u64,
    pub notes_dir: Option<String>,
    pub sticky_width: u32,
    pub sticky_height: u32,
    pub editor_width: u32,
    pub editor_height: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            font_family: String::from(if cfg!(windows) {
                "Consolas"
            } else {
                "monospace"
            }),
            font_size: 11.5,
            word_wrap: true,
            theme: ThemeMode::Auto,
            default_palette: 0,
            opacity: 0.96,
            autosave_ms: 500,
            notes_dir: None,
            sticky_width: 360,
            sticky_height: 320,
            editor_width: 780,
            editor_height: 600,
        }
    }
}

impl Config {
    pub fn parse_ini(input: &str) -> Self {
        let mut cfg = Self::default();
        for raw_line in input.lines() {
            let line = raw_line.trim();
            if line.is_empty()
                || line.starts_with('#')
                || line.starts_with(';')
                || line.starts_with('[')
            {
                continue;
            }
            if let Some((k, v)) = line.split_once('=') {
                let key = k.trim();
                let val = v.trim();
                match key {
                    "font_family" if !val.is_empty() => cfg.font_family = val.to_string(),
                    "font_size" => {
                        if let Some(n) = parse_f32(val) {
                            cfg.font_size = n.clamp(7.0, 48.0);
                        }
                    }
                    "word_wrap" => cfg.word_wrap = parse_bool(val, true),
                    "theme" => cfg.theme = ThemeMode::from_str(val),
                    "default_palette" => {
                        if let Ok(n) = val.parse::<usize>() {
                            cfg.default_palette = n % PALETTE_COUNT;
                        }
                    }
                    "opacity" => {
                        if let Some(n) = parse_f32(val) {
                            cfg.opacity = n.clamp(0.3, 1.0);
                        }
                    }
                    "autosave_ms" => {
                        if let Ok(n) = val.parse::<u64>() {
                            cfg.autosave_ms = n.clamp(100, 10_000);
                        }
                    }
                    "notes_dir" if !val.is_empty() => cfg.notes_dir = Some(val.to_string()),
                    "sticky_width" => {
                        if let Ok(n) = val.parse::<u32>() {
                            cfg.sticky_width = n.clamp(220, 2400);
                        }
                    }
                    "sticky_height" => {
                        if let Ok(n) = val.parse::<u32>() {
                            cfg.sticky_height = n.clamp(160, 2000);
                        }
                    }
                    "editor_width" => {
                        if let Ok(n) = val.parse::<u32>() {
                            cfg.editor_width = n.clamp(320, 3840);
                        }
                    }
                    "editor_height" => {
                        if let Ok(n) = val.parse::<u32>() {
                            cfg.editor_height = n.clamp(240, 2160);
                        }
                    }
                    _ => {}
                }
            }
        }
        cfg
    }

    pub fn to_ini(&self) -> String {
        let mut s = String::with_capacity(320);
        s.push_str("# Kadex configuration\nfont_family = ");
        s.push_str(&self.font_family);
        s.push_str("\nfont_size = ");
        push_fixed(&mut s, self.font_size, 10);
        s.push_str("\nword_wrap = ");
        s.push_str(if self.word_wrap { "true" } else { "false" });
        s.push_str("\ntheme = ");
        s.push_str(self.theme.as_str());
        s.push_str("\ndefault_palette = ");
        push_u64(&mut s, self.default_palette as u64);
        s.push_str("\nopacity = ");
        push_fixed(&mut s, self.opacity, 100);
        s.push_str("\nautosave_ms = ");
        push_u64(&mut s, self.autosave_ms);
        s.push_str("\nsticky_width = ");
        push_u64(&mut s, self.sticky_width as u64);
        s.push_str("\nsticky_height = ");
        push_u64(&mut s, self.sticky_height as u64);
        s.push_str("\neditor_width = ");
        push_u64(&mut s, self.editor_width as u64);
        s.push_str("\neditor_height = ");
        push_u64(&mut s, self.editor_height as u64);
        s.push('\n');
        if let Some(dir) = &self.notes_dir {
            s.push_str("notes_dir = ");
            s.push_str(dir);
            s.push('\n');
        }
        s
    }
}

fn push_u64(s: &mut String, mut n: u64) {
    let mut buf = [0u8; 20];
    let mut i = buf.len();
    if n == 0 {
        s.push('0');
        return;
    }
    while n > 0 {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    if let Ok(part) = std::str::from_utf8(&buf[i..]) {
        s.push_str(part);
    }
}

fn push_fixed(s: &mut String, val: f32, scale: u64) {
    let scaled = (val.max(0.0) * (scale as f32) + 0.5) as u64;
    let whole = scaled / scale;
    let frac = scaled % scale;
    push_u64(s, whole);
    s.push('.');
    if scale == 100 && frac < 10 {
        s.push('0');
    }
    push_u64(s, frac);
}

fn parse_f32(s: &str) -> Option<f32> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let mut whole: u64 = 0;
    let mut frac: u64 = 0;
    let mut frac_div: f32 = 1.0;
    let mut in_frac = false;
    let mut saw_digit = false;
    for b in s.bytes() {
        if b == b'.' && !in_frac {
            in_frac = true;
        } else if b.is_ascii_digit() {
            saw_digit = true;
            let d = (b - b'0') as u64;
            if in_frac {
                if frac_div < 10_000.0 {
                    frac = frac * 10 + d;
                    frac_div *= 10.0;
                }
            } else {
                whole = whole.saturating_mul(10).saturating_add(d);
            }
        } else {
            return None;
        }
    }
    if !saw_digit {
        return None;
    }
    Some(whole as f32 + (frac as f32) / frac_div)
}

#[derive(Debug, Clone)]
pub struct NoteWindowState {
    pub id: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub palette: usize,
    pub pinned: bool,
    pub open: bool,
}

impl NoteWindowState {
    pub fn new(id: String, palette: usize, width: u32, height: u32) -> Self {
        Self {
            id,
            x: 120,
            y: 120,
            width,
            height,
            palette: palette % PALETTE_COUNT,
            pinned: false,
            open: true,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct AppState {
    pub notes: BTreeMap<String, NoteWindowState>,
}

impl AppState {
    pub fn parse_ini(input: &str) -> Self {
        let mut state = Self::default();
        let mut cur: Option<NoteWindowState> = None;

        for raw in input.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') {
                if let Some(prev) = cur.take() {
                    state.notes.insert(prev.id.clone(), prev);
                }
                let section = &line[1..line.len() - 1];
                if let Some(id) = section.strip_prefix("note:") {
                    cur = Some(NoteWindowState::new(id.to_string(), 0, 360, 320));
                }
                continue;
            }
            if let (Some(note), Some((k, v))) = (cur.as_mut(), line.split_once('=')) {
                let key = k.trim();
                let val = v.trim();
                match key {
                    "x" => {
                        if let Ok(n) = val.parse() {
                            note.x = n;
                        }
                    }
                    "y" => {
                        if let Ok(n) = val.parse() {
                            note.y = n;
                        }
                    }
                    "width" => {
                        if let Ok(n) = val.parse::<u32>() {
                            note.width = n.clamp(220, 2400);
                        }
                    }
                    "height" => {
                        if let Ok(n) = val.parse::<u32>() {
                            note.height = n.clamp(160, 2000);
                        }
                    }
                    "palette" => {
                        if let Ok(n) = val.parse::<usize>() {
                            note.palette = n % PALETTE_COUNT;
                        }
                    }
                    "pinned" => note.pinned = parse_bool(val, false),
                    "open" => note.open = parse_bool(val, true),
                    _ => {}
                }
            }
        }
        if let Some(last) = cur {
            state.notes.insert(last.id.clone(), last);
        }
        state
    }

    pub fn to_ini(&self) -> String {
        let mut s = String::with_capacity(self.notes.len() * 96 + 32);
        s.push_str("# Kadex window state\n");
        for note in self.notes.values() {
            s.push_str("\n[note:");
            s.push_str(&note.id);
            s.push_str("]\n");
            s.push_str(&format!("x = {}\n", note.x));
            s.push_str(&format!("y = {}\n", note.y));
            s.push_str(&format!("width = {}\n", note.width));
            s.push_str(&format!("height = {}\n", note.height));
            s.push_str(&format!("palette = {}\n", note.palette));
            s.push_str(&format!("pinned = {}\n", note.pinned));
            s.push_str(&format!("open = {}\n", note.open));
        }
        s
    }
}

fn parse_bool(v: &str, default: bool) -> bool {
    match v.to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => true,
        "false" | "0" | "no" | "off" => false,
        _ => default,
    }
}

/// 6 curated palettes (each available in dark and light variants).
///
/// Index semantics are stable (`palette = N` in `state.ini`): 0 Graphite (was Obsidian/Paper),
/// 1 Amber (Amber/Butter), 2 Sage (Sage/Mint), 3 Cobalt (Cobalt/Sky), 4 Rose (Rose/Blush),
/// 5 Iris (Iris/Lilac).
pub const PALETTE_COUNT: usize = 6;

/// Kavox brand red. Used for the text caret in every palette.
pub const RED: u32 = 0xE8262D;

/// Colour tokens of the Kavox / Kapix design family, resolved for one palette and theme.
///
/// Palette 0 (Graphite) carries the Kavox tokens verbatim (`field`=bg, `bg`=panel, `header`=surface,
/// `hover`=surface_2, `border`=line, `border_strong`=line_strong). The other five keep the same
/// lightness steps and only wash a hue over them, so text contrast is identical in every palette.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub name: &'static str,
    /// Recessed field background (Kavox `bg`): find/replace inputs.
    pub field: u32, // 0xRRGGBB
    /// Note body (Kavox `panel`).
    pub bg: u32,
    /// Header / status bar (Kavox `surface`).
    pub header: u32,
    /// Hover fill (Kavox `surface_2`).
    pub hover: u32,
    /// 1 px separators (Kavox `line`).
    pub border: u32,
    /// Palette-dot ring, hovered outlines (Kavox `line_strong`).
    pub border_strong: u32,
    /// Body and primary text (Kavox `text`).
    pub fg: u32,
    pub muted: u32,
    /// Separator dots, disabled glyphs.
    pub faint: u32,
    /// Hue-specific colour of the palette dot (and focus accents).
    pub accent: u32,
    /// Primary / active fill (Kavox `lit`); also the selection colour.
    pub lit: u32,
    /// Content drawn on `lit`.
    pub on_lit: u32,
    /// Red for icons/text on `header` (danger hover).
    pub red_text: u32,
    /// Alpha (0-255) of `lit` used as the selection highlight (Kavox: 56 dark, 40 light).
    pub selection_alpha: u8,
}

impl Palette {
    /// Text caret colour: Kavox red in every palette (matches the Kadex logo).
    pub const fn caret(&self) -> u32 {
        RED
    }
}

pub const DARK_PALETTES: [Palette; PALETTE_COUNT] = [
    // 0: Graphite
    Palette {
        name: "Graphite",
        field: 0x141414,
        bg: 0x1B1B1B,
        header: 0x222222,
        hover: 0x2B2B2B,
        border: 0x2B2B2B,
        border_strong: 0x3D3D3D,
        fg: 0xF2F1EE,
        muted: 0x8F8E89,
        faint: 0x5C5B57,
        accent: 0xD6D5D0,
        lit: 0xF2F1EE,
        on_lit: 0x141414,
        red_text: 0xFF4D53,
        selection_alpha: 56,
    },
    // 1: Amber
    Palette {
        name: "Amber",
        field: 0x17140E,
        bg: 0x1F1A13,
        header: 0x272117,
        hover: 0x322A1E,
        border: 0x322A1D,
        border_strong: 0x463B2B,
        fg: 0xF4F1EB,
        muted: 0x968D7F,
        faint: 0x625A4D,
        accent: 0xF2A93B,
        lit: 0xF4F1EB,
        on_lit: 0x17140E,
        red_text: 0xFF4D53,
        selection_alpha: 56,
    },
    // 2: Sage
    Palette {
        name: "Sage",
        field: 0x101612,
        bg: 0x161D18,
        header: 0x1B251D,
        hover: 0x222F25,
        border: 0x222F25,
        border_strong: 0x314235,
        fg: 0xEDF3EE,
        muted: 0x849287,
        faint: 0x525F55,
        accent: 0x8DB596,
        lit: 0xEDF3EE,
        on_lit: 0x101612,
        red_text: 0xFF4D53,
        selection_alpha: 56,
    },
    // 3: Cobalt
    Palette {
        name: "Cobalt",
        field: 0x101519,
        bg: 0x161C22,
        header: 0x1B232B,
        hover: 0x222C37,
        border: 0x222C37,
        border_strong: 0x313F4D,
        fg: 0xEDF2F7,
        muted: 0x848F9B,
        faint: 0x525C67,
        accent: 0x86A6D6,
        lit: 0xEDF2F7,
        on_lit: 0x101519,
        red_text: 0xFF4D53,
        selection_alpha: 56,
    },
    // 4: Rose
    Palette {
        name: "Rose",
        field: 0x191213,
        bg: 0x221819,
        header: 0x2B1E20,
        hover: 0x362628,
        border: 0x372628,
        border_strong: 0x4C3739,
        fg: 0xF7EFF0,
        muted: 0x9B898A,
        faint: 0x675658,
        accent: 0xD98C9D,
        lit: 0xF7EFF0,
        on_lit: 0x191213,
        red_text: 0xFF4D53,
        selection_alpha: 56,
    },
    // 5: Iris
    Palette {
        name: "Iris",
        field: 0x141319,
        bg: 0x1B1A22,
        header: 0x23202B,
        hover: 0x2C2936,
        border: 0x2C2937,
        border_strong: 0x3E3A4C,
        fg: 0xF1F0F6,
        muted: 0x8F8C9B,
        faint: 0x5C5967,
        accent: 0xAA9CDB,
        lit: 0xF1F0F6,
        on_lit: 0x141319,
        red_text: 0xFF4D53,
        selection_alpha: 56,
    },
];

pub const LIGHT_PALETTES: [Palette; PALETTE_COUNT] = [
    // 0: Graphite
    Palette {
        name: "Graphite",
        field: 0xEFEEE9,
        bg: 0xF8F7F4,
        header: 0xE4E3DE,
        hover: 0xD9D8D2,
        border: 0xD8D6CF,
        border_strong: 0xBDBBB4,
        fg: 0x191919,
        muted: 0x696863,
        faint: 0xA3A19A,
        accent: 0x4A4A47,
        lit: 0x191919,
        on_lit: 0xF4F3EF,
        red_text: 0xC2161C,
        selection_alpha: 40,
    },
    // 1: Amber
    Palette {
        name: "Amber",
        field: 0xF2EDE5,
        bg: 0xFAF7F2,
        header: 0xE8E2D9,
        hover: 0xDED7CC,
        border: 0xDDD5C9,
        border_strong: 0xC2BAAD,
        fg: 0x1B1916,
        muted: 0x6C6760,
        faint: 0xA6A098,
        accent: 0xA86400,
        lit: 0x1B1916,
        on_lit: 0xF6F3EE,
        red_text: 0xC2161C,
        selection_alpha: 40,
    },
    // 2: Sage
    Palette {
        name: "Sage",
        field: 0xE8F0EA,
        bg: 0xF4F9F5,
        header: 0xDDE6DE,
        hover: 0xD0DBD2,
        border: 0xCEDAD0,
        border_strong: 0xB2BFB5,
        fg: 0x171A17,
        muted: 0x636A64,
        faint: 0x9BA39D,
        accent: 0x4C7A57,
        lit: 0x171A17,
        on_lit: 0xF0F4F1,
        red_text: 0xC2161C,
        selection_alpha: 40,
    },
    // 3: Cobalt
    Palette {
        name: "Cobalt",
        field: 0xE8EFF6,
        bg: 0xF4F8FC,
        header: 0xDCE4EC,
        hover: 0xD0D9E3,
        border: 0xCDD7E2,
        border_strong: 0xB2BCC8,
        fg: 0x17191C,
        muted: 0x62696F,
        faint: 0x9BA2A9,
        accent: 0x3F6AA5,
        lit: 0x17191C,
        on_lit: 0xF0F3F8,
        red_text: 0xC2161C,
        selection_alpha: 40,
    },
    // 4: Rose
    Palette {
        name: "Rose",
        field: 0xF6EBEC,
        bg: 0xFCF5F6,
        header: 0xECDFE0,
        hover: 0xE3D4D5,
        border: 0xE2D2D3,
        border_strong: 0xC8B6B8,
        fg: 0x1C1818,
        muted: 0x6F6566,
        faint: 0xA99E9F,
        accent: 0xB04A63,
        lit: 0x1C1818,
        on_lit: 0xF8F1F2,
        red_text: 0xC2161C,
        selection_alpha: 40,
    },
    // 5: Iris
    Palette {
        name: "Iris",
        field: 0xEEEDF5,
        bg: 0xF7F6FB,
        header: 0xE3E1EC,
        hover: 0xD8D6E2,
        border: 0xD7D4E1,
        border_strong: 0xBCB9C7,
        fg: 0x19181C,
        muted: 0x68676F,
        faint: 0xA1A0A9,
        accent: 0x6F5CB0,
        lit: 0x19181C,
        on_lit: 0xF3F2F7,
        red_text: 0xC2161C,
        selection_alpha: 40,
    },
];

pub fn palette(index: usize, dark: bool) -> Palette {
    let idx = index % PALETTE_COUNT;
    if dark {
        DARK_PALETTES[idx]
    } else {
        LIGHT_PALETTES[idx]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_roundtrip() {
        let mut c = Config::default();
        c.font_family = "JetBrains Mono".into();
        c.font_size = 13.0;
        c.theme = ThemeMode::Dark;
        c.default_palette = 2;
        c.word_wrap = false;
        let ini = c.to_ini();
        let parsed = Config::parse_ini(&ini);
        assert_eq!(parsed.font_family, "JetBrains Mono");
        assert_eq!(parsed.font_size, 13.0);
        assert_eq!(parsed.theme, ThemeMode::Dark);
        assert_eq!(parsed.default_palette, 2);
        assert!(!parsed.word_wrap);
    }

    #[test]
    fn state_roundtrip() {
        let mut st = AppState::default();
        st.notes.insert(
            "20261123-01".into(),
            NoteWindowState {
                id: "20261123-01".into(),
                x: 40,
                y: 80,
                width: 400,
                height: 300,
                palette: 3,
                pinned: true,
                open: true,
            },
        );
        let ini = st.to_ini();
        let parsed = AppState::parse_ini(&ini);
        let n = parsed.notes.get("20261123-01").unwrap();
        assert_eq!(
            (n.x, n.y, n.width, n.height, n.palette, n.pinned),
            (40, 80, 400, 300, 3, true)
        );
    }

    fn channel(c: u32) -> f64 {
        let v = c as f64 / 255.0;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    }

    fn luminance(rgb: u32) -> f64 {
        0.2126 * channel((rgb >> 16) & 0xFF)
            + 0.7152 * channel((rgb >> 8) & 0xFF)
            + 0.0722 * channel(rgb & 0xFF)
    }

    /// WCAG contrast ratio.
    fn contrast(a: u32, b: u32) -> f64 {
        let (x, y) = (luminance(a), luminance(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    #[test]
    fn graphite_is_kavox_tokens() {
        let d = palette(0, true);
        assert_eq!(d.name, "Graphite");
        assert_eq!(
            (d.field, d.bg, d.header, d.hover, d.border, d.border_strong),
            (0x141414, 0x1B1B1B, 0x222222, 0x2B2B2B, 0x2B2B2B, 0x3D3D3D)
        );
        assert_eq!(
            (d.fg, d.muted, d.faint, d.lit),
            (0xF2F1EE, 0x8F8E89, 0x5C5B57, 0xF2F1EE)
        );
        assert_eq!(d.selection_alpha, 56);
        let l = palette(0, false);
        assert_eq!(l.name, "Graphite");
        assert_eq!(
            (l.field, l.bg, l.header, l.hover, l.border, l.border_strong),
            (0xEFEEE9, 0xF8F7F4, 0xE4E3DE, 0xD9D8D2, 0xD8D6CF, 0xBDBBB4)
        );
        assert_eq!(
            (l.fg, l.muted, l.faint, l.on_lit),
            (0x191919, 0x696863, 0xA3A19A, 0xF4F3EF)
        );
        assert_eq!(l.selection_alpha, 40);
    }

    #[test]
    fn palettes_keep_text_contrast() {
        for dark in [true, false] {
            for i in 0..PALETTE_COUNT {
                let p = palette(i, dark);
                // WCAG AAA for body text, AA for secondary text on the note body.
                assert!(contrast(p.fg, p.bg) >= 7.0, "{} fg", p.name);
                assert!(contrast(p.muted, p.bg) >= 4.5, "{} muted", p.name);
                assert!(contrast(p.fg, p.header) >= 7.0, "{} header fg", p.name);
                // The Kavox-red caret must stay visible on the note body.
                assert!(contrast(p.caret(), p.bg) >= 3.0, "{} caret", p.name);
            }
        }
    }

    #[test]
    fn palette_names_are_stable_per_index() {
        let names = ["Graphite", "Amber", "Sage", "Cobalt", "Rose", "Iris"];
        for (i, n) in names.iter().enumerate() {
            assert_eq!(palette(i, true).name, *n);
            assert_eq!(palette(i, false).name, *n);
        }
        // Existing `palette = N` values keep working, even out of range.
        assert_eq!(palette(PALETTE_COUNT + 2, true).name, "Sage");
    }
}
