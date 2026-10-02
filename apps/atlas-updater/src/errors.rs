//! Helper errors in plain language.

use atlas_core::helper_client::{Error, HelperErrorKind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpError {
    /// The password prompt was dismissed, or policy denied the request (the
    /// helper cannot tell these apart). Shown as a short note, not an error.
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
            HelperErrorKind::ShuttingDown => {
                OpError::Message("The system helper was just shutting down. Try again.".into())
            }
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
        Error::DBus(e) => OpError::Message(dbus_message(e)),
        Error::Parse(_) => OpError::Message(
            "The system helper sent an answer Atlas Updater could not read.".into(),
        ),
    }
}

/// Authorization needed by `Cancelled`, in words.
pub const DENIED_TEXT: &str = "Authorization was cancelled or denied. Nothing was changed.";

fn dbus_message(e: &zbus::Error) -> String {
    if let zbus::Error::MethodError(name, _, _) = e {
        match name.as_str() {
            "org.freedesktop.DBus.Error.AccessDenied" => {
                return "The system did not allow this account to talk to the update helper."
                    .into();
            }
            "org.freedesktop.DBus.Error.NoReply" | "org.freedesktop.DBus.Error.Timeout" => {
                return "The system helper did not answer in time. Try again.".into();
            }
            _ => {}
        }
    }
    "Can't reach the Atlas system helper. It may be missing, or this system may not be an AtlasOS install.".into()
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

    #[test]
    fn dbus_denials_and_timeouts_have_their_own_text() {
        let denied = zbus::Error::MethodError(
            "org.freedesktop.DBus.Error.AccessDenied"
                .try_into()
                .unwrap(),
            None,
            zbus::Message::method_call("/", "X")
                .unwrap()
                .build(&())
                .unwrap(),
        );
        assert!(dbus_message(&denied).contains("did not allow"));
        assert!(dbus_message(&zbus::Error::Unsupported).contains("Can't reach"));
    }
}
