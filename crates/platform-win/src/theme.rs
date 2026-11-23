//! Windows theme detection (Registry AppsUseLightTheme) and RGB -> COLORREF conversion.

#[cfg(target_os = "windows")]
use kadex_core::config::ThemeMode;

#[cfg(target_os = "windows")]
pub fn rgb_to_colorref(rgb: u32) -> u32 {
    let r = (rgb >> 16) & 0xFF;
    let g = (rgb >> 8) & 0xFF;
    let b = rgb & 0xFF;
    r | (g << 8) | (b << 16)
}

#[cfg(target_os = "windows")]
pub fn to_wide_null(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(target_os = "windows")]
pub fn detect_dark_mode(mode: ThemeMode) -> bool {
    match mode {
        ThemeMode::Dark => true,
        ThemeMode::Light => false,
        ThemeMode::Auto => is_system_dark_mode(),
    }
}

#[cfg(target_os = "windows")]
pub fn is_system_dark_mode() -> bool {
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, KEY_READ, REG_DWORD,
    };

    let subkey = to_wide_null("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize");
    let value_name = to_wide_null("AppsUseLightTheme");
    let mut hkey: HKEY = std::ptr::null_mut();
    unsafe {
        if RegOpenKeyExW(HKEY_CURRENT_USER, subkey.as_ptr(), 0, KEY_READ, &mut hkey) != 0 {
            return true;
        }
        let mut data: u32 = 0;
        let mut data_size: u32 = std::mem::size_of::<u32>() as u32;
        let mut reg_type: u32 = 0;
        let rc = RegQueryValueExW(
            hkey,
            value_name.as_ptr(),
            std::ptr::null_mut(),
            &mut reg_type,
            (&mut data as *mut u32) as *mut u8,
            &mut data_size,
        );
        RegCloseKey(hkey);
        if rc == 0 && reg_type == REG_DWORD {
            data == 0
        } else {
            true
        }
    }
}
