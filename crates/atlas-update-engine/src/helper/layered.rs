//! Systems with local rpm-ostree changes: packages added with
//! `rpm-ostree install`, base packages replaced or removed, a regenerated
//! initramfs.
//!
//! bootc refuses to upgrade such a system ("Deployment contains local
//! rpm-ostree modifications; cannot upgrade via bootc") and shows its
//! deployments as `incompatible`, without an image, version or channel, and
//! with a null `spec.image`. So on one the helper
//! - fills those entries' `image` in from `rpm-ostree status --json` (the
//!   deployment with the same ostree commit) and `spec.image` from the
//!   deployment rpm-ostree upgrades from, its first (the staged one, else the
//!   one a queued rollback goes to, else the booted one: rpm-ostree takes the
//!   origin of the deployment ahead of the booted one, if any);
//! - checks for an update with `skopeo inspect`, since rpm-ostree's own
//!   `upgrade --check` looks only at the added packages, never at the image,
//!   and keeps what it found, shown as the booted entry's `cachedUpdate`;
//! - downloads with `rpm-ostree upgrade` and switches channel with
//!   `rpm-ostree rebase`, which keep the local changes.
//!
//! Everything else then reads the status as it would bootc's.

use serde_json::{Value, json};

use atlas_framework_system::bootc::{ImageReference, ImageStatus, TRANSPORTS, utc_second};
use atlas_framework_system::history::rfc3339_from_unix;

/// Where the helper keeps the last update check's result on such a system
/// (bootc keeps its own in the ostree repo). /var/lib/atlas-core is the
/// state directory's name from before the package was atlas-system-helper.
pub const UPDATE_FILE: &str = "/var/lib/atlas-core/layered-update.json";

/// True when bootc shows local rpm-ostree changes on the booted or staged
/// deployment: then `bootc upgrade` and `bootc switch` refuse to run.
pub fn is_layered(bootc: &Value) -> bool {
    let status = &bootc["status"];
    ["booted", "staged"]
        .iter()
        .any(|k| status[k]["incompatible"] == Value::Bool(true))
}

/// An rpm-ostree origin (`container-image-reference`, such as
/// `ostree-image-signed:docker://ghcr.io/eternalcoder454/atlasos:stable`) as
/// bootc shows an image reference. `None` for one that is not a container
/// image.
pub fn parse_origin(origin: &str) -> Option<ImageReference> {
    let (kind, r) = origin.split_once(':')?;
    let (signature, rest) = match kind {
        "ostree-unverified-registry" => (None, format!("docker://{r}")),
        "ostree-unverified-image" => (None, r.to_string()),
        "ostree-image-signed" => (Some(json!("containerPolicy")), r.to_string()),
        "ostree-remote-registry" | "ostree-remote-image" => {
            let (remote, image) = r.split_once(':')?;
            let image = if kind == "ostree-remote-registry" {
                format!("docker://{image}")
            } else {
                image.to_string()
            };
            (Some(json!({ "ostreeRemote": remote })), image)
        }
        _ => return None,
    };
    let (transport, image) = match rest.strip_prefix("docker://") {
        Some(image) => ("registry", image),
        None => rest.split_once(':')?,
    };
    // skopeo and rpm-ostree get these as arguments
    if !TRANSPORTS.contains(&transport) || image.is_empty() || image.starts_with('-') {
        return None;
    }
    Some(ImageReference {
        image: image.to_string(),
        transport: Some(transport.to_string()),
        signature,
    })
}

/// `origin` with only its tag replaced by `channel`: what `rpm-ostree
/// rebase` switches to. The checks are [`ImageReference::with_channel`]'s.
pub fn origin_with_channel(origin: &str, channel: &str) -> Result<String, String> {
    origin_retargeted(origin, |r| {
        r.with_channel(channel).map_err(|e| e.to_string())
    })
}

/// `origin` with its image replaced by what `retarget` makes of the image it
/// names; the part before the image (the kind and transport) stays.
pub fn origin_retargeted(
    origin: &str,
    retarget: impl FnOnce(&ImageReference) -> Result<ImageReference, String>,
) -> Result<String, String> {
    let unusable = || format!("unusable rpm-ostree origin {origin:?}");
    let r = parse_origin(origin).ok_or_else(unusable)?;
    let prefix = origin.strip_suffix(r.image.as_str()).ok_or_else(unusable)?;
    let new = retarget(&r)?;
    Ok(format!("{prefix}{}", new.image))
}

/// The origin rpm-ostree upgrades from: its first deployment's.
pub fn followed_origin(rpm_ostree: &str) -> Result<String, String> {
    let v: Value = serde_json::from_str(rpm_ostree)
        .map_err(|e| format!("cannot parse rpm-ostree status: {e}"))?;
    v["deployments"][0]["container-image-reference"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| "rpm-ostree follows no container image".into())
}

/// How skopeo names the image `r`.
pub fn skopeo_ref(r: &ImageReference) -> String {
    match r.transport_or_default() {
        "registry" => format!("docker://{}", r.image),
        t => format!("{t}:{}", r.image),
    }
}

/// `skopeo inspect` output for the image `r` as bootc shows an image. Its
/// `Digest` is the manifest's, which is what rpm-ostree records for a
/// deployment as long as the image is not a multi-arch index (AtlasOS's
/// aren't).
pub fn image_from_skopeo(json: &str, r: &ImageReference) -> Result<ImageStatus, String> {
    let v: Value =
        serde_json::from_str(json).map_err(|e| format!("cannot parse skopeo inspect: {e}"))?;
    let digest = v["Digest"]
        .as_str()
        .filter(|d| !d.is_empty())
        .ok_or("skopeo inspect gave no digest")?;
    Ok(ImageStatus {
        image: r.clone(),
        architecture: v["Architecture"].as_str().map(str::to_string),
        version: v["Labels"]["org.opencontainers.image.version"]
            .as_str()
            .map(str::to_string),
        timestamp: v["Created"].as_str().map(str::to_string),
        image_digest: digest.to_string(),
    })
}

/// One rpm-ostree deployment's image. A deployment with local changes keeps
/// its image's build time as `base-timestamp`; its `timestamp` is when the
/// changes were made.
fn image_of(d: &Value) -> Option<ImageStatus> {
    let image = parse_origin(d["container-image-reference"].as_str()?)?;
    let digest = d["container-image-reference-digest"]
        .as_str()
        .filter(|d| !d.is_empty())?;
    let secs = d["base-timestamp"].as_u64().or(d["timestamp"].as_u64());
    Some(ImageStatus {
        image,
        architecture: None,
        version: d["version"].as_str().map(str::to_string),
        timestamp: secs.map(rfc3339_from_unix),
        image_digest: digest.to_string(),
    })
}

/// The rpm-ostree deployment behind a bootc status entry: the same commit,
/// and the same deploy serial and stateroot where bootc shows them (one
/// commit can be deployed twice, under different origins).
fn same_deployment(d: &Value, ostree: &Value) -> bool {
    let commit = ostree["checksum"].as_str().filter(|c| !c.is_empty());
    commit.is_some()
        && d["checksum"].as_str() == commit
        && ostree["deploySerial"]
            .as_u64()
            .is_none_or(|s| d["serial"].as_u64().is_none_or(|t| t == s))
        && ostree["stateroot"]
            .as_str()
            .is_none_or(|s| d["osname"].as_str().is_none_or(|t| t == s))
}

/// True when `update` was built after every one of `images`: it is still
/// news. An image whose time can't be compared doesn't count.
fn newer_than_all(update: &ImageStatus, images: &[ImageStatus]) -> bool {
    let u = update.timestamp.as_deref().and_then(utc_second);
    images.iter().all(|i| {
        let t = i.timestamp.as_deref().and_then(utc_second);
        i.image_digest != update.image_digest && u.zip(t).is_none_or(|(u, t)| u > t)
    })
}

/// bootc's status (`bootc`, parsed) with what it leaves out on a system with
/// local rpm-ostree changes filled in from `rpm_ostree` (see the module
/// docs). `update` is the last check's result; it becomes the booted entry's
/// `cachedUpdate` while it is for the image rpm-ostree follows.
pub fn fill(
    mut bootc: Value,
    rpm_ostree: &str,
    update: Option<&ImageStatus>,
) -> Result<String, String> {
    let r: Value = serde_json::from_str(rpm_ostree)
        .map_err(|e| format!("cannot parse rpm-ostree status: {e}"))?;
    let empty = Vec::new();
    let deployments = r["deployments"].as_array().unwrap_or(&empty);
    let to_value = |i: &ImageStatus| serde_json::to_value(i).map_err(|e| e.to_string());
    for key in ["staged", "booted", "rollback"] {
        let Some(entry) = bootc
            .get_mut("status")
            .and_then(|s| s.get_mut(key))
            .and_then(Value::as_object_mut)
        else {
            continue;
        };
        if !entry.get("image").is_none_or(Value::is_null) {
            continue;
        }
        let Some(ostree) = entry.get("ostree") else {
            continue;
        };
        if let Some(image) = deployments
            .iter()
            .find(|d| same_deployment(d, ostree))
            .and_then(image_of)
        {
            entry.insert("image".into(), to_value(&image)?);
        }
    }
    let follows = deployments.first().and_then(image_of).map(|i| i.image);
    let Some(host) = bootc.as_object_mut() else {
        return Err("bootc status is not an object".into());
    };
    if let Some(follows) = &follows {
        let spec = host.entry("spec").or_insert_with(|| json!({}));
        if let Some(spec) = spec.as_object_mut()
            && spec.get("image").is_none_or(Value::is_null)
        {
            spec.insert(
                "image".into(),
                serde_json::to_value(follows).map_err(|e| e.to_string())?,
            );
        }
    }
    // The check's answer goes stale once an image as new is booted or staged
    // (the stage timer stages without asking the helper).
    let current: Vec<ImageStatus> = deployments
        .iter()
        .filter(|d| d["booted"] == Value::Bool(true) || d["staged"] == Value::Bool(true))
        .filter_map(image_of)
        .collect();
    if let (Some(update), Some(follows)) = (update, &follows)
        && update.image.same_image(follows)
        && newer_than_all(update, &current)
        && let Some(booted) = host
            .get_mut("status")
            .and_then(|s| s.get_mut("booted"))
            .and_then(Value::as_object_mut)
    {
        booted.insert("cachedUpdate".into(), to_value(update)?);
    }
    serde_json::to_string(&bootc).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_framework_system::bootc::{Channel, Status};

    const BOOTC: &str = include_str!("../../tests/fixtures/status-layered.json");
    const RPM_OSTREE: &str = include_str!("../../tests/fixtures/rpm-ostree-layered.json");
    const STABLE: &str = "ostree-unverified-image:oci:/var/mnt/atlasreg/registry:stable";

    fn filled(update: Option<&ImageStatus>) -> Status {
        let v = serde_json::from_str(BOOTC).unwrap();
        Status::from_json(&fill(v, RPM_OSTREE, update).unwrap()).unwrap()
    }

    #[test]
    fn layered_when_booted_or_staged_is_incompatible() {
        assert!(is_layered(&serde_json::from_str(BOOTC).unwrap()));
        let plain = include_str!("../../tests/fixtures/status-plain.json");
        assert!(!is_layered(&serde_json::from_str(plain).unwrap()));
        let staged_only = json!({"status": {"booted": {"incompatible": false},
                                            "staged": {"incompatible": true}}});
        assert!(is_layered(&staged_only));
        assert!(!is_layered(&json!({"status": {"booted": null}})));
    }

    #[test]
    fn origins_as_bootc_shows_them() {
        let r = parse_origin("ostree-image-signed:docker://ghcr.io/eternalcoder454/atlasos:stable")
            .unwrap();
        assert_eq!(r.image, "ghcr.io/eternalcoder454/atlasos:stable");
        assert_eq!(r.transport_or_default(), "registry");
        assert_eq!(r.signature, Some(json!("containerPolicy")));
        let r = parse_origin("ostree-unverified-registry:ghcr.io/x/atlasos:testing").unwrap();
        assert_eq!(
            (
                r.image.as_str(),
                r.transport_or_default(),
                r.signature.clone()
            ),
            ("ghcr.io/x/atlasos:testing", "registry", None)
        );
        let r = parse_origin(STABLE).unwrap();
        assert_eq!(
            (r.image.as_str(), r.transport_or_default()),
            ("/var/mnt/atlasreg/registry:stable", "oci")
        );
        let r = parse_origin("ostree-remote-image:fedora:docker://quay.io/f/k:44").unwrap();
        assert_eq!(r.image, "quay.io/f/k:44");
        assert_eq!(r.signature, Some(json!({"ostreeRemote": "fedora"})));
        for bad in [
            "",
            "fedora:fedora/44/x86_64/kinoite",
            "ostree-unverified-image:",
            "ostree-unverified-image:oci",
            "ostree-unverified-image:oci:--authfile=/x:y",
            "ostree-unverified-registry:-x/y:stable",
            "ostree-unverified-image:dir:/var/tmp/x",
        ] {
            assert_eq!(parse_origin(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn rebase_target_changes_only_the_tag() {
        assert_eq!(
            origin_with_channel(STABLE, "testing").unwrap(),
            "ostree-unverified-image:oci:/var/mnt/atlasreg/registry:testing"
        );
        assert_eq!(
            origin_with_channel(
                "ostree-image-signed:docker://ghcr.io/eternalcoder454/atlasos:testing",
                "stable"
            )
            .unwrap(),
            "ostree-image-signed:docker://ghcr.io/eternalcoder454/atlasos:stable"
        );
        assert_eq!(
            origin_with_channel(
                "ostree-unverified-registry:ghcr.io/x/atlasos:stable",
                "testing"
            )
            .unwrap(),
            "ostree-unverified-registry:ghcr.io/x/atlasos:testing"
        );
        assert!(origin_with_channel(STABLE, "latest").is_err());
        assert!(origin_with_channel("fedora:fedora/44/x86_64/kinoite", "stable").is_err());
    }

    #[test]
    fn skopeo_names() {
        let r = parse_origin("ostree-image-signed:docker://ghcr.io/x/atlasos:stable").unwrap();
        assert_eq!(skopeo_ref(&r), "docker://ghcr.io/x/atlasos:stable");
        assert_eq!(
            skopeo_ref(&parse_origin(STABLE).unwrap()),
            "oci:/var/mnt/atlasreg/registry:stable"
        );
    }

    #[test]
    fn fills_images_spec_and_channel_from_rpm_ostree() {
        let s = filled(None);
        // the booted and staged deployments carry htop; bootc showed neither
        let booted = s.status.booted.as_ref().unwrap();
        assert_eq!(booted.version(), Some("44.20261034"));
        assert_eq!(
            booted.digest(),
            Some("sha256:e9f26ecee723163461a91a07197ae7adaf6bf08997081939c589cc7cf99e381f")
        );
        // the image's build time, not when htop was added
        assert_eq!(booted.timestamp(), Some("2026-10-02T18:54:39Z"));
        let staged = s.status.staged.as_ref().unwrap();
        assert_eq!(staged.version(), Some("44.20261002"));
        assert_eq!(
            staged.digest(),
            Some("sha256:87cf0ded78678b44ca5ce652a56fdc6ee412c00100cf3d085e67dbc2e1dc3037")
        );
        // bootc's own rollback entry is left as it was
        let rollback = s.status.rollback.as_ref().unwrap();
        assert_eq!(rollback.timestamp(), Some("2026-10-02T18:54:39.043034591Z"));
        assert_eq!(s.channel(), Some(Channel::Stable));
        assert_eq!(
            s.booted_ref().unwrap().image,
            "/var/mnt/atlasreg/registry:stable"
        );
        assert!(!s.update_available());
    }

    #[test]
    fn a_checked_update_shows_while_it_is_for_the_followed_image() {
        let r = parse_origin(STABLE).unwrap();
        let newer = ImageStatus {
            image: r.clone(),
            version: Some("44.20261009".into()),
            // built after the booted image (whose test version is higher)
            timestamp: Some("2026-10-09T04:00:00Z".into()),
            image_digest: "sha256:ccc".into(),
            ..Default::default()
        };
        let s = filled(Some(&newer));
        assert_eq!(
            s.available_update().map(|u| u.image_digest.as_str()),
            Some("sha256:ccc")
        );
        // the one already staged is not offered again
        let staged = ImageStatus {
            image_digest: "sha256:87cf0ded78678b44ca5ce652a56fdc6ee412c00100cf3d085e67dbc2e1dc3037"
                .into(),
            ..newer.clone()
        };
        assert!(!filled(Some(&staged)).update_available());
        // a check from before a channel switch is not
        let other = ImageStatus {
            image: r.with_channel("testing").unwrap(),
            ..newer
        };
        assert!(!filled(Some(&other)).update_available());
    }

    #[test]
    fn a_checked_update_goes_stale_once_one_as_new_is_staged() {
        // staged: built 2026-10-02T22:42:16Z (its base-timestamp)
        let update = |t: &str| ImageStatus {
            image: parse_origin(STABLE).unwrap(),
            timestamp: Some(t.into()),
            image_digest: "sha256:ccc".into(),
            ..Default::default()
        };
        assert!(!filled(Some(&update("2026-10-02T20:00:00.5Z"))).update_available());
        assert!(!filled(Some(&update("2026-10-02T22:42:16.19Z"))).update_available());
        assert!(filled(Some(&update("2026-10-03T04:11:41.779Z"))).update_available());
        // a time that doesn't order as text doesn't hide it
        assert!(filled(Some(&update("2026-10-02T20:00:00+02:00"))).update_available());
    }

    #[test]
    fn a_queued_rollback_s_origin_is_followed() {
        // rpm-ostree upgrades from the deployment ahead of the booted one
        let rpm = json!({"deployments": [
            {"booted": false, "container-image-reference": "ostree-unverified-registry:r/a:testing"},
            {"booted": true, "container-image-reference": "ostree-unverified-registry:r/a:stable"},
        ]});
        assert_eq!(
            followed_origin(&rpm.to_string()).unwrap(),
            "ostree-unverified-registry:r/a:testing"
        );
    }

    #[test]
    fn deployments_match_by_commit_serial_and_stateroot() {
        let o = json!({"checksum": "abc", "deploySerial": 1, "stateroot": "default"});
        let d = |c: &str, s: u64, n: &str| json!({"checksum": c, "serial": s, "osname": n});
        assert!(same_deployment(&d("abc", 1, "default"), &o));
        assert!(!same_deployment(&d("abc", 0, "default"), &o));
        assert!(!same_deployment(&d("abc", 1, "other"), &o));
        assert!(!same_deployment(&d("abd", 1, "default"), &o));
        assert!(same_deployment(&json!({"checksum": "abc"}), &o));
        assert!(!same_deployment(
            &json!({"checksum": ""}),
            &json!({"checksum": ""})
        ));
    }

    #[test]
    fn skopeo_inspect_as_an_image() {
        let r = parse_origin(STABLE).unwrap();
        let out = r#"{"Digest":"sha256:87cf","Created":"2026-10-02T22:42:16.19Z",
                      "Architecture":"amd64","Labels":{"org.opencontainers.image.version":"44.20261002"}}"#;
        let i = image_from_skopeo(out, &r).unwrap();
        assert_eq!(i.image_digest, "sha256:87cf");
        assert_eq!(i.version.as_deref(), Some("44.20261002"));
        assert_eq!(i.timestamp.as_deref(), Some("2026-10-02T22:42:16.19Z"));
        assert_eq!(i.image, r);
        assert!(image_from_skopeo(r#"{"Digest":""}"#, &r).is_err());
        assert!(image_from_skopeo("not json", &r).is_err());
    }
}
