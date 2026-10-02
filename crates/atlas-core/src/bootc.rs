//! Types for `bootc status --json` (bootc 1.16, API `org.containers.bootc/v1`)
//! and the channel tag-rewrite helper.
//!
//! All fields are lenient: unknown fields are ignored and optional ones
//! default, so a newer bootc does not break the parser.

use std::fmt;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// The whole `bootc status --json` document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    #[serde(default)]
    pub api_version: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub spec: Spec,
    #[serde(default)]
    pub status: HostStatus,
    /// Commits the ostree image refs point to (see [`image_ref_heads`]). Not
    /// part of bootc's JSON: the caller fills it in, and without it
    /// [`Status::latest_image`] falls back to the booted entry.
    #[serde(skip)]
    pub image_ref_heads: Vec<String>,
}

/// Where ostree-ext keeps one ref per container image reference
/// (`ostree/container/image/<escaped reference>`), each pointing to the
/// commit of the image last pulled for it. Readable without root.
pub const IMAGE_REFS_DIR: &str = "/ostree/repo/refs/heads/ostree/container/image";

/// The commit checksums of the image refs under `dir`. Empty when they can't
/// be read (not an ostree system, a test).
pub fn image_ref_heads(dir: &Path) -> Vec<String> {
    let mut heads = Vec::new();
    let mut dirs = vec![dir.to_path_buf()];
    while let Some(d) = dirs.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
            match e.file_type() {
                Ok(t) if t.is_dir() => dirs.push(e.path()),
                Ok(t) if t.is_file() => {
                    if let Ok(text) = std::fs::read_to_string(e.path()) {
                        let c = text.trim();
                        if !c.is_empty() {
                            heads.push(c.to_string());
                        }
                    }
                }
                _ => {}
            }
        }
    }
    heads
}

/// The desired state (`spec`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Spec {
    /// The image the host tracks.
    #[serde(default)]
    pub image: Option<ImageReference>,
    #[serde(default)]
    pub boot_order: Option<String>,
}

/// The observed state (`status`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct HostStatus {
    #[serde(default)]
    pub staged: Option<BootEntry>,
    #[serde(default)]
    pub booted: Option<BootEntry>,
    #[serde(default)]
    pub rollback: Option<BootEntry>,
    #[serde(default)]
    pub rollback_queued: bool,
    /// `bootcHost` on a bootc system, null on a plain container.
    #[serde(default, rename = "type")]
    pub host_type: Option<String>,
}

/// A booted, staged or rollback deployment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct BootEntry {
    /// Null for a deployment that is not a container image.
    #[serde(default)]
    pub image: Option<ImageStatus>,
    /// An update bootc found with `upgrade --check` but has not downloaded.
    #[serde(default)]
    pub cached_update: Option<ImageStatus>,
    #[serde(default)]
    pub incompatible: bool,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub store: Option<String>,
    #[serde(default)]
    pub ostree: Option<OstreeEntry>,
}

impl BootEntry {
    /// The image version label (`org.opencontainers.image.version`).
    pub fn version(&self) -> Option<&str> {
        self.image.as_ref()?.version.as_deref()
    }

    /// The image build time, RFC 3339.
    pub fn timestamp(&self) -> Option<&str> {
        self.image.as_ref()?.timestamp.as_deref()
    }

    pub fn digest(&self) -> Option<&str> {
        Some(self.image.as_ref()?.image_digest.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct OstreeEntry {
    #[serde(default)]
    pub checksum: String,
    #[serde(default)]
    pub deploy_serial: u32,
    #[serde(default)]
    pub stateroot: String,
}

/// Image reference, version, build time and digest of one deployment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ImageStatus {
    pub image: ImageReference,
    #[serde(default)]
    pub architecture: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    /// Image build time, RFC 3339.
    #[serde(default)]
    pub timestamp: Option<String>,
    #[serde(default)]
    pub image_digest: String,
}

/// A container image reference: a name (with tag) plus a transport.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ImageReference {
    /// Name with tag, e.g. `ghcr.io/eternalcoder454/atlasos:stable`, or for
    /// `oci` a path such as `/var/lib/test/img:stable`.
    pub image: String,
    /// `registry`, `oci`, `containers-storage`, ... Missing means `registry`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<serde_json::Value>,
}

/// An update channel. Only these two exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    Stable,
    Testing,
}

impl Channel {
    pub fn as_str(self) -> &'static str {
        match self {
            Channel::Stable => "stable",
            Channel::Testing => "testing",
        }
    }
}

impl fmt::Display for Channel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for Channel {
    type Err = RefError;

    /// Accepts exactly `stable` or `testing`.
    fn from_str(s: &str) -> Result<Self, RefError> {
        match s {
            "stable" => Ok(Channel::Stable),
            "testing" => Ok(Channel::Testing),
            other => Err(RefError::BadChannel(other.to_string())),
        }
    }
}

/// Why a reference or channel was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefError {
    BadChannel(String),
    BadImage(String),
    BadTransport(String),
}

impl fmt::Display for RefError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RefError::BadChannel(c) => {
                write!(f, "channel must be \"stable\" or \"testing\", got {c:?}")
            }
            RefError::BadImage(i) => write!(f, "unusable image reference {i:?}"),
            RefError::BadTransport(t) => write!(f, "unsupported image transport {t:?}"),
        }
    }
}

impl std::error::Error for RefError {}

/// Transports a switch may use. The transport of the booted ref is kept as is.
const TRANSPORTS: &[&str] = &[
    "registry",
    "oci",
    "oci-archive",
    "containers-storage",
    "docker-daemon",
];

impl ImageReference {
    /// The same image and transport (the signature policy is not compared).
    pub fn same_image(&self, other: &ImageReference) -> bool {
        self.image == other.image && self.transport_or_default() == other.transport_or_default()
    }

    /// The transport, `registry` when not given.
    pub fn transport_or_default(&self) -> &str {
        self.transport.as_deref().unwrap_or("registry")
    }

    /// The tag of `image`, if any.
    pub fn tag(&self) -> Option<&str> {
        split_tag(&self.image).1
    }

    /// The channel in the tag, if it is `stable` or `testing`.
    pub fn channel(&self) -> Option<Channel> {
        self.tag()?.parse().ok()
    }

    /// The same reference with only the tag replaced by `channel`. The
    /// transport and name stay. A digest (`@sha256:...`) is dropped, since it
    /// would pin the old image.
    pub fn with_channel(&self, channel: &str) -> Result<ImageReference, RefError> {
        let channel: Channel = channel.parse()?;
        let transport = self.transport_or_default();
        if !TRANSPORTS.contains(&transport) {
            return Err(RefError::BadTransport(transport.to_string()));
        }
        let name = split_tag(&self.image).0;
        let name = name.split('@').next().unwrap_or(name);
        if name.is_empty()
            || name.starts_with('-')
            || name.chars().any(|c| c.is_control() || c.is_whitespace())
        {
            return Err(RefError::BadImage(self.image.clone()));
        }
        Ok(ImageReference {
            image: format!("{name}:{channel}"),
            transport: Some(transport.to_string()),
            signature: self.signature.clone(),
        })
    }
}

/// Split `name:tag` where the tag is the part after a colon that follows the
/// last `/` (so `host:5000/img` has no tag). A `@digest` suffix is not a tag.
fn split_tag(image: &str) -> (&str, Option<&str>) {
    let no_digest = image.split('@').next().unwrap_or(image);
    let after_slash = no_digest.rfind('/').map_or(0, |i| i + 1);
    match no_digest[after_slash..].rfind(':') {
        Some(i) => {
            let at = after_slash + i;
            (&no_digest[..at], Some(&no_digest[at + 1..]))
        }
        None => (no_digest, None),
    }
}

impl Status {
    pub fn from_json(json: &str) -> Result<Status, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// The booted image's reference; falls back to `spec.image`.
    pub fn booted_ref(&self) -> Option<&ImageReference> {
        self.status
            .booted
            .as_ref()
            .and_then(|b| b.image.as_ref())
            .map(|i| &i.image)
            .or(self.spec.image.as_ref())
    }

    /// The channel the system follows (`stable`, `testing`), if it has one.
    /// That is `spec.image`, which a switch changes at once; the booted ref
    /// keeps the old channel until the restart.
    pub fn channel(&self) -> Option<Channel> {
        self.spec.image.as_ref().or(self.booted_ref())?.channel()
    }

    /// True when a staged deployment exists.
    pub fn has_staged(&self) -> bool {
        self.status.staged.is_some()
    }

    /// The newest image of the tracked reference this system knows of.
    ///
    /// `upgrade --check` records what it found as a `cachedUpdate` on the
    /// commit the image's ostree ref points to: the image pulled last. That is
    /// the staged entry, else usually the booted one, but after a rollback the
    /// rollback entry, while the booted entry keeps a stale `cachedUpdate`
    /// from an older check (bootc 1.16). So: the `cachedUpdate` of the entry
    /// holding the ref's commit, or that entry's own image when it has none
    /// (the registry has nothing newer). Without ref heads, or when no entry
    /// holds the commit, the booted entry's `cachedUpdate`.
    pub fn latest_image(&self) -> Option<&ImageStatus> {
        let s = &self.status;
        let head = [s.staged.as_ref(), s.booted.as_ref(), s.rollback.as_ref()]
            .into_iter()
            .flatten()
            .find(|e| {
                let Some(img) = e.image.as_ref() else {
                    return false;
                };
                let at_head = e.ostree.as_ref().is_some_and(|o| {
                    !o.checksum.is_empty() && self.image_ref_heads.contains(&o.checksum)
                });
                at_head
                    && self
                        .spec
                        .image
                        .as_ref()
                        .is_none_or(|spec| img.image.same_image(spec))
            });
        match head {
            Some(e) => e.cached_update.as_ref().or(e.image.as_ref()),
            None => s.booted.as_ref().and_then(|b| b.cached_update.as_ref()),
        }
    }

    /// [`Status::latest_image`] when it is neither the booted nor the staged
    /// image: an update `upgrade --check` found that is not staged yet. It can
    /// be the rollback image (the version the user went back from).
    pub fn available_update(&self) -> Option<&ImageStatus> {
        let latest = self.latest_image()?;
        let d = latest.image_digest.as_str();
        let is = |e: Option<&BootEntry>| e.and_then(BootEntry::digest) == Some(d);
        if d.is_empty() || is(self.status.booted.as_ref()) || is(self.status.staged.as_ref()) {
            return None;
        }
        Some(latest)
    }

    /// True when [`Status::available_update`] has one.
    pub fn update_available(&self) -> bool {
        self.available_update().is_some()
    }
}

#[cfg(test)]
pub(crate) mod fixtures {
    pub const BOOTED_WITH_UPDATE: &str = include_str!("../tests/fixtures/status-update.json");
    pub const PLAIN: &str = include_str!("../tests/fixtures/status-plain.json");
    pub const NOT_BOOTC: &str = include_str!("../tests/fixtures/status-container.json");
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;

    fn r(image: &str, transport: Option<&str>) -> ImageReference {
        ImageReference {
            image: image.into(),
            transport: transport.map(Into::into),
            signature: None,
        }
    }

    const SPEC: &str = r#"{"image":"ghcr.io/e/atlasos:stable","transport":"registry"}"#;

    fn img(version: &str, digest: &str) -> String {
        format!(
            r#"{{"image":{SPEC},"version":"{version}","timestamp":"2026-10-01T00:00:00Z","imageDigest":"{digest}"}}"#
        )
    }

    /// An entry with an image, a commit and an optional cachedUpdate.
    fn entry(version: &str, digest: &str, csum: &str, cached: Option<(&str, &str)>) -> String {
        let cached = cached.map_or("null".to_string(), |(v, d)| img(v, d));
        format!(
            r#"{{"image":{},"cachedUpdate":{cached},"ostree":{{"checksum":"{csum}","deploySerial":0,"stateroot":"default"}}}}"#,
            img(version, digest)
        )
    }

    fn host(staged: &str, booted: &str, rollback: &str, heads: &[&str]) -> Status {
        let json = format!(
            r#"{{"apiVersion":"org.containers.bootc/v1","kind":"BootcHost","spec":{{"image":{SPEC}}},
               "status":{{"staged":{staged},"booted":{booted},"rollback":{rollback},"rollbackQueued":false,"type":"bootcHost"}}}}"#
        );
        let mut s = Status::from_json(&json).unwrap();
        s.image_ref_heads = heads.iter().map(|h| h.to_string()).collect();
        s
    }

    fn latest(s: &Status) -> Option<&str> {
        s.latest_image().map(|i| i.image_digest.as_str())
    }

    fn available(s: &Status) -> Option<&str> {
        s.available_update().map(|i| i.image_digest.as_str())
    }

    #[test]
    fn after_going_back_the_check_result_is_on_the_rollback_entry() {
        // Booted I keeps a stale cachedUpdate (the image it went back from);
        // the check put the new image on the rollback entry, the ref's head.
        let s = host(
            "null",
            &entry("44.24", "sha256:i", "c-i", Some(("44.99", "sha256:old"))),
            &entry(
                "44.99",
                "sha256:bad",
                "c-bad",
                Some(("44.25", "sha256:new")),
            ),
            &["c-bad"],
        );
        assert_eq!(latest(&s), Some("sha256:new"));
        assert_eq!(available(&s), Some("sha256:new"));
    }

    #[test]
    fn after_a_rollback_with_nothing_newer_the_latest_is_the_rollback_image() {
        let s = host(
            "null",
            &entry("44.24", "sha256:i", "c-i", Some(("44.99", "sha256:old"))),
            &entry("44.99", "sha256:bad", "c-bad", None),
            &["c-bad"],
        );
        assert_eq!(latest(&s), Some("sha256:bad"));
        // Offered as the version the user went back from, not the stale one.
        assert_eq!(available(&s), Some("sha256:bad"));
    }

    #[test]
    fn booted_at_the_head_uses_its_own_check_result() {
        let up = host(
            "null",
            &entry("44.24", "sha256:i", "c-i", Some(("44.25", "sha256:new"))),
            &entry("44.23", "sha256:h", "c-h", Some(("44.30", "sha256:stale"))),
            &["c-i"],
        );
        assert_eq!(available(&up), Some("sha256:new"));
        let none = host(
            "null",
            &entry("44.24", "sha256:i", "c-i", None),
            "null",
            &["c-i"],
        );
        assert_eq!(latest(&none), Some("sha256:i"));
        assert_eq!(available(&none), None);
        assert!(!none.update_available());
    }

    #[test]
    fn a_staged_update_is_not_offered_again() {
        let s = host(
            &entry("44.25", "sha256:new", "c-new", None),
            &entry("44.24", "sha256:i", "c-i", Some(("44.25", "sha256:new"))),
            "null",
            &["c-new"],
        );
        assert_eq!(available(&s), None);
        let newer = host(
            &entry(
                "44.25",
                "sha256:new",
                "c-new",
                Some(("44.26", "sha256:newer")),
            ),
            &entry("44.24", "sha256:i", "c-i", Some(("44.25", "sha256:new"))),
            "null",
            &["c-new"],
        );
        assert_eq!(available(&newer), Some("sha256:newer"));
    }

    #[test]
    fn without_a_matching_head_the_booted_cached_update_is_used() {
        let booted = entry("44.24", "sha256:i", "c-i", Some(("44.25", "sha256:new")));
        let rollback = entry("44.23", "sha256:h", "c-h", Some(("44.30", "sha256:x")));
        for heads in [&[][..], &["c-elsewhere"][..]] {
            let s = host("null", &booted, &rollback, heads);
            assert_eq!(available(&s), Some("sha256:new"));
        }
    }

    #[test]
    fn a_head_of_another_image_reference_is_ignored() {
        let other = entry("44.30", "sha256:t", "c-t", Some(("44.31", "sha256:t2")))
            .replace("atlasos:stable", "atlasos:testing");
        let s = host(
            "null",
            &entry("44.24", "sha256:i", "c-i", Some(("44.25", "sha256:new"))),
            &other,
            &["c-t"],
        );
        assert_eq!(available(&s), Some("sha256:new"));
    }

    #[test]
    fn reads_image_ref_heads_from_nested_ref_files() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("oci_3A__2F_var/mnt/reg");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("x_3A_stable"), "abc123\n").unwrap();
        std::fs::write(dir.path().join("ghcr.io_2F_e_3A_testing"), "def456\n").unwrap();
        let mut heads = image_ref_heads(dir.path());
        heads.sort();
        assert_eq!(heads, ["abc123", "def456"]);
        assert!(image_ref_heads(&dir.path().join("missing")).is_empty());
    }

    #[test]
    fn parses_status_with_staged_rollback_and_cached_update() {
        let s = Status::from_json(BOOTED_WITH_UPDATE).unwrap();
        let booted = s.status.booted.as_ref().unwrap();
        assert_eq!(booted.version(), Some("44.20261001"));
        assert!(booted.digest().unwrap().starts_with("sha256:"));
        assert_eq!(booted.timestamp(), Some("2026-10-01T04:12:09Z"));
        assert_eq!(
            booted.cached_update.as_ref().unwrap().version.as_deref(),
            Some("44.20261008")
        );
        assert_eq!(
            s.status.staged.as_ref().unwrap().version(),
            Some("44.20261008")
        );
        assert_eq!(
            s.status.rollback.as_ref().unwrap().version(),
            Some("44.20260924")
        );
        assert_eq!(s.channel(), Some(Channel::Stable));
        assert!(s.has_staged());
        assert!(!s.update_available(), "cached update is already staged");
        assert_eq!(s.status.host_type.as_deref(), Some("bootcHost"));
    }

    #[test]
    fn parses_plain_status_without_staged() {
        let s = Status::from_json(PLAIN).unwrap();
        assert!(s.status.staged.is_none());
        assert!(s.status.rollback.is_none());
        assert!(s.update_available(), "cached update, nothing staged");
        assert_eq!(s.channel(), Some(Channel::Testing));
        assert_eq!(s.booted_ref().unwrap().transport_or_default(), "registry");
    }

    #[test]
    fn channel_follows_a_switch_before_the_restart() {
        let json = r#"{
            "spec": {"image": {"image": "ghcr.io/eternalcoder454/atlasos:testing", "transport": "registry"}},
            "status": {
                "booted": {"image": {"image": {"image": "ghcr.io/eternalcoder454/atlasos:stable", "transport": "registry"}}},
                "staged": {"image": {"image": {"image": "ghcr.io/eternalcoder454/atlasos:testing", "transport": "registry"}}}
            }
        }"#;
        let s = Status::from_json(json).unwrap();
        assert_eq!(s.channel(), Some(Channel::Testing));
        assert_eq!(s.booted_ref().unwrap().channel(), Some(Channel::Stable));
    }

    #[test]
    fn parses_status_outside_a_bootc_host() {
        let s = Status::from_json(NOT_BOOTC).unwrap();
        assert!(s.status.booted.is_none());
        assert!(s.booted_ref().is_none());
        assert!(!s.update_available());
    }

    #[test]
    fn channel_accepts_only_stable_and_testing() {
        assert_eq!("stable".parse::<Channel>(), Ok(Channel::Stable));
        assert_eq!("testing".parse::<Channel>(), Ok(Channel::Testing));
        for bad in ["", "Stable", "latest", "stable ", "stable:x", "../x", "-x"] {
            assert!(bad.parse::<Channel>().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn rewrites_registry_tag() {
        let n = r("ghcr.io/eternalcoder454/atlasos:stable", Some("registry"))
            .with_channel("testing")
            .unwrap();
        assert_eq!(n.image, "ghcr.io/eternalcoder454/atlasos:testing");
        assert_eq!(n.transport.as_deref(), Some("registry"));
    }

    #[test]
    fn rewrites_untagged_and_versioned_tags() {
        let n = r("ghcr.io/e/atlasos", None).with_channel("stable").unwrap();
        assert_eq!(n.image, "ghcr.io/e/atlasos:stable");
        assert_eq!(n.transport.as_deref(), Some("registry"));
        let n = r("ghcr.io/e/atlasos:44.20261001", None)
            .with_channel("testing")
            .unwrap();
        assert_eq!(n.image, "ghcr.io/e/atlasos:testing");
    }

    #[test]
    fn registry_port_is_not_a_tag() {
        let n = r("localhost:5000/atlasos", Some("registry"))
            .with_channel("testing")
            .unwrap();
        assert_eq!(n.image, "localhost:5000/atlasos:testing");
        let n = r("localhost:5000/atlasos:stable", None)
            .with_channel("testing")
            .unwrap();
        assert_eq!(n.image, "localhost:5000/atlasos:testing");
    }

    #[test]
    fn digest_is_dropped() {
        let n = r("ghcr.io/e/atlasos:stable@sha256:abcd", None)
            .with_channel("testing")
            .unwrap();
        assert_eq!(n.image, "ghcr.io/e/atlasos:testing");
        let n = r("ghcr.io/e/atlasos@sha256:abcd", None)
            .with_channel("stable")
            .unwrap();
        assert_eq!(n.image, "ghcr.io/e/atlasos:stable");
    }

    #[test]
    fn oci_forms_keep_transport() {
        let n = r("/var/lib/vm/atlasos:stable", Some("oci"))
            .with_channel("testing")
            .unwrap();
        assert_eq!(n.image, "/var/lib/vm/atlasos:testing");
        assert_eq!(n.transport.as_deref(), Some("oci"));
        let n = r("/var/lib/vm/atlasos", Some("oci"))
            .with_channel("testing")
            .unwrap();
        assert_eq!(n.image, "/var/lib/vm/atlasos:testing");
        let n = r("/x/a.tar:stable", Some("oci-archive"))
            .with_channel("testing")
            .unwrap();
        assert_eq!(n.transport.as_deref(), Some("oci-archive"));
    }

    #[test]
    fn rejects_bad_channels_images_and_transports() {
        let base = r("ghcr.io/e/atlasos:stable", None);
        assert!(matches!(
            base.with_channel("latest"),
            Err(RefError::BadChannel(_))
        ));
        assert!(matches!(
            base.with_channel(""),
            Err(RefError::BadChannel(_))
        ));
        assert!(matches!(
            r("-evil:stable", None).with_channel("stable"),
            Err(RefError::BadImage(_))
        ));
        assert!(matches!(
            r("a b:stable", None).with_channel("stable"),
            Err(RefError::BadImage(_))
        ));
        assert!(matches!(
            r("x:stable", Some("--apply")).with_channel("stable"),
            Err(RefError::BadTransport(_))
        ));
    }
}
