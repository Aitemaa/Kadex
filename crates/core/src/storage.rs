//! File encoding/EOL detection, crash-safe atomic writes, XDG/APPDATA/portable paths, and sticky note storage.

use std::env;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::{AppState, Config};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
}

impl Encoding {
    pub fn label(self) -> &'static str {
        match self {
            Encoding::Utf8 => "UTF-8",
            Encoding::Utf8Bom => "UTF-8 BOM",
            Encoding::Utf16Le => "UTF-16 LE",
            Encoding::Utf16Be => "UTF-16 BE",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEnding {
    Lf,
    CrLf,
}

impl LineEnding {
    pub fn label(self) -> &'static str {
        match self {
            LineEnding::Lf => "LF",
            LineEnding::CrLf => "CRLF",
        }
    }

    pub fn platform_default() -> Self {
        #[cfg(target_os = "windows")]
        {
            LineEnding::CrLf
        }
        #[cfg(not(target_os = "windows"))]
        {
            LineEnding::Lf
        }
    }
}

#[derive(Debug, Clone)]
pub struct LoadedText {
    pub text: String,
    pub encoding: Encoding,
    pub line_ending: LineEnding,
}

/// Detect encoding (UTF-8, UTF-8 BOM, UTF-16 LE/BE) and line ending (`LF` / `CRLF`),
/// returning LF-normalized UTF-8 text.
pub fn decode_bytes(bytes: &[u8]) -> LoadedText {
    let (encoding, raw_text) = if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        (
            Encoding::Utf8Bom,
            String::from_utf8_lossy(&bytes[3..]).into_owned(),
        )
    } else if bytes.starts_with(&[0xFF, 0xFE]) {
        (Encoding::Utf16Le, decode_utf16(&bytes[2..], true))
    } else if bytes.starts_with(&[0xFE, 0xFF]) {
        (Encoding::Utf16Be, decode_utf16(&bytes[2..], false))
    } else {
        (Encoding::Utf8, String::from_utf8_lossy(bytes).into_owned())
    };

    let crlf_count = raw_text.matches("\r\n").count();
    let lf_count = raw_text.matches('\n').count().saturating_sub(crlf_count);
    let line_ending = if crlf_count > 0 && crlf_count >= lf_count {
        LineEnding::CrLf
    } else if lf_count > 0 {
        LineEnding::Lf
    } else {
        LineEnding::platform_default()
    };

    let normalized = if raw_text.contains('\r') {
        raw_text.replace("\r\n", "\n").replace('\r', "\n")
    } else {
        raw_text
    };

    LoadedText {
        text: normalized,
        encoding,
        line_ending,
    }
}

/// Encode LF-normalized UTF-8 text back to the requested `Encoding` and `LineEnding`.
pub fn encode_bytes(text: &str, encoding: Encoding, line_ending: LineEnding) -> Vec<u8> {
    let with_eol: String;
    let src: &str = match line_ending {
        LineEnding::Lf => text,
        LineEnding::CrLf => {
            with_eol = text.replace('\n', "\r\n");
            &with_eol
        }
    };

    match encoding {
        Encoding::Utf8 => src.as_bytes().to_vec(),
        Encoding::Utf8Bom => {
            let mut out = Vec::with_capacity(3 + src.len());
            out.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
            out.extend_from_slice(src.as_bytes());
            out
        }
        Encoding::Utf16Le => {
            let mut out = Vec::with_capacity(2 + src.len() * 2);
            out.extend_from_slice(&[0xFF, 0xFE]);
            for u in src.encode_utf16() {
                out.extend_from_slice(&u.to_le_bytes());
            }
            out
        }
        Encoding::Utf16Be => {
            let mut out = Vec::with_capacity(2 + src.len() * 2);
            out.extend_from_slice(&[0xFE, 0xFF]);
            for u in src.encode_utf16() {
                out.extend_from_slice(&u.to_be_bytes());
            }
            out
        }
    }
}

fn decode_utf16(bytes: &[u8], le: bool) -> String {
    let mut units = Vec::with_capacity(bytes.len() / 2);
    let mut chunks = bytes.chunks_exact(2);
    for c in &mut chunks {
        let u = if le {
            u16::from_le_bytes([c[0], c[1]])
        } else {
            u16::from_be_bytes([c[0], c[1]])
        };
        units.push(u);
    }
    String::from_utf16_lossy(&units)
}

pub fn load_text_file(path: &Path) -> io::Result<LoadedText> {
    let bytes = fs::read(path)?;
    Ok(decode_bytes(&bytes))
}

/// Crash-safe atomic write: write sibling temp file -> fsync -> atomic rename.
pub fn atomic_write(path: &Path, data: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;

    let pid = std::process::id();
    let seq = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file");
    let tmp_path = parent.join(format!(".{file_name}.kadex.{pid}.{seq}.tmp"));

    let write_res = (|| -> io::Result<()> {
        let mut f = File::create(&tmp_path)?;
        f.write_all(data)?;
        f.sync_all()?;
        Ok(())
    })();

    if let Err(e) = write_res {
        let _ = fs::remove_file(&tmp_path);
        return Err(e);
    }

    #[cfg(target_os = "windows")]
    {
        // fs::rename on Windows uses MoveFileExW(MOVEFILE_REPLACE_EXISTING) in Rust std
        if let Err(e) = fs::rename(&tmp_path, path) {
            let _ = fs::remove_file(&tmp_path);
            return Err(e);
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        if let Err(e) = fs::rename(&tmp_path, path) {
            let _ = fs::remove_file(&tmp_path);
            return Err(e);
        }
    }

    Ok(())
}

pub fn save_text_file(
    path: &Path,
    text: &str,
    encoding: Encoding,
    line_ending: LineEnding,
) -> io::Result<()> {
    let bytes = encode_bytes(text, encoding, line_ending);
    atomic_write(path, &bytes)
}

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub config_file: PathBuf,
    pub state_file: PathBuf,
    pub notes_dir: PathBuf,
    pub archive_dir: PathBuf,
    pub socket_path: PathBuf,
    pub portable: bool,
}

impl AppPaths {
    pub fn resolve(custom_notes_dir: Option<&str>) -> Self {
        // Portable mode: presence of `portable.ini` next to the binary
        if let Ok(exe) = env::current_exe() {
            if let Some(exe_dir) = exe.parent() {
                if exe_dir.join("portable.ini").exists() {
                    let notes_dir = custom_notes_dir
                        .map(PathBuf::from)
                        .unwrap_or_else(|| exe_dir.join("notes"));
                    let archive_dir = notes_dir.join("archive");
                    return Self {
                        config_file: exe_dir.join("config.ini"),
                        state_file: exe_dir.join("state.ini"),
                        notes_dir,
                        archive_dir,
                        socket_path: Self::default_socket_path(),
                        portable: true,
                    };
                }
            }
        }

        #[cfg(target_os = "windows")]
        {
            let base = env::var_os("APPDATA")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."))
                .join("kadex");
            let notes_dir = custom_notes_dir
                .map(PathBuf::from)
                .unwrap_or_else(|| base.join("notes"));
            let archive_dir = notes_dir.join("archive");
            Self {
                config_file: base.join("config.ini"),
                state_file: base.join("state.ini"),
                notes_dir,
                archive_dir,
                socket_path: base.join("kadex.sock"),
                portable: false,
            }
        }

        #[cfg(not(target_os = "windows"))]
        {
            let home = env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/tmp"));
            let config_base = env::var_os("XDG_CONFIG_HOME")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".config"))
                .join("kadex");
            let data_base = env::var_os("XDG_DATA_HOME")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".local/share"))
                .join("kadex");
            let state_base = env::var_os("XDG_STATE_HOME")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".local/state"))
                .join("kadex");
            let notes_dir = custom_notes_dir
                .map(PathBuf::from)
                .unwrap_or_else(|| data_base.join("notes"));
            let archive_dir = notes_dir.join("archive");
            Self {
                config_file: config_base.join("config.ini"),
                state_file: state_base.join("state.ini"),
                notes_dir,
                archive_dir,
                socket_path: Self::default_socket_path(),
                portable: false,
            }
        }
    }

    pub fn default_socket_path() -> PathBuf {
        if let Some(rt) = env::var_os("XDG_RUNTIME_DIR").filter(|v| !v.is_empty()) {
            PathBuf::from(rt).join("kadex.sock")
        } else {
            PathBuf::from("/tmp").join("kadex.sock")
        }
    }

    pub fn ensure_dirs(&self) -> io::Result<()> {
        if let Some(p) = self.config_file.parent() {
            fs::create_dir_all(p)?;
        }
        if let Some(p) = self.state_file.parent() {
            fs::create_dir_all(p)?;
        }
        fs::create_dir_all(&self.notes_dir)?;
        fs::create_dir_all(&self.archive_dir)?;
        Ok(())
    }

    pub fn note_path(&self, id: &str) -> PathBuf {
        let safe_id: String = id
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        self.notes_dir.join(format!("{safe_id}.txt"))
    }
}

#[derive(Debug, Clone)]
pub struct NoteEntry {
    pub id: String,
    pub title: String,
    pub modified_secs: u64,
}

pub struct Storage {
    pub paths: AppPaths,
    pub config: Config,
    pub state: AppState,
}

impl Storage {
    pub fn open() -> io::Result<Self> {
        let initial_paths = AppPaths::resolve(None);
        let config = match fs::read_to_string(&initial_paths.config_file) {
            Ok(text) => Config::parse_ini(&text),
            Err(_) => Config::default(),
        };
        let paths = AppPaths::resolve(config.notes_dir.as_deref());
        paths.ensure_dirs()?;

        if !paths.config_file.exists() {
            let _ = atomic_write(&paths.config_file, config.to_ini().as_bytes());
        }

        let state = match fs::read_to_string(&paths.state_file) {
            Ok(text) => AppState::parse_ini(&text),
            Err(_) => AppState::default(),
        };

        Ok(Self {
            paths,
            config,
            state,
        })
    }

    pub fn save_config(&self) -> io::Result<()> {
        atomic_write(&self.paths.config_file, self.config.to_ini().as_bytes())
    }

    pub fn save_state(&self) -> io::Result<()> {
        atomic_write(&self.paths.state_file, self.state.to_ini().as_bytes())
    }

    pub fn new_note_id(&self) -> String {
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        for suffix in 1..1000u32 {
            let id = format!("note-{secs}-{suffix:02}");
            if !self.paths.note_path(&id).exists() {
                return id;
            }
        }
        format!("note-{secs}-{}", std::process::id())
    }

    pub fn list_notes(&self) -> io::Result<Vec<NoteEntry>> {
        let mut entries = Vec::new();
        let rd = match fs::read_dir(&self.paths.notes_dir) {
            Ok(r) => r,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(entries),
            Err(e) => return Err(e),
        };
        for item in rd.flatten() {
            let path = item.path();
            if !path.is_file() {
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("txt") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let modified_secs = item
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let title = read_first_line(&path);
            entries.push(NoteEntry {
                id: stem.to_string(),
                title,
                modified_secs,
            });
        }
        entries.sort_by(|a, b| b.modified_secs.cmp(&a.modified_secs).then_with(|| a.id.cmp(&b.id)));
        Ok(entries)
    }

    pub fn load_note(&self, id: &str) -> io::Result<String> {
        let path = self.paths.note_path(id);
        if !path.exists() {
            return Ok(String::new());
        }
        Ok(load_text_file(&path)?.text)
    }

    pub fn save_note(&self, id: &str, text: &str) -> io::Result<()> {
        let path = self.paths.note_path(id);
        save_text_file(&path, text, Encoding::Utf8, LineEnding::Lf)
    }

    pub fn archive_note(&mut self, id: &str) -> io::Result<()> {
        let src = self.paths.note_path(id);
        if src.exists() {
            fs::create_dir_all(&self.paths.archive_dir)?;
            // Reuse the sanitised file name so a crafted id can't escape the archive folder.
            let Some(name) = src.file_name() else {
                return Ok(());
            };
            let dst = self.paths.archive_dir.join(name);
            fs::rename(&src, &dst)?;
        }
        self.state.notes.remove(id);
        let _ = self.save_state();
        Ok(())
    }

    pub fn delete_note(&mut self, id: &str) -> io::Result<()> {
        let src = self.paths.note_path(id);
        if src.exists() {
            fs::remove_file(&src)?;
        }
        self.state.notes.remove(id);
        let _ = self.save_state();
        Ok(())
    }
}

fn read_first_line(path: &Path) -> String {
    match fs::read(path) {
        Ok(bytes) => {
            let slice = &bytes[..bytes.len().min(512)];
            let decoded = decode_bytes(slice);
            for line in decoded.text.lines() {
                let t = line.trim();
                if !t.is_empty() {
                    let s: String = t.chars().take(80).collect();
                    return s.replace('\t', " ");
                }
            }
            String::from("(empty note)")
        }
        Err(_) => String::from("(unreadable)"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_and_crlf_roundtrip() {
        let original = "Hello\nKadex 🌍";
        let bytes = encode_bytes(original, Encoding::Utf16Le, LineEnding::CrLf);
        assert!(bytes.starts_with(&[0xFF, 0xFE]));
        let loaded = decode_bytes(&bytes);
        assert_eq!(loaded.encoding, Encoding::Utf16Le);
        assert_eq!(loaded.line_ending, LineEnding::CrLf);
        assert_eq!(loaded.text, original);
    }

    #[test]
    fn archive_stays_inside_archive_dir() {
        let dir = env::temp_dir().join(format!("kadex-archive-test-{}", std::process::id()));
        let notes_dir = dir.join("notes");
        let paths = AppPaths {
            config_file: dir.join("config.ini"),
            state_file: dir.join("state.ini"),
            archive_dir: notes_dir.join("archive"),
            notes_dir,
            socket_path: dir.join("kadex.sock"),
            portable: true,
        };
        paths.ensure_dirs().unwrap();
        let mut storage = Storage {
            paths,
            config: Config::default(),
            state: AppState::default(),
        };
        let id = "../../escape";
        storage.save_note(id, "hello").unwrap();
        storage.archive_note(id).unwrap();
        assert!(storage.paths.archive_dir.join("escape.txt").exists());
        assert!(!dir.join("escape.txt").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn atomic_write_creates_clean_file() {
        let dir = env::temp_dir().join(format!("kadex-test-{}", std::process::id()));
        let target = dir.join("sample.txt");
        atomic_write(&target, b"hello atomic").unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "hello atomic");
        let _ = fs::remove_dir_all(&dir);
    }
}
