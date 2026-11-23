//! Private UI font: Archivo Medium (SIL OFL 1.1, Latin subset) is embedded in the binary and made
//! visible to Pango/fontconfig for this process only; nothing is installed system-wide.
//!
//! The font bytes are written once to `$XDG_RUNTIME_DIR/kadex/` (tmpfs, per login session), or
//! `$XDG_CACHE_HOME/kadex/` / `~/.cache/kadex/` when there is no runtime dir, and registered with
//! `FcConfigAppFontAddFile` (works on every Pango version). If anything fails the UI silently falls
//! back to the system `sans-serif`, because [`UI_FAMILY`] lists it as the second choice.

use std::ffi::CString;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

use crate::ffi::kdx_font_register;

const ARCHIVO_MEDIUM: &[u8] = include_bytes!("../../../assets/fonts/Archivo-Medium.ttf");
const FILE_NAME: &str = "Archivo-Medium.ttf";

/// Pango family list for header, status bar and find bar text.
pub const UI_FAMILY: &str = "Archivo Medium,sans-serif";

fn target_dir() -> Option<PathBuf> {
    let env = |k: &str| {
        std::env::var_os(k)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    let base = env("XDG_RUNTIME_DIR")
        .or_else(|| env("XDG_CACHE_HOME"))
        .or_else(|| env("HOME").map(|h| h.join(".cache")))?;
    Some(base.join("kadex"))
}

fn materialize() -> Option<PathBuf> {
    let dir = target_dir()?;
    let path = dir.join(FILE_NAME);
    if fs::read(&path)
        .map(|b| b == ARCHIVO_MEDIUM)
        .unwrap_or(false)
    {
        return Some(path);
    }
    fs::create_dir_all(&dir).ok()?;
    let tmp = dir.join(format!("{FILE_NAME}.{}.tmp", std::process::id()));
    let mut f = fs::File::create(&tmp).ok()?;
    f.write_all(ARCHIVO_MEDIUM).ok()?;
    drop(f);
    fs::rename(&tmp, &path).ok()?;
    Some(path)
}

/// Registers the embedded UI font. Returns whether Pango can now see it.
pub fn register_ui_font() -> bool {
    let Some(path) = materialize() else {
        return false;
    };
    let Ok(c_path) = CString::new(path.as_os_str().as_encoded_bytes()) else {
        return false;
    };
    unsafe { kdx_font_register(c_path.as_ptr()) }
}
