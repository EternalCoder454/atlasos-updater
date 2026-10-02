//! Helper errors in plain language.

use atlas_core::helper_client::{Error, HelperErrorKind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpError {
    /// The user closed the password prompt. Not an error.
    Cancelled,
    Message(String),
}

fn tail(s: &str, lines: usize) -> String {
    let all: Vec<&str> = s.trim().lines().collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}

pub fn friendly(e: &Error) -> OpError {
    match e {
        Error::Helper { kind, message } => match kind {
            HelperErrorKind::NotAuthorized => OpError::Cancelled,
            HelperErrorKind::Busy => OpError::Message(
                "Another update task is already running. Try again when it finishes.".into(),
            ),
            HelperErrorKind::ShuttingDown => OpError::Message(
                "The system helper was just shutting down. Try again.".into(),
            ),
            HelperErrorKind::InvalidArgument => OpError::Message(
                "The system helper turned the request down. This is a bug in Atlas Updater.".into(),
            ),
            HelperErrorKind::Failed => {
                let detail = tail(message, 4);
                if detail.is_empty() {
                    OpError::Message("The update tool failed without saying why.".into())
                } else {
                    OpError::Message(format!("The update tool reported a problem:\n{detail}"))
                }
            }
        },
        Error::DBus(_) => OpError::Message(
            "Can't reach the Atlas system helper. It may be missing, or this system may not be an AtlasOS install.".into(),
        ),
        Error::Parse(_) => OpError::Message(
            "The system helper sent an answer Atlas Updater could not read.".into(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_authorized_is_not_an_error() {
        let e = Error::Helper {
            kind: HelperErrorKind::NotAuthorized,
            message: String::new(),
        };
        assert_eq!(friendly(&e), OpError::Cancelled);
    }

    #[test]
    fn failed_keeps_last_lines() {
        let e = Error::Helper {
            kind: HelperErrorKind::Failed,
            message: "a\nb\nc\nd\ne\nf".into(),
        };
        match friendly(&e) {
            OpError::Message(m) => assert!(m.ends_with("c\nd\ne\nf") && !m.contains("\nb\n")),
            _ => panic!(),
        }
    }
}
