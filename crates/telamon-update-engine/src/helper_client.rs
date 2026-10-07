//! Client for the `net.eterneon.telamon.SystemHelper1` D-Bus service.
//!
//! The calls are async and run on the zbus tokio executor, so call them from
//! inside a tokio runtime. There is no method timeout: `Upgrade` downloads an
//! image and can take minutes.
//!
//! The client talks to the new identity. If the new name has no owner and no
//! activation file (a helper from before the rename is installed), it uses
//! the old one, `net.eterneon.atlas.SystemHelper`, which that helper serves
//! (and a new one serves too, for this release). Decided once, when
//! connecting; see [`crate::identity`].

use std::fmt;
use std::pin::Pin;
use std::task::{Context, Poll};

use crate::identity::Identity;
use crate::progress::Progress;
use telamon_framework_system::bootc::{self, Channel, Status};
use zbus::export::futures_core::Stream;

pub const BUS_NAME: &str = Identity::Telamon.bus_name();
pub const OBJECT_PATH: &str = Identity::Telamon.object_path();
pub const INTERFACE: &str = Identity::Telamon.interface();

/// What the helper says when asked to queue a rollback that is already queued.
pub const ROLLBACK_ALREADY_QUEUED: &str =
    "A rollback is already queued. Restart to go back, or cancel it first.";
/// Start of the error when bootc queued the rollback but the new state could
/// not be read afterwards: going back is set up, nothing failed.
pub const STATE_UNREAD: &str = "Going back is set up, but Telamon OS couldn't read the new state.";
/// The same for cancelling a queued rollback.
pub const STATE_UNREAD_CANCEL: &str =
    "The rollback was canceled, but Telamon OS couldn't read the new state.";
/// What the helper says when asked to cancel a rollback that is not queued.
pub const NO_ROLLBACK_QUEUED: &str = "No rollback is queued.";
/// Start of the error when `Upgrade` or `SwitchChannel` found an image older
/// than the one this computer runs (the version follows): refused before the
/// download when the registry could be asked, otherwise taken out again after.
pub const DOWNGRADE_REFUSED: &str =
    "The update on the server is older than the version on this computer, so it wasn't installed.";

/// Raw proxy: every method returns the `bootc status --json` text.
#[zbus::proxy(
    interface = "net.eterneon.telamon.SystemHelper1",
    default_service = "net.eterneon.telamon.SystemHelper",
    default_path = "/net/eterneon/telamon/SystemHelper",
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

/// The same under the old names, for a helper from before the rename.
#[zbus::proxy(
    interface = "net.eterneon.atlas.SystemHelper1",
    default_service = "net.eterneon.atlas.SystemHelper",
    default_path = "/net/eterneon/atlas/SystemHelper",
    gen_blocking = false
)]
pub trait LegacySystemHelper1 {
    fn status(&self) -> zbus::Result<String>;
    fn check_for_update(&self) -> zbus::Result<String>;
    fn upgrade(&self) -> zbus::Result<String>;
    fn rollback(&self) -> zbus::Result<String>;
    fn cancel_rollback(&self) -> zbus::Result<String>;
    fn switch_channel(&self, channel: &str) -> zbus::Result<String>;

    #[zbus(property)]
    fn progress(&self) -> zbus::Result<String>;
}

/// The proxy of the identity the client uses.
#[derive(Clone)]
enum Proxy {
    Telamon(SystemHelper1Proxy<'static>),
    Legacy(LegacySystemHelper1Proxy<'static>),
}

macro_rules! each {
    ($self:expr, $p:ident => $e:expr) => {
        match $self {
            Proxy::Telamon($p) => $e,
            Proxy::Legacy($p) => $e,
        }
    };
}

impl Proxy {
    async fn new(conn: &zbus::Connection, identity: Identity) -> zbus::Result<Proxy> {
        Ok(match identity {
            Identity::Telamon => Proxy::Telamon(SystemHelper1Proxy::new(conn).await?),
            Identity::Legacy => Proxy::Legacy(LegacySystemHelper1Proxy::new(conn).await?),
        })
    }

    fn identity(&self) -> Identity {
        match self {
            Proxy::Telamon(_) => Identity::Telamon,
            Proxy::Legacy(_) => Identity::Legacy,
        }
    }

    fn connection(&self) -> &zbus::Connection {
        each!(self, p => p.inner().connection())
    }

    async fn status(&self) -> zbus::Result<String> {
        each!(self, p => p.status().await)
    }

    async fn check_for_update(&self) -> zbus::Result<String> {
        each!(self, p => p.check_for_update().await)
    }

    async fn upgrade(&self) -> zbus::Result<String> {
        each!(self, p => p.upgrade().await)
    }

    async fn rollback(&self) -> zbus::Result<String> {
        each!(self, p => p.rollback().await)
    }

    async fn cancel_rollback(&self) -> zbus::Result<String> {
        each!(self, p => p.cancel_rollback().await)
    }

    async fn switch_channel(&self, channel: &str) -> zbus::Result<String> {
        each!(self, p => p.switch_channel(channel).await)
    }

    async fn progress(&self) -> zbus::Result<String> {
        each!(self, p => p.progress().await)
    }

    async fn receive_progress_changed(&self) -> zbus::proxy::PropertyStream<'static, String> {
        each!(self, p => p.receive_progress_changed().await)
    }
}

/// The helper's error names: `net.eterneon.telamon.Error.*`, or the old
/// `net.eterneon.atlas.Error.*` from a helper that answers through the old
/// identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelperErrorKind {
    InvalidArgument,
    NotAuthorized,
    Busy,
    /// The helper was exiting; the client already retried once.
    ShuttingDown,
    Failed,
}

impl HelperErrorKind {
    /// The kind a D-Bus error name stands for, under either identity.
    pub fn from_dbus_name(name: &str) -> Option<HelperErrorKind> {
        let (prefix, kind) = name.rsplit_once('.')?;
        if !Identity::ALL.iter().any(|i| i.error_prefix() == prefix) {
            return None;
        }
        match kind {
            "InvalidArgument" => Some(HelperErrorKind::InvalidArgument),
            "NotAuthorized" => Some(HelperErrorKind::NotAuthorized),
            "Busy" => Some(HelperErrorKind::Busy),
            "ShuttingDown" => Some(HelperErrorKind::ShuttingDown),
            "Failed" => Some(HelperErrorKind::Failed),
            _ => None,
        }
    }
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
        if let zbus::Error::MethodError(name, desc, _) = &e
            && let Some(kind) = HelperErrorKind::from_dbus_name(name.as_str())
        {
            return Error::Helper {
                kind,
                message: desc.clone().unwrap_or_default(),
            };
        }
        Error::DBus(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// Typed client. Each method returns the parsed status after the action.
#[derive(Clone)]
pub struct HelperClient {
    proxy: Proxy,
}

/// Whether a call to `identity`'s name would reach a helper: someone owns the
/// name, or D-Bus can start one for it. A bus that cannot say counts as yes.
async fn reachable(dbus: &zbus::fdo::DBusProxy<'_>, identity: Identity) -> zbus::Result<bool> {
    let name = zbus::names::BusName::try_from(identity.bus_name())?;
    if dbus.name_has_owner(name).await? {
        return Ok(true);
    }
    Ok(dbus
        .list_activatable_names()
        .await
        .map(|names| names.iter().any(|n| n.as_str() == identity.bus_name()))
        .unwrap_or(true))
}

/// The identity to call on `conn`: the new one, unless nothing answers to its
/// name (no owner, no activation file) but the old name does, which is what a
/// helper from before the rename leaves. When neither does, the new one: the
/// call then fails the way it always did.
pub async fn choose_identity(conn: &zbus::Connection) -> zbus::Result<Identity> {
    let dbus = zbus::fdo::DBusProxy::new(conn).await?;
    let [new, old] = Identity::ALL;
    if reachable(&dbus, new).await? {
        return Ok(new);
    }
    if reachable(&dbus, old).await? {
        return Ok(old);
    }
    Ok(new)
}

impl HelperClient {
    /// Connect to the helper on the system bus (D-Bus activates it).
    pub async fn connect() -> Result<Self> {
        Self::with_connection(&zbus::Connection::system().await?).await
    }

    /// Use an existing connection (for tests on a private bus).
    pub async fn with_connection(conn: &zbus::Connection) -> Result<Self> {
        let identity = choose_identity(conn).await?;
        Self::with_identity(conn, identity).await
    }

    /// Use `identity` whatever the bus says (tests).
    pub async fn with_identity(conn: &zbus::Connection, identity: Identity) -> Result<Self> {
        Ok(Self {
            proxy: Proxy::new(conn, identity).await?,
        })
    }

    /// The identity this client calls.
    pub fn identity(&self) -> Identity {
        self.proxy.identity()
    }

    /// Run `f`; if the helper answers `ShuttingDown` (it was exiting), wait a
    /// moment, reconnect the proxy (D-Bus starts a fresh helper) and retry once.
    async fn call<F, Fut>(&self, f: F) -> Result<Status>
    where
        F: Fn(Proxy) -> Fut,
        Fut: std::future::Future<Output = zbus::Result<String>>,
    {
        match f(self.proxy.clone()).await.map_err(Error::from) {
            Err(Error::Helper {
                kind: HelperErrorKind::ShuttingDown,
                ..
            }) => {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                let conn = self.proxy.connection().clone();
                let proxy = Proxy::new(&conn, self.proxy.identity()).await?;
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

    /// Every change of the progress (no initial value: read that with
    /// [`progress`](Self::progress) first); `None` when it goes back to nothing.
    /// Items that cannot be read are skipped. Ends when the stream is dropped
    /// or the connection closes.
    pub async fn progress_changes(&self) -> Result<impl Stream<Item = Option<Progress>> + use<>> {
        let mut changes = self.proxy.receive_progress_changed().await;
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        tokio::spawn(async move {
            loop {
                let next = std::future::poll_fn(|cx| Pin::new(&mut changes).poll_next(cx));
                let c = tokio::select! {
                    // the stream was dropped: stop listening
                    () = tx.closed() => break,
                    c = next => match c {
                        Some(c) => c,
                        None => break,
                    },
                };
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testbus::TestBus;

    #[test]
    fn error_names_of_both_identities_map_to_the_same_kinds() {
        for (suffix, kind) in [
            ("InvalidArgument", HelperErrorKind::InvalidArgument),
            ("NotAuthorized", HelperErrorKind::NotAuthorized),
            ("Busy", HelperErrorKind::Busy),
            ("ShuttingDown", HelperErrorKind::ShuttingDown),
            ("Failed", HelperErrorKind::Failed),
        ] {
            for id in Identity::ALL {
                assert_eq!(
                    HelperErrorKind::from_dbus_name(&format!("{}.{suffix}", id.error_prefix())),
                    Some(kind)
                );
            }
        }
        for other in [
            "net.eterneon.telamon.Error.Nope",
            "net.eterneon.other.Error.Busy",
            "org.freedesktop.DBus.Error.ServiceUnknown",
            "Busy",
            "",
        ] {
            assert_eq!(HelperErrorKind::from_dbus_name(other), None, "{other}");
        }
        // and a zbus method error turns into a helper error either way
        for id in Identity::ALL {
            let e = zbus::Error::MethodError(
                zbus::names::OwnedErrorName::try_from(format!("{}.Busy", id.error_prefix()))
                    .unwrap(),
                Some("another operation is running".into()),
                zbus::message::Message::method_call("/", "X")
                    .unwrap()
                    .build(&())
                    .unwrap(),
            );
            assert!(matches!(
                Error::from(e),
                Error::Helper {
                    kind: HelperErrorKind::Busy,
                    ref message
                } if message == "another operation is running"
            ));
        }
    }

    /// What `HelperClient::with_connection` picks on a bus where `owners`
    /// hold names and `activatable` have activation files.
    async fn picks(owners: &[Identity], activatable: &[Identity]) -> Option<Identity> {
        let names: Vec<_> = activatable.iter().map(|i| i.bus_name()).collect();
        let bus = TestBus::start(&names)?;
        let mut keep = Vec::new();
        for id in owners {
            let c = bus.connect().await;
            c.request_name(id.bus_name()).await.unwrap();
            keep.push(c);
        }
        let conn = bus.connect().await;
        Some(
            HelperClient::with_connection(&conn)
                .await
                .unwrap()
                .identity(),
        )
    }

    #[tokio::test]
    async fn the_client_follows_the_names_on_the_bus() {
        use Identity::{Legacy, Telamon};
        let cases: [(&[Identity], &[Identity], Identity, &str); 8] = [
            (&[Telamon], &[], Telamon, "the new name is owned"),
            (&[Telamon, Legacy], &[], Telamon, "both are owned"),
            (&[], &[Telamon], Telamon, "the new name can be activated"),
            (&[], &[Telamon, Legacy], Telamon, "both can be activated"),
            (&[Legacy], &[], Legacy, "an older helper is running"),
            (&[], &[Legacy], Legacy, "only the old name can be activated"),
            (&[Legacy], &[Telamon], Telamon, "the new one can be started"),
            (
                &[],
                &[],
                Telamon,
                "nothing answers: the usual error follows",
            ),
        ];
        for (owners, activatable, want, why) in cases {
            let Some(got) = picks(owners, activatable).await else {
                return;
            };
            assert_eq!(got, want, "{why}");
        }
    }

    #[tokio::test]
    async fn with_only_an_old_helper_installed_the_client_talks_to_it() {
        use crate::helper::service::{LegacyService, Service};
        use crate::identity::Identity::Legacy;
        let Some(bus) = TestBus::start(&[Legacy.bus_name()]) else {
            return;
        };
        // what the previous package's helper answers to: the old name only
        let service = Service::new(
            std::sync::Arc::new(Static(include_str!("../tests/fixtures/status-plain.json"))),
            None,
        )
        .with_authorizer(std::sync::Arc::new(Allow));
        let _helper = bus
            .builder()
            .name(Legacy.bus_name())
            .unwrap()
            .serve_at(Legacy.object_path(), LegacyService(service))
            .unwrap()
            .build()
            .await
            .unwrap();
        let conn = bus.connect().await;
        let client = HelperClient::with_connection(&conn).await.unwrap();
        assert_eq!(client.identity(), Legacy);
        client.status().await.unwrap();
        assert_eq!(client.progress().await.unwrap(), None);
        // the new proxy, asked directly, finds nobody: the fallback was needed
        let direct = HelperClient::with_identity(&conn, Identity::Telamon)
            .await
            .unwrap();
        match direct.status().await {
            Err(Error::DBus(_)) => {}
            other => panic!("{:?}", other.map(|_| ())),
        }
    }

    struct Static(&'static str);

    impl crate::helper::BootcRunner for Static {
        fn run(&self, _args: &[&str]) -> std::result::Result<String, String> {
            Ok(self.0.to_string())
        }
    }

    struct Allow;

    impl crate::helper::service::Authorizer for Allow {
        fn check<'a>(
            &'a self,
            _conn: &'a zbus::Connection,
            _sender: Option<&'a str>,
            _action: &'a str,
        ) -> Pin<
            Box<
                dyn std::future::Future<
                        Output = std::result::Result<(), crate::helper::HelperError>,
                    > + Send
                    + 'a,
            >,
        > {
            Box::pin(async { Ok(()) })
        }
    }
}
