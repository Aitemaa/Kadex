//! Single-instance Unix domain socket IPC ($XDG_RUNTIME_DIR/kadex.sock).

use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::io::{AsRawFd, RawFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use kadex_core::ipc::IpcCommand;

pub struct IpcServer {
    listener: UnixListener,
    path: PathBuf,
}

impl IpcServer {
    pub fn bind(path: &Path) -> io::Result<Self> {
        if path.exists() {
            // Check whether a live daemon is already listening
            if UnixStream::connect(path).is_ok() {
                return Err(io::Error::new(
                    io::ErrorKind::AddrInUse,
                    "kadex daemon already running",
                ));
            }
            let _ = fs::remove_file(path);
        }
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let listener = UnixListener::bind(path)?;
        listener.set_nonblocking(true)?;
        Ok(Self {
            listener,
            path: path.to_path_buf(),
        })
    }

    pub fn raw_fd(&self) -> RawFd {
        self.listener.as_raw_fd()
    }

    /// Accepts all currently queued connections without blocking.
    pub fn accept_all<F>(&self, mut handle: F)
    where
        F: FnMut(IpcCommand) -> String,
    {
        loop {
            match self.listener.accept() {
                Ok((mut stream, _)) => {
                    let _ = stream.set_read_timeout(Some(Duration::from_millis(200)));
                    let _ = stream.set_write_timeout(Some(Duration::from_millis(200)));
                    let mut line = String::new();
                    let read_ok = {
                        let mut reader = BufReader::new(&stream);
                        reader.read_line(&mut line).is_ok()
                    };
                    if read_ok {
                        let resp = match IpcCommand::decode(&line) {
                            Some(cmd) => handle(cmd),
                            None => String::from("ERR invalid command\n"),
                        };
                        let _ = stream.write_all(resp.as_bytes());
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(_) => break,
            }
        }
    }
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Sends a single command to an already-running Kadex daemon.
/// Returns `Ok(response)` if a daemon handled it, or `Err` if no daemon is running.
pub fn send_command(socket_path: &Path, cmd: &IpcCommand) -> io::Result<String> {
    let mut stream = UnixStream::connect(socket_path)?;
    stream.set_read_timeout(Some(Duration::from_millis(500)))?;
    stream.set_write_timeout(Some(Duration::from_millis(500)))?;
    let wire = cmd.encode();
    stream.write_all(wire.as_bytes())?;
    let _ = stream.shutdown(std::net::Shutdown::Write);

    let mut out = String::new();
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    while reader.read_line(&mut line)? > 0 {
        out.push_str(&line);
        line.clear();
    }
    Ok(out)
}
