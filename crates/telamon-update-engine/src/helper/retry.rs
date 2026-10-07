//! Retrying the network half of an operation.
//!
//! bootc, rpm-ostree and skopeo fetch from a registry, and a dropped
//! connection, a DNS hiccup or a registry's 502/503 fails the whole
//! operation. Each of the fetching steps (`upgrade --check`, `upgrade`,
//! `switch`, `rebase`, `skopeo inspect`) is idempotent: ostree commits each
//! layer as it arrives and stages atomically, so a second run picks up where
//! the first stopped. So the helper runs such a step up to [`ATTEMPTS`]
//! times, waiting between tries, but only when the error reads as a passing
//! network problem. Anything else (a signature or policy refusal, a missing
//! image, a full disk, a denied login, an interrupted bootc) fails at once:
//! trying again would not change it. `bootc rollback` is never retried (it
//! toggles).

use std::time::Duration;

/// Tries in all, the first included.
pub const ATTEMPTS: usize = 3;

/// The waits before the second and third try.
pub const DELAYS: [Duration; ATTEMPTS - 1] = [Duration::from_secs(3), Duration::from_secs(10)];

/// How many of the error's last non-empty lines say what failed: the tools
/// log warnings (an internal retry after a reset connection) before the
/// error that ended them.
const LAST_LINES: usize = 3;

/// Phrases (lowercase) of errors that a later try can get past: connection
/// and DNS failures, timeouts, and the registry's own "try later" answers,
/// as Go's net/http (skopeo, containers-image-proxy), glib (ostree) and
/// libcurl word them.
const TRANSIENT: &[&str] = &[
    "connection reset",
    "connection refused",
    "connection closed",
    "connection timed out",
    "connection terminated unexpectedly",
    "broken pipe",
    "i/o timeout",
    "socket i/o timed out",
    "operation timed out",
    "tls handshake timeout",
    "timeout was reached",
    "context deadline exceeded",
    "client.timeout exceeded",
    "temporary failure in name resolution",
    "could not resolve host",
    "name or service not known",
    "server misbehaving",
    "no such host",
    "network is unreachable",
    "no route to host",
    "unexpected eof",
    "http2: server sent goaway",
    "http2: client connection lost",
    "500 internal server error",
    "502 bad gateway",
    "503 service unavailable",
    "504 gateway timeout",
    "504 gateway time-out",
    "http 502",
    "http 503",
    "http 504",
    "status: 502",
    "status: 503",
    "status: 504",
    "429 too many requests",
    "toomanyrequests",
];

/// Phrases of a signature that was checked and found wanting (not one that
/// couldn't be fetched).
pub const SIGNATURE_REFUSED: &[&str] = &[
    "a signature was required",
    "signature for identity",
    "invalid signature",
    "signature verification failed",
];

/// Phrases, anywhere in the error, that mean a retry can't help even next
/// to a transient-looking one (a signature that doesn't verify is still
/// wrong after a reset connection).
const PERMANENT: &[&str] = &[
    "denied",
    "unauthorized",
    "manifest unknown",
    "no space left",
    "min-free-space",
    "read-only file system",
    "local rpm-ostree modifications",
];

/// What [`retrying`] puts between a later try's error and the network error
/// of the first; what follows it is not judged again.
pub const AFTER_NOTE: &str = "\n(after a try that failed with: ";

/// `error` without the note [`retrying`] may have added.
pub fn final_error(error: &str) -> &str {
    error.split(AFTER_NOTE).next().unwrap_or(error)
}

/// True when `error` reads as a passing network problem worth another try:
/// its last lines name one, and nothing in it says retrying can't help.
/// (Telamon OS's update-stage has a shell copy of these lists.)
pub fn is_transient(error: &str) -> bool {
    let e = final_error(error).to_lowercase();
    if SIGNATURE_REFUSED
        .iter()
        .chain(PERMANENT)
        .any(|p| e.contains(p))
    {
        return false;
    }
    let lines: Vec<&str> = e.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    lines[lines.len().saturating_sub(LAST_LINES)..]
        .iter()
        // Go's bare `Get "https://...": EOF`
        .any(|l| TRANSIENT.iter().any(|p| l.contains(p)) || l.ends_with(": eof"))
}

/// Run `step` until it succeeds, fails with an error that is not
/// [`is_transient`], or has run [`ATTEMPTS`] times; `wait` sleeps between
/// tries and returns false to stop early (the helper is shutting down),
/// which ends with [`INTERRUPTED`](super::INTERRUPTED). When a later try
/// fails another way (rpm-ostree still busy with the first), the network
/// error that started it is said too.
pub fn retrying<T>(
    mut step: impl FnMut() -> Result<T, String>,
    mut wait: impl FnMut(Duration) -> bool,
) -> Result<T, String> {
    let mut first: Option<String> = None;
    let mut tries = 0;
    loop {
        tries += 1;
        match step() {
            Err(e) if tries < ATTEMPTS && is_transient(&e) => {
                eprintln!("telamon-system-helper: try {tries} failed, trying again: {e}");
                if !wait(DELAYS[tries - 1]) {
                    return Err(super::INTERRUPTED.into());
                }
                first.get_or_insert(e);
            }
            Err(e) => {
                return Err(match first {
                    Some(f) if !is_transient(&e) => format!("{e}{AFTER_NOTE}{f})"),
                    _ => e,
                });
            }
            ok => return ok,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn network_failures_are_transient() {
        for e in [
            "error: Upgrading: Creating importer: failed to invoke method OpenImage: pinging container registry ghcr.io: Get \"https://ghcr.io/v2/\": dial tcp 140.82.121.34:443: i/o timeout",
            "reading blob sha256:abc: Get \"https://pkg-containers.githubusercontent.com/...\": read tcp 10.0.0.2:51234->185.199.108.154:443: read: connection reset by peer",
            "error: Fetching: dial tcp: lookup ghcr.io: Temporary failure in name resolution",
            "dial tcp: lookup ghcr.io on 127.0.0.53:53: server misbehaving",
            "Error resolving “ghcr.io”: Name or service not known",
            "Could not resolve host: ghcr.io",
            "Operation timed out after 30000 milliseconds with 0 bytes received",
            "Socket I/O timed out",
            "Server returned HTTP 503",
            "received unexpected HTTP status: 503 Service Unavailable",
            "received unexpected HTTP status: 504 Gateway Time-out",
            "unexpected EOF",
            "Get \"https://ghcr.io/v2/e/atlasos/manifests/stable\": EOF",
            "Timeout was reached",
            "error: Fetching layer: TOOMANYREQUESTS: retry later",
            // the signature couldn't be fetched, not found wrong
            "Source image rejected: reading signatures: Get \"https://ghcr.io/v2/...\": read: connection reset by peer",
        ] {
            assert!(is_transient(e), "{e}");
        }
    }

    #[test]
    fn everything_else_fails_at_once() {
        for e in [
            "error: Upgrading: Deployment contains local rpm-ostree modifications; cannot upgrade via bootc",
            "Source image rejected: A signature was required, but no signature exists",
            // a signature failure after a reset is still a signature failure
            "Source image rejected: Signature for identity ... connection reset",
            "reading manifest stable in ghcr.io/x/atlasos: manifest unknown",
            "writing blob: No space left on device",
            "error: min-free-space-percent '3%' would be exceeded",
            "requested access to the resource is denied",
            // a warning about a retry the tool made itself, then another error
            "time=\"...\" level=warning msg=\"Failed, retrying in 1s ... connection reset by peer\"\nchoosing an image from manifest list: no image found in manifest list for architecture amd64\nerror: Upgrading: exit status 1\nerror: pulling failed",
            crate::helper::INTERRUPTED,
            "boom",
            "",
        ] {
            assert!(!is_transient(e), "{e}");
        }
    }

    #[test]
    fn retries_a_transient_failure_with_the_delays() {
        let waits = RefCell::new(Vec::new());
        let mut n = 0;
        let r = retrying(
            || {
                n += 1;
                if n < 3 {
                    Err("connection reset by peer".to_string())
                } else {
                    Ok(n)
                }
            },
            |d| {
                waits.borrow_mut().push(d);
                true
            },
        );
        assert_eq!(r, Ok(3));
        assert_eq!(*waits.borrow(), DELAYS);
    }

    #[test]
    fn gives_up_after_the_last_try_with_its_error() {
        let mut n = 0;
        let r: Result<(), String> = retrying(
            || {
                n += 1;
                Err(format!("i/o timeout {n}"))
            },
            |_| true,
        );
        assert_eq!(r, Err("i/o timeout 3".into()));
        assert_eq!(n, ATTEMPTS);
    }

    #[test]
    fn a_later_failure_of_another_kind_keeps_the_network_error() {
        let mut n = 0;
        let r: Result<(), String> = retrying(
            || {
                n += 1;
                Err(if n == 1 {
                    "connection reset by peer".into()
                } else {
                    "error: Transaction in progress: upgrade".into()
                })
            },
            |_| true,
        );
        let e = r.unwrap_err();
        assert!(e.starts_with("error: Transaction in progress"), "{e}");
        assert!(e.ends_with("failed with: connection reset by peer)"), "{e}");
        assert_eq!(n, 2);
        // the note doesn't make it a network error
        assert!(!is_transient(&e));
        assert_eq!(final_error(&e), "error: Transaction in progress: upgrade");
    }

    #[test]
    fn a_permanent_failure_or_a_stop_is_not_retried() {
        let mut n = 0;
        let r: Result<(), String> = retrying(
            || {
                n += 1;
                Err("manifest unknown".into())
            },
            |_| true,
        );
        assert_eq!(r, Err("manifest unknown".into()));
        assert_eq!(n, 1);
        let mut n = 0;
        let r: Result<(), String> = retrying(
            || {
                n += 1;
                Err("connection refused".into())
            },
            |_| false,
        );
        assert_eq!(r, Err(crate::helper::INTERRUPTED.into()));
        assert_eq!(n, 1);
    }
}
