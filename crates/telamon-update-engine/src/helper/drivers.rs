//! Automatic hardware drivers: which Telamon OS image a machine should run, given
//! the PCI devices it has. See docs/DESIGN.md ("Drivers").
//!
//! Everything here is plain code: the table, the sysfs reading, the decision,
//! the loop rules and the state file. [`Core::auto_drivers`](super::Core)
//! does the switching, through the same code as a channel switch. Nothing
//! takes an image from a caller: the target is built only from [`DRIVERS`],
//! [`BASE_IMAGE`] and the booted ref's registry, transport and tag.

use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use telamon_framework_system::bootc::ImageReference;

/// Where images of Telamon OS live (the booted repo must start with this).
pub const NAMESPACE: &str = "ghcr.io/eternalcoder454/";
/// The image without any driver.
pub const BASE_IMAGE: &str = "atlasos";
pub const STATE_PATH: &str = "/var/lib/atlas-core/drivers.json";
/// The state file is tiny; anything bigger is not ours.
const STATE_MAX: u64 = 64 * 1024;
/// Most PCI devices read (a machine has tens).
const MAX_DEVICES: usize = 4096;

/// A PCI device as sysfs shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PciDevice {
    /// 24 bits: base class, subclass, interface
    pub class: u32,
    pub vendor: u16,
    pub device: u16,
}

/// A driver image. `matches` is asked with the machine's devices.
#[derive(Debug)]
pub struct Driver {
    pub id: &'static str,
    /// the image's name under [`NAMESPACE`]
    pub image: &'static str,
    pub matches: fn(&[PciDevice]) -> bool,
}

impl PartialEq for Driver {
    fn eq(&self, other: &Driver) -> bool {
        self.id == other.id
    }
}

impl Eq for Driver {}

pub const DRIVERS: &[Driver] = &[Driver {
    id: "nvidia",
    image: "atlasos-nvidia",
    matches: nvidia,
}];

/// An NVIDIA GPU (VGA or 3D controller) of Turing (device IDs 0x1e00) or
/// newer: what the open kernel modules support. Pascal and Volta are below.
fn nvidia(devices: &[PciDevice]) -> bool {
    devices.iter().any(|d| {
        d.vendor == 0x10de && matches!(d.class >> 8, 0x0300 | 0x0302) && d.device >= 0x1e00
    })
}

fn hex(text: &str, max: u32) -> Option<u32> {
    let v = u32::from_str_radix(text.trim().strip_prefix("0x")?, 16).ok()?;
    (v <= max).then_some(v)
}

fn read_small(path: &Path) -> Option<String> {
    let mut s = String::new();
    fs::File::open(path)
        .ok()?
        .take(64)
        .read_to_string(&mut s)
        .ok()?;
    Some(s)
}

/// The PCI devices under `root`/sys/bus/pci/devices. Entries that cannot be
/// read are skipped (a machine without PCI gives none).
pub fn scan_pci(root: &Path) -> Vec<PciDevice> {
    let Ok(dir) = fs::read_dir(root.join("sys/bus/pci/devices")) else {
        return Vec::new();
    };
    dir.flatten()
        .take(MAX_DEVICES)
        .filter_map(|e| {
            let p = e.path();
            Some(PciDevice {
                class: hex(&read_small(&p.join("class"))?, 0xff_ffff)?,
                vendor: hex(&read_small(&p.join("vendor"))?, 0xffff)? as u16,
                device: hex(&read_small(&p.join("device"))?, 0xffff)? as u16,
            })
        })
        .collect()
}

/// The booted image as `(repo, tag)`: only a registry image with a plain
/// `stable` or `testing` tag and no digest, whose repo is the base image or a
/// driver image of Telamon OS.
pub fn booted_repo(r: &ImageReference) -> Option<(&str, &str)> {
    if r.transport_or_default() != "registry" || r.image.contains('@') {
        return None;
    }
    let tag = r.channel()?.as_str();
    let repo = r.image.strip_suffix(tag)?.strip_suffix(':')?;
    let name = repo.strip_prefix(NAMESPACE)?;
    (name == BASE_IMAGE || DRIVERS.iter().any(|d| d.image == name)).then_some((repo, tag))
}

/// The driver whose image is `repo`.
pub fn driver_of(repo: &str) -> Option<&'static Driver> {
    let name = repo.strip_prefix(NAMESPACE)?;
    DRIVERS.iter().find(|d| d.image == name)
}

/// Whether `repo` is an image this module may switch to.
pub fn is_known_repo(repo: &str) -> bool {
    repo.strip_prefix(NAMESPACE)
        .is_some_and(|n| n == BASE_IMAGE || DRIVERS.iter().any(|d| d.image == n))
}

/// `booted` on the image `repo`: same transport, tag and signature setting.
/// Refuses a booted ref [`booted_repo`] does not accept and an unknown `repo`.
pub fn retarget(booted: &ImageReference, repo: &str) -> Result<ImageReference, String> {
    let (_, tag) =
        booted_repo(booted).ok_or("the booted image is not a Telamon OS registry image")?;
    if !is_known_repo(repo) {
        return Err(format!("{repo:?} is not a Telamon OS image"));
    }
    Ok(ImageReference {
        image: format!("{repo}:{tag}"),
        transport: booted.transport.clone(),
        signature: booted.signature.clone(),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Not a Telamon OS image from the registry on stable or testing: do nothing.
    NotApplicable(&'static str),
    /// The machine should run `repo` (`driver` is its driver; `None` for the
    /// base image). `key` names the hardware that decided it.
    Target {
        repo: String,
        driver: Option<&'static Driver>,
        key: String,
    },
}

/// Which image a machine with `devices` should run, starting from the booted
/// repo (`ghcr.io/eternalcoder454/atlasos` or a driver image).
pub fn target_image(booted_repo: &str, devices: &[PciDevice]) -> Decision {
    if !is_known_repo(booted_repo) {
        return Decision::NotApplicable("the booted image is not a Telamon OS image");
    }
    let driver = DRIVERS.iter().find(|d| (d.matches)(devices));
    let mut ids: Vec<String> = devices
        .iter()
        .filter(|d| DRIVERS.iter().any(|x| (x.matches)(std::slice::from_ref(d))))
        .map(|d| format!("{:04x}:{:04x}", d.vendor, d.device))
        .collect();
    ids.sort();
    ids.dedup();
    Decision::Target {
        repo: format!("{NAMESPACE}{}", driver.map_or(BASE_IMAGE, |d| d.image)),
        driver,
        key: if ids.is_empty() {
            "none".into()
        } else {
            ids.join(",")
        },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    /// written before the switch; it stays if the run dies in the middle
    Attempting,
    Staged,
    Failed,
}

/// `/var/lib/atlas-core/drivers.json`: the last decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub hardware_key: String,
    pub target: String,
    pub outcome: Outcome,
    pub attempts: u32,
    /// Unix seconds; no try before this
    pub next_try: u64,
    pub decided_at: u64,
}

/// How long to wait after the `attempts`-th failure: 15 minutes, 1 hour,
/// 6 hours, then a day.
pub fn backoff(attempts: u32) -> u64 {
    match attempts {
        0 | 1 => 15 * 60,
        2 => 3600,
        3 => 6 * 3600,
        _ => 24 * 3600,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plan {
    Switch,
    Wait(&'static str),
}

/// Whether to switch to `target` now, given the last decision. At most one
/// switch per (hardware key, target): once staged, never again for the pair
/// (the user may have gone back); a failure waits out its backoff.
pub fn plan(state: Option<&State>, key: &str, target: &str, now: u64) -> Plan {
    match state {
        Some(s) if s.hardware_key == key && s.target == target => match s.outcome {
            Outcome::Staged => Plan::Wait("already switched once for this hardware"),
            // (a clock set back far must not wait for longer than the cap)
            Outcome::Failed | Outcome::Attempting
                if now < s.next_try && s.next_try - now <= backoff(u32::MAX) =>
            {
                Plan::Wait("an earlier try failed; waiting before the next")
            }
            Outcome::Failed | Outcome::Attempting => Plan::Switch,
        },
        _ => Plan::Switch,
    }
}

/// The state written before a try at `now`: it counts as a failed try (with
/// its backoff) until [`finish`] says otherwise, so a run that dies in the
/// middle is not tried again at once.
pub fn begin(prev: Option<&State>, key: &str, target: &str, now: u64) -> State {
    let attempts = prev
        .filter(|s| s.hardware_key == key && s.target == target)
        .map_or(0, |s| s.attempts)
        .saturating_add(1);
    State {
        hardware_key: key.into(),
        target: target.into(),
        outcome: Outcome::Attempting,
        attempts,
        next_try: now + backoff(attempts),
        decided_at: now,
    }
}

/// The state after the try `attempting` ended, `ok` or not.
pub fn finish(attempting: &State, ok: bool, now: u64) -> State {
    State {
        outcome: if ok { Outcome::Staged } else { Outcome::Failed },
        next_try: if ok {
            0
        } else {
            now + backoff(attempting.attempts)
        },
        decided_at: now,
        ..attempting.clone()
    }
}

/// The saved state; none if missing, too big or not ours.
pub fn read_state(path: &Path) -> Option<State> {
    let f = fs::File::open(path).ok()?;
    let mut buf = Vec::new();
    f.take(STATE_MAX).read_to_end(&mut buf).ok()?;
    serde_json::from_slice(&buf).ok()
}

/// Write `state` atomically: a temp file in the same directory, synced, then
/// renamed over `path` (0644).
pub fn write_state(path: &Path, state: &State) -> io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    let mut tmp: PathBuf = path.into();
    tmp.set_extension(format!("json.{}.tmp", std::process::id()));
    let _ = fs::remove_file(&tmp);
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o644)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&tmp)?;
    let res = (|| {
        f.write_all(&serde_json::to_vec(state).map_err(io::Error::other)?)?;
        f.sync_all()?;
        fs::rename(&tmp, path)?;
        fs::File::open(dir)?.sync_all()
    })();
    if res.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(devs: &[(&str, &str, &str)]) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        for (i, (class, vendor, device)) in devs.iter().enumerate() {
            let p = d.path().join(format!("sys/bus/pci/devices/0000:0{i}:00.0"));
            fs::create_dir_all(&p).unwrap();
            fs::write(p.join("class"), format!("{class}\n")).unwrap();
            fs::write(p.join("vendor"), format!("{vendor}\n")).unwrap();
            fs::write(p.join("device"), format!("{device}\n")).unwrap();
        }
        d
    }

    const BASE: &str = "ghcr.io/eternalcoder454/atlasos";
    const NV: &str = "ghcr.io/eternalcoder454/atlasos-nvidia";

    fn decide(booted: &str, devs: &[(&str, &str, &str)]) -> Decision {
        let d = tree(devs);
        target_image(booted, &scan_pci(d.path()))
    }

    fn repo_of(d: Decision) -> String {
        match d {
            Decision::Target { repo, .. } => repo,
            Decision::NotApplicable(w) => panic!("not applicable: {w}"),
        }
    }

    #[test]
    fn an_rtx_3060_gets_the_nvidia_image() {
        let d = decide(BASE, &[("0x030000", "0x10de", "0x2504")]);
        assert_eq!(
            d,
            Decision::Target {
                repo: NV.into(),
                driver: Some(&DRIVERS[0]),
                key: "10de:2504".into()
            }
        );
    }

    #[test]
    fn older_nvidia_audio_and_other_vendors_get_the_base_image() {
        for devs in [
            vec![("0x030000", "0x10de", "0x1b80")], // GTX 1080 (Pascal)
            vec![("0x030200", "0x10de", "0x1dba")], // GV100 (Volta)
            vec![("0x040300", "0x10de", "0x228e")], // audio function
            vec![("0x030000", "0x1002", "0x744c")], // AMD
            vec![("0x030000", "0x8086", "0x2504")], // right ID, wrong vendor
            vec![],
        ] {
            let d = decide(NV, &devs);
            assert_eq!(repo_of(d.clone()), BASE, "{devs:?}");
            assert!(matches!(d, Decision::Target { key, .. } if key == "none"));
        }
    }

    #[test]
    fn a_3d_controller_counts_and_the_key_is_sorted() {
        let d = decide(
            BASE,
            &[
                ("0x030200", "0x10de", "0x2684"),
                ("0x040300", "0x10de", "0x22ba"),
                ("0x030000", "0x10de", "0x2504"),
                ("0x030000", "0x10de", "0x2504"),
            ],
        );
        match d {
            Decision::Target { repo, key, .. } => {
                assert_eq!(repo, NV);
                assert_eq!(key, "10de:2504,10de:2684");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_boundary_id_and_garbage_files() {
        assert_eq!(
            repo_of(decide(BASE, &[("0x030000", "0x10de", "0x1e00")])),
            NV
        );
        assert_eq!(
            repo_of(decide(BASE, &[("0x030000", "0x10de", "0x1dff")])),
            BASE
        );
        // unreadable values are skipped, not guessed
        let d = tree(&[("nonsense", "0x10de", "0x2504"), ("0x030000", "0x10de", "")]);
        assert!(scan_pci(d.path()).is_empty());
        assert!(scan_pci(Path::new("/nonexistent-root")).is_empty());
    }

    #[test]
    fn only_atlasos_images_are_decided_for() {
        for r in [
            "ghcr.io/eternalcoder454/other",
            "ghcr.io/eternalcoder454/atlasos-nvidia2",
            "example.com/eternalcoder454/atlasos",
            "",
        ] {
            assert!(matches!(target_image(r, &[]), Decision::NotApplicable(_)));
        }
    }

    fn r(image: &str, transport: Option<&str>) -> ImageReference {
        ImageReference {
            image: image.into(),
            transport: transport.map(Into::into),
            signature: Some(serde_json::json!("containerPolicy")),
        }
    }

    #[test]
    fn only_stable_or_testing_registry_refs_of_atlasos_count() {
        let ok = |i, t| booted_repo(&r(i, t)).map(|(a, b)| (a.to_string(), b.to_string()));
        assert_eq!(
            ok("ghcr.io/eternalcoder454/atlasos:stable", None),
            Some((BASE.into(), "stable".into()))
        );
        assert_eq!(
            ok(
                "ghcr.io/eternalcoder454/atlasos-nvidia:testing",
                Some("registry")
            ),
            Some((NV.into(), "testing".into()))
        );
        for (i, t) in [
            ("ghcr.io/eternalcoder454/atlasos:latest", None),
            ("ghcr.io/eternalcoder454/atlasos", None),
            ("ghcr.io/eternalcoder454/atlasos:stable@sha256:abc", None),
            ("ghcr.io/eternalcoder454/atlasos@sha256:abc", None),
            (
                "ghcr.io/eternalcoder454/atlasos:stable",
                Some("containers-storage"),
            ),
            ("ghcr.io/eternalcoder454/atlasos:stable", Some("oci")),
            ("/var/lib/img:stable", Some("oci")),
            ("ghcr.io/eternalcoder454/fedora:stable", None),
            ("ghcr.io/someone/atlasos:stable", None),
        ] {
            assert_eq!(ok(i, t), None, "{i} {t:?}");
        }
    }

    #[test]
    fn retarget_moves_the_repo_and_keeps_tag_transport_and_signature() {
        let b = r("ghcr.io/eternalcoder454/atlasos:testing", Some("registry"));
        let n = retarget(&b, NV).unwrap();
        assert_eq!(n.image, "ghcr.io/eternalcoder454/atlasos-nvidia:testing");
        assert_eq!(n.transport.as_deref(), Some("registry"));
        assert_eq!(n.signature, b.signature);
        let back = retarget(&n, BASE).unwrap();
        assert_eq!(back.image, "ghcr.io/eternalcoder454/atlasos:testing");
        assert!(retarget(&b, "ghcr.io/evil/atlasos").is_err());
        assert!(retarget(&b, "ghcr.io/eternalcoder454/atlasos-nvidia:stable").is_err());
        assert!(retarget(&r("quay.io/x/y:stable", None), NV).is_err());
    }

    fn st(outcome: Outcome, attempts: u32, next_try: u64) -> State {
        State {
            hardware_key: "k".into(),
            target: "t".into(),
            outcome,
            attempts,
            next_try,
            decided_at: 1,
        }
    }

    #[test]
    fn at_most_one_switch_per_hardware_and_target() {
        assert_eq!(plan(None, "k", "t", 10), Plan::Switch);
        let staged = st(Outcome::Staged, 1, 0);
        assert!(matches!(plan(Some(&staged), "k", "t", 10), Plan::Wait(_)));
        // another pair is a new decision
        assert_eq!(plan(Some(&staged), "k2", "t", 10), Plan::Switch);
        assert_eq!(plan(Some(&staged), "k", "t2", 10), Plan::Switch);
    }

    fn failed(prev: Option<&State>, now: u64) -> State {
        finish(&begin(prev, "k", "t", now), false, now)
    }

    #[test]
    fn a_failure_waits_with_growing_backoff_and_never_retries_at_once() {
        let s = failed(None, 1000);
        assert_eq!(
            (s.outcome, s.attempts, s.next_try),
            (Outcome::Failed, 1, 1900)
        );
        assert!(matches!(plan(Some(&s), "k", "t", 1000), Plan::Wait(_)));
        assert!(matches!(plan(Some(&s), "k", "t", 1899), Plan::Wait(_)));
        assert_eq!(plan(Some(&s), "k", "t", 1900), Plan::Switch);
        let s2 = failed(Some(&s), 2000);
        assert_eq!((s2.attempts, s2.next_try), (2, 2000 + 3600));
        let s3 = failed(Some(&s2), 0);
        assert_eq!(s3.next_try, 6 * 3600);
        let s4 = failed(Some(&s3), 0);
        assert_eq!(s4.next_try, 24 * 3600);
        assert_eq!(failed(Some(&s4), 0).next_try, 24 * 3600);
        // success after failures
        let ok = finish(&begin(Some(&s2), "k", "t", 5000), true, 5000);
        assert_eq!(
            (ok.outcome, ok.attempts, ok.next_try),
            (Outcome::Staged, 3, 0)
        );
        // a new pair starts over
        assert_eq!(begin(Some(&s4), "k2", "t", 0).attempts, 1);
    }

    #[test]
    fn a_run_that_died_mid_switch_waits_like_a_failure() {
        let a = begin(None, "k", "t", 1000);
        assert_eq!(a.outcome, Outcome::Attempting);
        assert!(matches!(plan(Some(&a), "k", "t", 1100), Plan::Wait(_)));
        assert_eq!(plan(Some(&a), "k", "t", 1900), Plan::Switch);
    }

    #[test]
    fn a_next_try_far_in_the_future_is_a_clock_that_went_back() {
        let s = st(Outcome::Failed, 1, 10_000_000);
        assert_eq!(plan(Some(&s), "k", "t", 5), Plan::Switch);
    }

    #[test]
    fn the_state_file_is_written_whole_and_read_back() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("sub/drivers.json");
        assert_eq!(read_state(&p), None);
        let s = st(Outcome::Failed, 2, 99);
        write_state(&p, &s).unwrap();
        write_state(&p, &s).unwrap();
        assert_eq!(read_state(&p), Some(s));
        let text = fs::read_to_string(&p).unwrap();
        assert!(text.contains("\"outcome\":\"failed\""), "{text}");
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&p).unwrap().permissions().mode() & 0o777,
            0o644
        );
        let left: Vec<_> = fs::read_dir(p.parent().unwrap())
            .unwrap()
            .flatten()
            .collect();
        assert_eq!(left.len(), 1, "no temp file left");
        fs::write(&p, "not json").unwrap();
        assert_eq!(read_state(&p), None);
        fs::write(&p, vec![b' '; 100_000]).unwrap();
        assert_eq!(read_state(&p), None);
    }
}
