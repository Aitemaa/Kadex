//! Embeds the Windows icon and version info into `kadex.exe`.
//!
//! The icon is resource ID 1, which the tray icon and window classes load at runtime.

fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon_with_id("assets/icon.ico", "1");
        // Task Manager and the "Open with" list show FileDescription, not the crate description.
        res.set("FileDescription", "Kadex");
        res.set("ProductName", "Kadex");
        if let Err(e) = res.compile() {
            println!("cargo:warning=could not embed the Windows icon: {e}");
        }
    }
}
