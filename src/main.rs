//! Kadex — ultra-lean native Rust Notepad & Sticky Notes (Windows + Linux / Wayland / Niri).

#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]

use std::env;
use std::process::ExitCode;

use kadex_core::ipc::IpcCommand;

fn print_help() {
    println!(
        "Kadex {} — Native Rust Notepad & Sticky Notes\n\n\
         USAGE:\n\
         \x20 kadex                  Show sticky notes (or create one)\n\
         \x20 kadex <file>           Open a file in File Mode\n\
         \x20 kadex new              Spawn a new sticky note\n\
         \x20 kadex toggle           Show/hide all sticky notes\n\
         \x20 kadex hide             Hide all sticky notes\n\
         \x20 kadex list             Print sticky notes as `id<TAB>title` (for fuzzel/rofi)\n\
         \x20 kadex open <id>        Open/focus a specific sticky note by ID\n\
         \x20 kadex overlay          Toggle Wayland layer-shell overlay scratchpad\n\
         \x20 kadex quit             Flush and stop the background daemon\n",
        env!("CARGO_PKG_VERSION")
    );
}

fn parse_cli() -> Result<Option<IpcCommand>, String> {
    let mut args = env::args().skip(1);
    let Some(first) = args.next() else {
        return Ok(Some(IpcCommand::Activate));
    };
    match first.as_str() {
        "-h" | "--help" => {
            print_help();
            Ok(None)
        }
        "-V" | "--version" => {
            println!("kadex {}", env!("CARGO_PKG_VERSION"));
            Ok(None)
        }
        "new" => Ok(Some(IpcCommand::NewSticky)),
        "toggle" => Ok(Some(IpcCommand::Toggle)),
        "hide" => Ok(Some(IpcCommand::Hide)),
        "list" => Ok(Some(IpcCommand::List)),
        "overlay" => Ok(Some(IpcCommand::Overlay)),
        "quit" => Ok(Some(IpcCommand::Quit)),
        "open" => {
            let id = args.next().ok_or_else(|| String::from("usage: kadex open <id>"))?;
            Ok(Some(IpcCommand::OpenNote(id)))
        }
        file_path => {
            // Canonicalize relative paths so the running daemon opens the right file
            let resolved = std::path::PathBuf::from(file_path);
            let abs = if resolved.is_absolute() {
                resolved
            } else {
                env::current_dir()
                    .map(|cwd| cwd.join(&resolved))
                    .unwrap_or(resolved)
            };
            Ok(Some(IpcCommand::OpenFile(abs.to_string_lossy().into_owned())))
        }
    }
}

fn main() -> ExitCode {
    #[cfg(target_os = "windows")]
    if env::args_os().len() > 1 {
        kadex_platform_win::attach_parent_console();
    }

    let cmd = match parse_cli() {
        Ok(Some(c)) => c,
        Ok(None) => return ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("kadex: {e}");
            return ExitCode::FAILURE;
        }
    };

    #[cfg(target_os = "linux")]
    {
        // Fast-path: send command over Unix socket to an existing daemon (<5 ms)
        if let Some(resp) = kadex_platform_wl::try_send_ipc(&cmd) {
            if matches!(cmd, IpcCommand::List) {
                print!("{resp}");
            }
            return ExitCode::SUCCESS;
        }
        // Offline fast-path for `kadex list`, `hide`, `quit` when no daemon is running
        match &cmd {
            IpcCommand::List => match kadex_platform_wl::list_notes_offline() {
                Ok(out) => {
                    print!("{out}");
                    return ExitCode::SUCCESS;
                }
                Err(e) => {
                    eprintln!("kadex: {e}");
                    return ExitCode::FAILURE;
                }
            },
            IpcCommand::Hide | IpcCommand::Quit => return ExitCode::SUCCESS,
            _ => {}
        }
        if let Err(e) = kadex_platform_wl::run_daemon(cmd) {
            eprintln!("kadex: {e}");
            return ExitCode::FAILURE;
        }
        ExitCode::SUCCESS
    }

    #[cfg(target_os = "windows")]
    {
        if matches!(cmd, IpcCommand::List) {
            if let Ok(storage) = kadex_core::storage::Storage::open() {
                if let Ok(entries) = storage.list_notes() {
                    for e in entries {
                        println!("{}\t{}", e.id, e.title);
                    }
                    return ExitCode::SUCCESS;
                }
            }
            return ExitCode::FAILURE;
        }
        if kadex_platform_win::try_send_ipc(&cmd).is_some() {
            return ExitCode::SUCCESS;
        }
        if matches!(cmd, IpcCommand::Hide | IpcCommand::Quit) {
            return ExitCode::SUCCESS;
        }
        if let Err(e) = kadex_platform_win::run_daemon(cmd) {
            eprintln!("kadex: {e}");
            return ExitCode::FAILURE;
        }
        ExitCode::SUCCESS
    }

    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        eprintln!("kadex: unsupported target OS");
        ExitCode::FAILURE
    }
}
