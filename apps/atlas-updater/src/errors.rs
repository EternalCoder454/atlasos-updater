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
            // bootc did it, only the follow-up read failed: not a failure
            HelperErrorKind::Failed
                if message.starts_with(atlas_core::helper_client::STATE_UNREAD) =>
            {
                OpError::Message(format!(
                    "{} Restart to finish going back, or check again in a moment.",
                    atlas_core::helper_client::STATE_UNREAD
                ))
            }
            HelperErrorKind::Failed
                if message.starts_with(atlas_core::helper_client::STATE_UNREAD_CANCEL) =>
            {
                OpError::Message(atlas_core::helper_client::STATE_UNREAD_CANCEL.into())
            }
            // the helper's own plain refusals (nothing was run)
            HelperErrorKind::Failed
                if message == atlas_core::helper_client::ROLLBACK_ALREADY_QUEUED
                    || message == atlas_core::helper_client::NO_ROLLBACK_QUEUED =>
            {
                OpError::Message(message.clone())
            }
            // an older build on the server: taken out again, said plainly
            HelperErrorKind::Failed
                if message.starts_with(atlas_core::helper_client::DOWNGRADE_REFUSED) =>
            {
                OpError::Message(message.clone())
            }
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

/// What `Cancelled` says for `action` ("check for updates"): polkit refuses
/// without a prompt in an inactive or remote session, and a dismissed prompt
/// looks the same, so the text covers both. Shown as an error, never silently.
pub fn denied_text(action: &str) -> String {
    format!(
        "You aren't allowed to {action} from this session, or the password prompt was canceled. Nothing was changed."
    )
}

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
    fn a_denial_names_the_action() {
        let t = denied_text("check for updates");
        assert!(t.starts_with("You aren't allowed to check for updates from this session"));
        assert!(t.contains("Nothing was changed"));
    }

    #[test]
    fn rollback_refusals_are_shown_plainly() {
        let e = Error::Helper {
            kind: HelperErrorKind::Failed,
            message: atlas_core::helper_client::ROLLBACK_ALREADY_QUEUED.into(),
        };
        assert_eq!(
            friendly(&e),
            OpError::Message(atlas_core::helper_client::ROLLBACK_ALREADY_QUEUED.into())
        );
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

    #[test]
    fn a_refused_downgrade_is_said_as_the_helper_says_it() {
        let message = format!(
            "{} (found 44.20260920, installed 44.20261001)",
            atlas_core::helper_client::DOWNGRADE_REFUSED
        );
        let e = Error::Helper {
            kind: HelperErrorKind::Failed,
            message: message.clone(),
        };
        assert_eq!(friendly(&e), OpError::Message(message));
    }
}
