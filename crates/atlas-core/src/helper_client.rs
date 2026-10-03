//! Client for the `net.eterneon.atlas.SystemHelper1` D-Bus service.
//!
//! The calls are async and run on the zbus tokio executor, so call them from
//! inside a tokio runtime. There is no method timeout: `Upgrade` downloads an
//! image and can take minutes.

use std::fmt;
use std::pin::Pin;
use std::task::{Context, Poll};

use crate::bootc::{self, Channel, Status};
use crate::progress::Progress;
use zbus::export::futures_core::Stream;

pub const BUS_NAME: &str = "net.eterneon.atlas.SystemHelper";
pub const OBJECT_PATH: &str = "/net/eterneon/atlas/SystemHelper";
pub const INTERFACE: &str = "net.eterneon.atlas.SystemHelper1";

/// What the helper says when asked to queue a rollback that is already queued.
pub const ROLLBACK_ALREADY_QUEUED: &str =
    "A rollback is already queued. Restart to go back, or cancel it first.";
/// Start of the error when bootc queued the rollback but the new state could
/// not be read afterwards: going back is set up, nothing failed.
pub const STATE_UNREAD: &str = "Going back is set up, but AtlasOS couldn't read the new state.";
/// The same for cancelling a queued rollback.
pub const STATE_UNREAD_CANCEL: &str =
    "The rollback was canceled, but AtlasOS couldn't read the new state.";
/// What the helper says when asked to cancel a rollback that is not queued.
pub const NO_ROLLBACK_QUEUED: &str = "No rollback is queued.";

/// Raw proxy: every method returns the `bootc status --json` text.
#[zbus::proxy(
    interface = "net.eterneon.atlas.SystemHelper1",
    default_service = "net.eterneon.atlas.SystemHelper",
    default_path = "/net/eterneon/atlas/SystemHelper",
    gen_blocking = false
)]
pub trait SystemHelper1 {
    fn status(&self) -> zbus::Result<String>;
    fn check_for_update(&self) -> zbus::Result<String>;
    fn upgrade(&self) -> zbus::Result<String>;
    fn rollback(&self) -> zbus::Result<String>;
    fn cancel_rollback(&self) -> zbus::Result<String>;
    fn switch_channel(&self, channel: &str) -> zbus::Result<String>;

    /// JSON of the current progress while an upgrade or switch runs, else "".
    #[zbus(property)]
    fn progress(&self) -> zbus::Result<String>;
}

/// The helper's error names (`net.eterneon.atlas.Error.*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelperErrorKind {
    InvalidArgument,
    NotAuthorized,
    Busy,
    /// The helper was exiting; the client already retried once.
    ShuttingDown,
    Failed,
}

#[derive(Debug)]
pub enum Error {
    /// The helper refused or failed. `message` is for display.
    Helper {
        kind: HelperErrorKind,
        message: String,
    },
    /// Bus or transport problem (helper missing, no system bus, ...).
    DBus(zbus::Error),
    /// The helper answered with JSON that is not a bootc status.
    Parse(serde_json::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Helper { kind, message } => write!(f, "{kind:?}: {message}"),
            Error::DBus(e) => write!(f, "D-Bus error: {e}"),
            Error::Parse(e) => write!(f, "bad status JSON from the helper: {e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<zbus::Error> for Error {
    fn from(e: zbus::Error) -> Self {
        if let zbus::Error::MethodError(name, desc, _) = &e {
            let kind = match name.as_str() {
                "net.eterneon.atlas.Error.InvalidArgument" => {
                    Some(HelperErrorKind::InvalidArgument)
                }
                "net.eterneon.atlas.Error.NotAuthorized" => Some(HelperErrorKind::NotAuthorized),
                "net.eterneon.atlas.Error.Busy" => Some(HelperErrorKind::Busy),
                "net.eterneon.atlas.Error.ShuttingDown" => Some(HelperErrorKind::ShuttingDown),
                "net.eterneon.atlas.Error.Failed" => Some(HelperErrorKind::Failed),
                _ => None,
            };
            if let Some(kind) = kind {
                return Error::Helper {
                    kind,
                    message: desc.clone().unwrap_or_default(),
                };
            }
        }
        Error::DBus(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// Typed client. Each method returns the parsed status after the action.
#[derive(Clone)]
pub struct HelperClient {
    proxy: SystemHelper1Proxy<'static>,
}

impl HelperClient {
    /// Connect to the helper on the system bus (D-Bus activates it).
    pub async fn connect() -> Result<Self> {
        Self::with_connection(&zbus::Connection::system().await?).await
    }

    /// Use an existing connection (for tests on a private bus).
    pub async fn with_connection(conn: &zbus::Connection) -> Result<Self> {
        Ok(Self {
            proxy: SystemHelper1Proxy::new(conn).await?,
        })
    }

    /// Run `f`; if the helper answers `ShuttingDown` (it was exiting), wait a
    /// moment, reconnect the proxy (D-Bus starts a fresh helper) and retry once.
    async fn call<F, Fut>(&self, f: F) -> Result<Status>
    where
        F: Fn(SystemHelper1Proxy<'static>) -> Fut,
        Fut: std::future::Future<Output = zbus::Result<String>>,
    {
        match f(self.proxy.clone()).await.map_err(Error::from) {
            Err(Error::Helper {
                kind: HelperErrorKind::ShuttingDown,
                ..
            }) => {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                let conn = self.proxy.inner().connection().clone();
                let proxy = SystemHelper1Proxy::new(&conn).await?;
                parse(f(proxy).await?)
            }
            other => parse(other?),
        }
    }

    /// `bootc status --json`.
    pub async fn status(&self) -> Result<Status> {
        self.call(|p| async move { p.status().await }).await
    }

    /// `bootc upgrade --check`; a found update shows in
    /// [`Status::available_update`].
    pub async fn check_for_update(&self) -> Result<Status> {
        self.call(|p| async move { p.check_for_update().await })
            .await
    }

    /// Download and stage the update (never reboots).
    pub async fn upgrade(&self) -> Result<Status> {
        self.call(|p| async move { p.upgrade().await }).await
    }

    /// Queue a rollback to the previous deployment. Refused when one is
    /// already queued (`bootc rollback` would cancel it).
    pub async fn rollback(&self) -> Result<Status> {
        self.call(|p| async move { p.rollback().await }).await
    }

    /// Cancel a queued rollback; refused when none is queued.
    pub async fn cancel_rollback(&self) -> Result<Status> {
        self.call(|p| async move { p.cancel_rollback().await })
            .await
    }

    /// Switch the booted image's tag to `stable` or `testing`.
    pub async fn switch_channel(&self, channel: Channel) -> Result<Status> {
        self.call(|p| async move { p.switch_channel(channel.as_str()).await })
            .await
    }
}

impl HelperClient {
    /// The progress of the running upgrade or switch, `None` when nothing runs.
    pub async fn progress(&self) -> Result<Option<Progress>> {
        parse_progress(&self.proxy.progress().await?)
    }

    /// Every change of the progress; `None` when it goes back to nothing.
    /// Items that cannot be read are skipped. Ends when the stream is dropped
    /// or the connection closes.
    pub async fn progress_changes(&self) -> Result<impl Stream<Item = Option<Progress>> + use<>> {
        let mut changes = self.proxy.receive_progress_changed().await;
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        tokio::spawn(async move {
            while let Some(c) =
                std::future::poll_fn(|cx| Pin::new(&mut changes).poll_next(cx)).await
            {
                let Ok(json) = c.get().await else { continue };
                let Ok(p) = parse_progress(&json) else {
                    continue;
                };
                if tx.send(p).is_err() {
                    break;
                }
            }
        });
        Ok(ProgressStream(rx))
    }
}

struct ProgressStream(tokio::sync::mpsc::UnboundedReceiver<Option<Progress>>);

impl Stream for ProgressStream {
    type Item = Option<Progress>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.0.poll_recv(cx)
    }
}

fn parse_progress(json: &str) -> Result<Option<Progress>> {
    if json.is_empty() {
        return Ok(None);
    }
    serde_json::from_str(json).map(Some).map_err(Error::Parse)
}

fn parse(json: String) -> Result<bootc::Status> {
    let mut st = Status::from_json(&json).map_err(Error::Parse)?;
    st.image_ref_heads = bootc::image_ref_heads(std::path::Path::new(bootc::IMAGE_REFS_DIR));
    st.bad_image_digests = bootc::bad_image_digests(std::path::Path::new(bootc::BAD_IMAGE_DIGESTS));
    Ok(st)
}
