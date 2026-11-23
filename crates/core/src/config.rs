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
            font_family: String::from("monospace"),
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
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') || line.starts_with('[') {
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
pub const PALETTE_COUNT: usize = 6;

#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub name: &'static str,
    pub bg: u32,        // 0xRRGGBB
    pub header: u32,    // 0xRRGGBB
    pub fg: u32,        // 0xRRGGBB
    pub muted: u32,     // 0xRRGGBB
    pub accent: u32,    // 0xRRGGBB
    pub selection: u32, // 0xRRGGBB
    pub border: u32,    // 0xRRGGBB
}

pub const DARK_PALETTES: [Palette; PALETTE_COUNT] = [
    // 0: Obsidian (default)
    Palette {
        name: "Obsidian",
        bg: 0x191B1F,
        header: 0x22252B,
        fg: 0xE6E9EF,
        muted: 0x8B92A0,
        accent: 0x7AA2F7,
        selection: 0x32466B,
        border: 0x2E333D,
    },
    // 1: Amber
    Palette {
        name: "Amber",
        bg: 0x221E17,
        header: 0x2E271C,
        fg: 0xF5EAD6,
        muted: 0xA8987D,
        accent: 0xE5B567,
        selection: 0x524229,
        border: 0x3D3424,
    },
    // 2: Sage
    Palette {
        name: "Sage",
        bg: 0x18211C,
        header: 0x202D26,
        fg: 0xE2EFE7,
        muted: 0x84A190,
        accent: 0x73C991,
        selection: 0x2C4A39,
        border: 0x2B3D33,
    },
    // 3: Cobalt
    Palette {
        name: "Cobalt",
        bg: 0x171D26,
        header: 0x1E2736,
        fg: 0xE1EAF7,
        muted: 0x8395B0,
        accent: 0x67B0E8,
        selection: 0x2A4366,
        border: 0x2A364B,
    },
    // 4: Rose
    Palette {
        name: "Rose",
        bg: 0x24191D,
        header: 0x302026,
        fg: 0xF5E4E9,
        muted: 0xA98692,
        accent: 0xEB6F92,
        selection: 0x542B39,
        border: 0x422B33,
    },
    // 5: Iris
    Palette {
        name: "Iris",
        bg: 0x1E1B26,
        header: 0x282334,
        fg: 0xEAE6F7,
        muted: 0x978EB0,
        accent: 0xBB9AF7,
        selection: 0x40335C,
        border: 0x362F47,
    },
];

pub const LIGHT_PALETTES: [Palette; PALETTE_COUNT] = [
    // 0: Paper
    Palette {
        name: "Paper",
        bg: 0xFAF9F6,
        header: 0xEFECE6,
        fg: 0x1E2025,
        muted: 0x6C727F,
        accent: 0x2F6FE4,
        selection: 0xC9DDFB,
        border: 0xDCD8CE,
    },
    // 1: Butter
    Palette {
        name: "Butter",
        bg: 0xFFF9DB,
        header: 0xF7ECBA,
        fg: 0x2B2412,
        muted: 0x7D6E48,
        accent: 0xC47F17,
        selection: 0xF2DB8D,
        border: 0xE6D79C,
    },
    // 2: Mint
    Palette {
        name: "Mint",
        bg: 0xEEF8F1,
        header: 0xDDF0E3,
        fg: 0x172A1E,
        muted: 0x52735E,
        accent: 0x238652,
        selection: 0xB8E4C7,
        border: 0xC7E2CF,
    },
    // 3: Sky
    Palette {
        name: "Sky",
        bg: 0xEEF5FC,
        header: 0xDDEAF8,
        fg: 0x162333,
        muted: 0x556E8A,
        accent: 0x2270C9,
        selection: 0xBCD8F7,
        border: 0xC5DAEE,
    },
    // 4: Blush
    Palette {
        name: "Blush",
        bg: 0xFCF0F3,
        header: 0xF7DFE6,
        fg: 0x2E1820,
        muted: 0x825866,
        accent: 0xC93B6B,
        selection: 0xF2BDCE,
        border: 0xEBCBD5,
    },
    // 5: Lilac
    Palette {
        name: "Lilac",
        bg: 0xF4F0FC,
        header: 0xE8E0F8,
        fg: 0x211933,
        muted: 0x685A85,
        accent: 0x794BC4,
        selection: 0xD4C2F5,
        border: 0xD8CCF0,
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
        assert_eq!((n.x, n.y, n.width, n.height, n.palette, n.pinned), (40, 80, 400, 300, 3, true));
    }
}
