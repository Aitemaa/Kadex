//! Cross-platform line-based IPC command protocol.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IpcCommand {
    /// Show existing sticky notes or create one if none are open.
    Activate,
    /// Spawn a brand new sticky note.
    NewSticky,
    /// Open a regular file in File Mode.
    OpenFile(String),
    /// Focus or open a specific sticky note by ID.
    OpenNote(String),
    /// Toggle visibility of all sticky notes.
    Toggle,
    /// Hide all sticky notes.
    Hide,
    /// List sticky notes (`id\tfirst line`).
    List,
    /// Toggle Wayland layer-shell overlay scratchpad.
    Overlay,
    /// Health check.
    Ping,
    /// Cleanly flush and quit daemon.
    Quit,
}

impl IpcCommand {
    pub fn encode(&self) -> String {
        match self {
            IpcCommand::Activate => String::from("ACTIVATE\n"),
            IpcCommand::NewSticky => String::from("NEW\n"),
            IpcCommand::OpenFile(p) => format!("OPEN_FILE {}\n", sanitize_line(p)),
            IpcCommand::OpenNote(id) => format!("OPEN_NOTE {}\n", sanitize_line(id)),
            IpcCommand::Toggle => String::from("TOGGLE\n"),
            IpcCommand::Hide => String::from("HIDE\n"),
            IpcCommand::List => String::from("LIST\n"),
            IpcCommand::Overlay => String::from("OVERLAY\n"),
            IpcCommand::Ping => String::from("PING\n"),
            IpcCommand::Quit => String::from("QUIT\n"),
        }
    }

    pub fn decode(line: &str) -> Option<Self> {
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            return None;
        }
        if let Some((cmd, arg)) = trimmed.split_once(' ') {
            match cmd {
                "OPEN_FILE" if !arg.is_empty() => Some(IpcCommand::OpenFile(arg.to_string())),
                "OPEN_NOTE" if !arg.is_empty() => Some(IpcCommand::OpenNote(arg.to_string())),
                _ => None,
            }
        } else {
            match trimmed {
                "ACTIVATE" => Some(IpcCommand::Activate),
                "NEW" => Some(IpcCommand::NewSticky),
                "TOGGLE" => Some(IpcCommand::Toggle),
                "HIDE" => Some(IpcCommand::Hide),
                "LIST" => Some(IpcCommand::List),
                "OVERLAY" => Some(IpcCommand::Overlay),
                "PING" => Some(IpcCommand::Ping),
                "QUIT" => Some(IpcCommand::Quit),
                _ => None,
            }
        }
    }
}

fn sanitize_line(s: &str) -> String {
    s.replace(['\r', '\n'], " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_decode_roundtrip() {
        let cmds = [
            IpcCommand::Activate,
            IpcCommand::NewSticky,
            IpcCommand::OpenFile("/tmp/hello world.txt".into()),
            IpcCommand::OpenNote("20261123-01".into()),
            IpcCommand::Toggle,
            IpcCommand::Hide,
            IpcCommand::List,
            IpcCommand::Overlay,
            IpcCommand::Ping,
            IpcCommand::Quit,
        ];
        for c in cmds {
            let wire = c.encode();
            assert_eq!(IpcCommand::decode(&wire), Some(c));
        }
    }
}
