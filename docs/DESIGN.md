# Telamon Updater and its system helper: design

Telamon OS is a Fedora Kinoite 44 bootc image (`ghcr.io/eternalcoder454/atlasos`).
This repo holds two things:

- **telamon-update-engine**: the privileged system helper, its D-Bus client and
  the progress parser (packaged as telamon-system-helper; it was
  atlas-system-helper until 0.3.0, and atlas-core before that).
- **telamon-updater**: the Telamon Updater app (`net.eterneon.telamon.updater`;
  it was Atlas Updater, `net.eterneon.atlas.updater`, until 0.3.0).

Telamon.Ui, the QML module every Telamon app shares, its Telamon Symbols
fonts, the design rules for Telamon apps and the app template live in
the **Telamon framework** (`EternalCoder454/atlas-framework` on GitHub until
the repository is renamed, `~/Documents/Atlas Framework`). The app uses the
installed Telamon.Ui (the telamon-ui package). The Rust code Telamon apps
share is there too: telamon-framework-core (os-release),
telamon-framework-system (bootc types, history, events, crash reports) and
telamon-framework-flatpak (the libflatpak wrapper). Both crates here use them
directly, pinned to one git rev.

Stack: Rust + Qt 6.11 + Kirigami 6.30 through CXX-Qt. Everything builds and runs
on Fedora 44 (Qt 6.11.2, KF6 6.30, bootc 1.16.13, flatpak 1.18.2, polkit 127,
Rust 1.98).

## Layout

```
Cargo.toml                    workspace: the crates and apps below
crates/telamon-update-engine/   lib `telamon_update_engine` + bin `telamon-system-helper`
  src/helper_client.rs        zbus proxy for the system helper (what apps call)
  src/progress.rs             progress of a running Upgrade/SwitchChannel
  src/helper/                 helper logic: bootc runner, polkit, retry, layered
  src/bin/telamon-system-helper.rs  the D-Bus system service
  data/                       D-Bus, polkit, systemd files for the helper
crates/telamon-updater-base/    what the window and the tray share: settings (rc), ops,
                              schedule, view, restart, locks, worker result, the fwupd
                              client, and the notifier (telamon_framework_system::notify,
                              feature `notify`)
apps/telamon-updater/           the window (CMake + Corrosion, or cxx-qt-build), and the
                              app jobs it runs for the tray (`--worker`, no Qt)
apps/telamon-updater-tray/      the resident tray: plain Rust, no Qt (zbus, inotify)
  data/                       its D-Bus session service file
  qml/                        Kirigami UI, compiled ahead of time (qmlcachegen)
  data/                       .desktop, autostart .desktop, .notifyrc, metainfo, icon
packaging/telamon-updater.spec  one spec: the app is the main package `telamon-updater`,
                              the helper the subpackage `telamon-system-helper`
packaging/build-rpm.sh        builds the RPMs inside fedora:44: build-rpm.sh <out dir>
```

## System helper (telamon-system-helper)

bootc has no D-Bus API and needs root, so apps go through a small system
service. It is D-Bus activated, so nothing runs at idle; it exits after 60 s
without calls.

- Binary: `/usr/libexec/telamon-system-helper`
- Bus name: `net.eterneon.telamon.SystemHelper` (system bus)
- Object: `/net/eterneon/telamon/SystemHelper`
- Interface: `net.eterneon.telamon.SystemHelper1`

The helper also answers under its **old identity** for this release (see
"Old names" below): the same six methods and the `Progress` property, from the
same process.

| Method | Returns | Runs | polkit action | Default |
|---|---|---|---|---|
| `Status()` | `s` JSON | `bootc status --json` | `net.eterneon.telamon.system.status` | yes (any) |
| `CheckForUpdate()` | `s` JSON | `bootc upgrade --check`, then `bootc status --json` | `net.eterneon.telamon.system.check` | yes (active) |
| `Upgrade()` | `s` JSON | `bootc upgrade` (stages only; never `--apply`); refuses a downgrade (below) | `net.eterneon.telamon.system.upgrade` | auth_admin_keep; wheel: yes (rules.d) |
| `Rollback()` | `s` JSON | `bootc rollback` | `net.eterneon.telamon.system.rollback` | auth_admin_keep |
| `CancelRollback()` | `s` JSON | `bootc rollback` again, only while one is queued | `net.eterneon.telamon.system.rollback` | auth_admin_keep |
| `SwitchChannel(s channel)` | `s` JSON | `bootc switch <ref with tag = channel>`; refuses a downgrade (below) | `net.eterneon.telamon.system.switch-channel` | auth_admin_keep |

The returned JSON string is `bootc status --json` after the action.

Rules:
- **Only these six methods.** No method takes a command, path, image ref or
  argument list. `channel` must be exactly `stable` or `testing`. The new ref
  is the booted image's own ref with only the tag replaced
  (`ghcr.io/eternalcoder454/atlasos:stable` → `:testing`; an `oci:` or
  local-registry ref in a test VM works the same way). The transport stays
  the same. Anything else is rejected with `net.eterneon.telamon.Error.InvalidArgument`.
- bootc runs by absolute path (`/usr/bin/bootc`) with a fixed argv and a clean
  environment (no inherited PATH or LD_*).
- polkit: `CheckAuthorization` on `org.freedesktop.PolicyKit1.Authority`, with
  subject `system-bus-name` (the caller's unique name) and the
  AllowUserInteraction flag. A failed check gives `net.eterneon.telamon.Error.NotAuthorized`.
- One operation at a time. A second call while one runs gets `net.eterneon.telamon.Error.Busy`.
- bootc failures give `net.eterneon.telamon.Error.Failed`, with bootc's stderr
  (last 4 KB) as the message.
- Calls take minutes (`Upgrade` downloads the image), so clients use no method
  timeout.
- `Status` is read-only: it never takes the busy flag, one `bootc status` runs
  at a time and its result is shared for 2 s. An operation that changes the
  system shares the status it returns the same way. `Upgrade` and
  `SwitchChannel` read the status once before they start and reuse it (a
  system with local rpm-ostree changes goes to rpm-ostree at once).
- While the helper exits (idle or SIGTERM) it releases its bus name first;
  calls that still reach it get `net.eterneon.telamon.Error.ShuttingDown`, and
  the client retries once. On SIGTERM a running bootc gets 40 s, then SIGTERM.
  No shutdown inhibitor: ostree pulls transactionally and stages atomically,
  so an interrupted bootc leaves the system unchanged (from ostree's design;
  verify in the VM).
- bootc runs in its own process group with a timeout (2 min status/check,
  60 min upgrade/rollback/switch) and 4 MiB output caps.
- D-Bus policy: anyone may call the interface (polkit decides). Only root may own the name.
- Each identity has its own D-Bus policy file and activation file; both
  activation files start `telamon-system-helper.service`, which takes both
  names (`BusName=` is the new one). If another helper (one from before the
  upgrade, still running) owns the old name, the new helper serves the new
  name only until that one is gone.

**Downgrades.** Signatures prove who built an image, not that it is the
newest: a tag moved back to an older signed build (with its old security
holes) would install like an update. So an image older than the booted or
the staged image of the same reference (and not that image itself) is not
an update (`Status::is_downgrade`; `ImageStatus::is_older_than`: the version labels,
compared as numbers, or the build times say it is older and neither says
the opposite; nothing comparable means not older). `available_update`
leaves it out, so the app never offers it; `Upgrade` and `SwitchChannel`
(a switch to the followed channel pulls its tag too) first ask the registry
with one quick `skopeo inspect` (30 s, no retries, bootc's `auth.json` if
there is one) and refuse an older image before anything is downloaded
(`DOWNGRADE_REFUSED`, "Nothing was downloaded"); if the registry can't be
asked, that is logged and the pull goes on. Then they check what they staged
against the status from before (or, if that couldn't be read, the booted
image after) and, for a downgrade, remove it again with `rpm-ostree cleanup
-p` and fail with `helper_client::DOWNGRADE_REFUSED` and the two versions
(recorded as `update-failed` / `channel-switch-failed`). That also removes
what was staged before (it was replaced), and the message says so. The
background stager skips it the same way (Telamon OS side), asking the registry
with skopeo when the image ref points at no deployment (as after such a
removal), and skipping the run if it can't. Go Back and a switch to the
other channel remain the ways to an older build; an image of another
reference is never compared.

**Retries.** The steps that fetch from the registry (`bootc upgrade
--check`, `bootc upgrade`, `bootc switch`, `rpm-ostree upgrade`/`rebase`,
`skopeo inspect`) are run up to 3 times, 3 s and then 10 s apart, when
the last lines of their error name a passing network problem (a dropped
or refused connection, DNS, a timeout, the registry's 5xx or 429) and
nothing in it says retrying can't help (a signature or policy refusal, a
denied login, a missing image, a full disk) (`helper::retry`). Each is
idempotent: ostree keeps what was fetched and stages atomically, and the
downgrade check runs after the last try. Rollback and CancelRollback are
never retried (`bootc rollback` toggles). On SIGTERM a waiting retry
gives up at once and the operation ends as interrupted (not recorded as
failed). A later try that fails another way keeps the first network error
in its message. The app says the common failures plainly (network, disk
space, signature) instead of the tool's last lines.

**Signatures on a switch.** `SwitchChannel` keeps the booted image's
signature setting (`containerPolicy` → `--enforce-container-sigpolicy`; an
other one it can't carry is refused). An unverified one (none, or
`insecure`) becomes `--enforce-container-sigpolicy` when
`/etc/containers/policy.json` demands a signature (`signedBy` or
`sigstoreSigned`) for the new image under its most specific matching scope
and its `default` does not accept anything (`bootc::policy_requires_signature`);
on a system with local rpm-ostree changes the rebase target becomes
`ostree-image-signed:docker://…` the same way. Every pull is checked under
that policy anyway; recording it in the origin keeps `bootc status` honest
and saves the stager a second deployment to record it.

**Progress.** The interface has one read-only property, `Progress` (`s`, not a
method, so the six-method rule stands): while `Upgrade` or `SwitchChannel`
runs, the JSON of `telamon_update_engine::progress::Progress`, otherwise `""`.

```json
{"op":"upgrade","stage":"downloading","done":123,"total":300028591,"detail":""}
```

- `op` is `upgrade` or `switch`. `stage` is `downloading` (`done` and `total`
  in bytes) or `installing` (steps); `total` 0 means unknown, so show an
  indeterminate bar. `detail` is a step name such as `Deploying Image`, or empty.
- `PropertiesChanged` is sent at most about 4 times a second, but at once on a
  stage change and when the property goes back to `""` (the operation ended,
  failed or not). Read it once after connecting (`HelperClient::progress`),
  then follow `HelperClient::progress_changes`.
- bootc: `upgrade` and `switch` get `--progress-fd <n>` (right after the
  subcommand), where `n` is the write end of a pipe the helper made. Its
  cleared FD_CLOEXEC is set in the child only (`pre_exec`), and the helper
  closes its own copy after the spawn. The helper parses bootc's JSON lines:
  `pulling` is the download (bytes done of bytes still needed); `importing` and
  `staging` are one installing range (steps of both added up).
- rpm-ostree (`upgrade`, `rebase`): stdout is parsed as it arrives. Download
  total is the sum of the `ostree chunk layers needed` and `custom layers
  needed` sizes, done the sum of the `Fetching layer` and `Fetching ostree
  chunk` lines finished so far (never more than the total); with nothing
  needed it goes straight to installing. Installing is the fixed list
  Checking out tree, Importing rpm-md, Resolving dependencies, Checking out
  packages, Running scripts, Writing rpmdb, Writing OSTree commit, Staging
  deployment: `done` is the index of the current step, `total` is 8. Other
  lines are ignored.
- A bootc that rejects `--progress-fd` (its stderr says the argument is unexpected, unrecognized or unknown, and it reported no progress) is run once
  more without it. A progress pipe whose fd would be 0 to 2 is not used.
- Progress is an addition: the output caps, timeouts and interruption
  handling are the same, nothing from the caller reaches argv, and if the
  progress pipe can't be made the operation runs without progress.

**Local rpm-ostree changes.** On a system with packages added by
`rpm-ostree install`, replaced or removed base packages or a regenerated
initramfs, bootc refuses `upgrade` and `switch` ("Deployment contains local
rpm-ostree modifications") and shows those deployments as `incompatible`,
with no image, version or channel. When bootc shows that on the booted or
staged deployment, the helper (`helper/layered.rs`):
- fills the missing `image` of those entries in from `rpm-ostree status
  --json` (the deployment with the same ostree commit) and `spec.image` from
  rpm-ostree's first deployment, so the JSON the methods return reads as
  bootc's would;
- `CheckForUpdate`: `skopeo inspect` of that image (rpm-ostree's own
  `upgrade --check` never looks at the image), kept in
  `/var/lib/atlas-core/layered-update.json` and shown as the booted entry's
  `cachedUpdate` while it is for the image followed and newer than the
  booted and staged images (the stage timer stages without the helper);
  `Upgrade` and `SwitchChannel` delete it;
- `Upgrade`: `rpm-ostree upgrade` (stages only; never `--reboot`);
- `SwitchChannel`: `rpm-ostree rebase <origin with tag = channel>`.

rpm-ostree and skopeo keep the added packages and run like bootc: absolute
paths (`/usr/bin/rpm-ostree`, `/usr/bin/skopeo`), fixed argv, clean
environment, the same timeouts and caps. `Rollback` is `rpm-ostree rollback`
there too: a second `bootc rollback` doesn't undo the first on such a system.

**History.** `telamon-system-helper record-boot` (CLI mode, run as root by
`telamon-record-boot.service`, a oneshot at boot) appends one line to
`/var/lib/atlas-core/history.jsonl` when the booted image digest differs from
the last line:
`{"version":"44.20261002","digest":"sha256:…","image":"ghcr.io/…:stable","timestamp":"<image build time>","first_booted":"<RFC3339 now>"}`.
The file is world-readable (0644), and the directory comes from
`StateDirectory=atlas-core`.

**State directory name.** `/var/lib/atlas-core` keeps the package's old name on
purpose. `/var` is shared by every deployment, so after a rollback the previous
image's helper reads and writes the same files; a new path would split the
history and events between images. Don't rename it.

## Drivers

The system moves itself to the image its hardware needs, with no opt-in, and
back to the base image when the hardware is gone. This adds **no D-Bus
method**: the helper keeps exactly its six. Nothing takes an image from a
caller; the target is built from the table below plus the booted ref's
registry, transport, tag and signature setting.

| Driver id | Image (under `ghcr.io/eternalcoder454/`) | Needs |
|---|---|---|
| (none) | `atlasos` | no driver hardware |
| `nvidia` | `atlasos-nvidia` | an NVIDIA GPU of Turing or newer |

Both images carry the same tags (`stable`, `testing`) and the same cosign
signature; the containers policy covers both. The code is
`helper/drivers.rs` (`Driver { id, image, matches }`, `DRIVERS`,
`target_image(booted_repo, devices) -> Decision`); a driver is a table entry.

- **Detection.** `/sys/bus/pci/devices/*/{class,vendor,device}`. NVIDIA
  matches vendor `0x10de`, class `0x0300xx` or `0x0302xx` (display
  controllers; the audio function is ignored) and device ID `>= 0x1e00`
  (TU1xx and newer, what the open kernel modules support; Pascal and Volta
  are below). `hardware_key` is the sorted, de-duplicated `vendor:device` of
  every matched device, or `none`.
- **Acts only** on a booted registry image whose repo is exactly
  `ghcr.io/eternalcoder454/atlasos` or `.../atlasos-nvidia` with the tag
  `stable` or `testing` (no digest). Anything else (other refs, local
  builds, other transports, no container image) does nothing and logs one
  line. If the staged deployment already is the target it does nothing.
- **When.** `telamon-drivers.timer` (OnBootSec=2min, OnUnitActiveSec=6h,
  Persistent, RandomizedDelaySec=5min; enabled by the preset) starts
  `telamon-drivers.service` (oneshot, after `network-online.target`, hardened
  like the helper), which runs `telamon-system-helper drivers`. At the start of
  the helper's `CheckForUpdate` the helper only plans (sysfs and the state
  file) and, if a switch is due, asks systemd (`StartUnit
  telamon-drivers.service`, mode `replace`, not waited for) to run it: the pull
  never runs inside the D-Bus call, and a failure never fails the check.
- **Only a checked pull.** The step skips (logs why) unless the booted origin
  is signed (`containerPolicy`) or the containers policy demands a signature
  for the target; an unreadable policy skips.
- **Switching** is the channel switch's code (`Op::SwitchDriver`, internal,
  not reachable over D-Bus): same `--enforce-container-sigpolicy` / signed
  origin rules, the pre-pull downgrade check, staged for the next boot, never
  applied. An operation lock file, `/run/telamon-system-helper.lock`, and
  `/run/atlas-system-helper.lock`, its name until 0.3.0 (both root's, 0600,
  `O_NOFOLLOW`, `flock`; both are taken, the new one first, and given up
  together, so the OS image's scripts and a helper from before the upgrade,
  which know only the old one, are kept out as well), is shared by every helper process
  and taken by every changing operation (upgrade, switch, rollback, driver
  switch), so a driver switch never overlaps another operation; if it is
  held, the driver step skips this round. The holder's name is written into
  the lock file; a caller refused because a driver switch holds it is told "A
  graphics driver is being installed. Try again in a few minutes."
- **No loops, offline-safe.** `/var/lib/atlas-core/drivers.json` (0644,
  written atomically: temp file, fsync, rename):
  `{hardware_key, target, outcome: "attempting"|"staged"|"failed", attempts, next_try,
  decided_at}`. At most one switch per (hardware_key, target): once staged,
  the pair is never switched again, even if the booted image is not the
  target (the user used Go Back). `attempting` (with the backoff already set) is written before the pull and
  the switch does not start if that write fails, so a run that dies in the
  middle counts as a failure. A failure records `failed` and waits
  15 minutes, 1 hour, 6 hours, then 24 hours; a retry is never in the same
  run (the `drivers` run does not retry transient errors either). Offline is
  a failed try.
- **Events.** A switch that staged appends `driver-install` or
  `driver-remove` to `events.jsonl`, the driver id in `version` (the crash
  reports skip it). The tray watches the file (always, not only with crash
  reports on), announces the newest unseen one once (`DriverEventTime` in
  the `Notified` group; without one, events since this boot began, from
  `btime` in `/proc/stat`, count) with the notifyrc event `driverStaged`: "NVIDIA graphics driver
  installed — restart to finish" or "... removed ...". With Secure Boot on (last
  byte of the `SecureBoot-8be4df61-...` EFI variable is 1) an install adds
  "After the restart, follow the prompt to confirm the driver's key."; the
  image's `nvidia-key-setup` does the key enrollment. The staged deployment
  also puts the tray in its usual restart state.

## Flatpak wrapper (telamon-framework-flatpak)

This uses libflatpak through the `libflatpak` crate (gtk-rs style). System
installs go through flatpak's own system helper, which asks polkit itself, so
our helper is not involved.

```rust
pub struct AppUpdate { pub id: String, pub name: String, pub branch: String,
                       pub installation: Installation /* System | User */,
                       pub download_size: u64, pub current_version: Option<String>,
                       pub new_version: Option<String> }
pub fn list_updates(refresh: bool) -> Result<Vec<AppUpdate>>;   // system + user, apps and runtimes
pub fn list_updates_with(refresh: bool, no_interaction: bool) -> Result<Vec<AppUpdate>>;
pub fn update_all(progress: impl FnMut(Progress)) -> Result<()>;  // one transaction per installation,
                                                                   // other installations as dependency sources
pub fn update(opts: &UpdateOptions, progress: impl FnMut(Progress)) -> Outcome;
// UpdateOptions { no_interaction, hold_new_permissions, check_only }
// Outcome { updated: Vec<Updated>, held_back: Vec<Held { app, permissions }>, error: Option<Error> }
```

`update` is for runs nobody asked for: `no_interaction` makes a step that
would need a password fail instead of asking, and `hold_new_permissions`
aborts each transaction in `ready-pre-auth` (before anything downloads or
asks polkit) when an update's metadata grants more than the installed one,
then runs it again without those apps. The metadata is parsed with GKeyFile,
as flatpak parses it. Every group counts except the ones that grant nothing
(`[Application]`, `[Runtime]`, `[ExtensionOf]`, `[Build]`, `[Extension *]`,
`[Extra Data]`):
a `[Context]` list that grants something new once its items are folded in
order (`x` grants, a later `!x` takes it away; `filesystems` items by path
without trailing slashes, `:ro` < `:rw` < `:create`; a `:reset`, a `!` with
a suffix or a backslash escape counts as unreadable rather than guessed
at), D-Bus names at a higher
level (an unknown level counts as the highest), any other value that
changed at all (`[Environment]`, `[USB Devices]`, `[Policy *]` and groups
flatpak may add later), and a different runtime or SDK ID (a new branch of
the same runtime is routine). A new file or value that can't be read counts
as new permissions. An app the update would install (an end-of-life rename)
is held too, put down to the update that brought it in (its related
operations, else the app the rename uninstalls); if it can't be put down to
one, that pass's apps are all held ("new permissions somewhere in this
update") and runtimes go on. A runtime's own `[Context]` applies to its
apps, so a runtime update that grants more (any group but
`[Environment]`, which changes routinely) is held as well, and so is a
runtime branch an update would install that grants anything (measured
against nothing; Flathub's and Fedora's runtimes grant nothing). A
runtime with any group beyond what it is, where its extensions mount and
its environment (a `[Context]`, a bus policy, USB devices, anything newer)
is read as strictly as an app, and measured against nothing when its
installed metadata can't be found. One without goes on even if part of it
can't be read, as does one with no new metadata at all (runtimes update
often and routinely). A list item with spaces around it is unreadable, as
flatpak keeps them.
Everything one app asks for, however it was found, is one held entry.
`no_interaction` is also set on the installations, so the listing and
remote refresh before the transaction can't prompt either.
Text from remotes (app names, versions, permission items, error messages)
is cleaned with `clean`/`clean_to` before it is shown or logged: control
characters become spaces, invisible and direction-changing ones (Unicode
Cf, Zl, Zp) go, at most 80 characters (300 for errors). Every installation
is tried; the first error is returned with what was updated before it and
what was held back.

`list_updates` reads cached metadata unless asked to refresh: it takes
`refresh: bool`, which updates appstream and summary first.

## Firmware (fwupd)

Firmware updates come from fwupd (2.1.8 on Fedora 44), over its system bus
API `org.freedesktop.fwupd` at `/` (interface `org.freedesktop.fwupd`). fwupd
asks polkit itself, so our helper is not involved, and it is D-Bus activated.
Firmware never installs by itself: only the user's press of "Install" in
the window installs it.

`crates/telamon-updater-base/src/fwupd.rs` (zbus only, used by the tray and
the window):

```rust
pub struct FirmwareUpdate { pub device_id: String, pub device: String /* Name */,
    pub vendor: String, pub current: String, pub version: String,
    pub summary: String, pub description: String /* plain text */,
    pub urgency: Urgency, pub size: u64, pub checksums: Vec<String>,
    pub locations: Vec<String>, pub remote_id: String, pub trusted: bool,
    pub needs_reboot: bool, pub needs_shutdown: bool, pub internal: bool }
pub struct Pending { pub device_id: String, pub device: String, pub version: String,
                     pub state: PendingState /* Reboot | Failed(String) */ }
pub struct Listing { pub updates: Vec<FirmwareUpdate>, pub pending: Vec<Pending>,
                     pub metadata_age: Option<Duration> /* newest enabled download remote */ }
pub async fn available(conn: &zbus::Connection) -> Result<bool>;  // fwupd activatable or running
pub async fn list(conn: &zbus::Connection) -> Result<Listing>;
pub async fn install(conn, device_id, fd: OwnedFd, progress: impl FnMut(Progress)) -> Result<Done>;
pub fn notice_key(updates: &[FirmwareUpdate]) -> String;  // hash of (device_id, version) pairs
```

- `list` calls `GetDevices`, then `GetUpgrades(DeviceId)` for each device
  with the `updatable` flag (bit 1) and without `updatable-hidden` or
  `locked`; `org.freedesktop.fwupd.NothingToDo` and `NotSupported` mean no
  update. Only the first (newest) release counts, and only one whose
  `TrustFlags`/`Flags` say `is-upgrade` (bit 2) and not `blocked-version`
  or `blocked-approval`. Devices with `UpdateState` pending or
  needs-reboot are listed as `Pending::Reboot`; failed with their
  `UpdateError`.
- Device flags read: `internal` (bit 0), `updatable` (1), `locked` (4),
  `needs-reboot` (8), `needs-shutdown` (17), `usable-during-update` (29),
  `updatable-hidden` (37). Release `Checksum` is a comma-separated list
  (SHA-1 and SHA-256 hex), `Locations` a list of URLs, possibly relative
  to the remote (`./name.cab`), `Description` AppStream markup (`<p>`,
  `<ul>`/`<ol>`/`<li>`, `<em>`, `<code>`), turned into plain text (paragraphs,
  "• " items) and never shown as rich text. Text from fwupd (names,
  versions, summaries, descriptions, errors) is cleaned as Flatpak remote
  text is (`clean`/`clean_to`; descriptions up to 2,000 characters).
- `metadata_age`: from `GetRemotes`, the newest `ModificationTime` among
  enabled download remotes (`Type` 1); `None` when none was ever fetched.
  The metadata is refreshed by `fwupd-refresh.timer` (the Telamon OS image
  enables it); the app adds no poller of its own.
- `install` first calls `SetFeatureFlags` with `detach-action`,
  `update-action`, `requests`, `requests-non-generic` and
  `allow-authentication` (bits 1, 2, 4, 9, 8) on the same connection, so
  fwupd can ask polkit with a prompt and send `DeviceRequest`s, then
  `Install(device_id, fd, {})` with no options (no reinstall, no older
  version, no branch switch) and no method timeout (an install can take
  minutes). While it runs it follows the daemon's `Status` and
  `Percentage` properties and `DeviceRequest` signals (`Message` in words,
  or the request `Id` such as `org.freedesktop.fwupd.request.remove-replug`
  said plainly). Errors in plain words: `AuthFailed`/`PermissionDenied` →
  cancelled (as polkit refusals are elsewhere), `BatteryLevelTooLow`,
  `NeedsUserAction`, `NothingToDo`, `NotSupported`, `AlreadyPending`,
  anything else as fwupd's message.

The window (`apps/telamon-updater/src/firmware.rs`) downloads the file before
`install`. The row carries the release's `trusted` flag and the checksum it
picked; `installFirmware(deviceId, version, checksum)` reads the release
again by `GetUpgrades` and refuses ("This update changed since it was
shown. Check again.") unless the version and the picked checksum are the
ones shown. A release that fwupd does not mark trusted (neither
`trusted-payload` nor `trusted-metadata` in `TrustFlags`, bits 0 and 1;
fwupd checks the payload again itself when it installs) is never
downloaded or installed ("This update is not signed by a trusted source,
so Telamon Updater won't install it."); the row says "Not signed by a trusted
source" and has no Install button. The first location that is an
absolute `https://` URL, or a relative one joined to the remote's
`FirmwareBaseUri` (which must be `https://`), is fetched with ureq (rustls,
15 s connect, 10 minutes in all, redirects only to `https://`) into an
in-memory `memfd`, never a file on disk, at most 256 MiB (the release's
`Size` is not the size of the cabinet, so it is not used as a cap). The
`memfd` is sealed (no write, grow or shrink, which also stops writes
through descriptors opened before) right after the download, and only
then is its SHA-256 checked, reading from the sealed file, so the bytes
checked are the bytes fwupd gets (SHA-1 is used only when no SHA-256 is
given, and only for a trusted release; with neither, nothing installs).
This check guards the transfer and pins the release to the one the user
saw; it is not an independent trust anchor, because the checksum comes
from fwupd. What authenticates the firmware is fwupd checking the cabinet
against its signed metadata. Download errors are logged by kind only (a
redirect URL can carry a token). Remote hosts that are `localhost` or an
IP address (loopback, private, link-local, unspecified, shared or unique-local) are refused. Debug
builds with `TELAMON_UPDATER_FIRMWARE_FILES` set to a folder (the test rig,
`tools/fwupd-rig.sh`, whose local remote has no `FirmwareBaseUri`) read a
relative location from that folder instead, by file name only (no `/` or
`..`, symlinks refused). One firmware operation runs
at a time (`$XDG_RUNTIME_DIR/telamon-updater-firmware.lock`); fwupd also runs
one at a time.

Nothing restarts or quits while a firmware install runs: the window's
"Restart" buttons are off and `restartNow` refuses, the window stays until
the install ends after it is closed, and the tray postpones a scheduled
restart (looking again each minute, saying so in its tooltip) and refuses
"Restart Now" while the firmware lock is held. The wait for fwupd's
`Install` reply ends after at most one hour with "result unknown, check the
device's firmware version"; a reply already queued when fwupd leaves the
bus still counts. The "Restart to finish installing firmware" row follows
fwupd's listing (a device pending a reboot), not only this session's install.

## Telamon Updater app

- Binary and package: `telamon-updater`. App ID: `net.eterneon.telamon.updater`.
- Two programs. `telamon-updater-tray` autostarts at login and runs all
  session: the panel icon, the schedule and the notifications. It has no Qt
  and loads no libflatpak, so it stays at a few MB (2.5 MB PSS idle in the
  test rig, against 94 MB for the Qt tray it replaced). `telamon-updater` is
  the window: started when the user opens it (from the panel icon, a
  notification or the menu), it quits once the window is closed and no
  operation runs. A second launch raises the existing window (single
  instance through D-Bus on the session bus, `net.eterneon.telamon.updater`),
  passing on `--page <updates|settings|reports|sent>` and `--check`.
  `telamon-updater --tray`, from older autostart entries, hands over to
  `telamon-updater-tray`.
- Update glow ("the system is being changed": an update, switch or rollback
  being staged, apps being updated, or a firmware install running; not checks): `ScreenGlow.qml` draws
  it around the edges of every screen, not inside the window: one continuous
  frame per screen (`GlowFrame.qml`, 3 grid units deep, accent colour with a
  lighter rim at the edge, fading inward). Four straight bands with linear
  gradients and four corner squares with radial gradients (centred on the
  inner corner, so the glow turns each corner in a quarter circle) share one
  set of stops and meet edge to edge without antialiasing: no seam, gap or
  overlap. On Wayland each screen gets one full-screen, transparent layer-shell
  overlay (`org.kde.layershell`, scope `telamon-updater-glow`, anchored to all
  four edges, no keyboard, exclusion zone -1, and `WindowTransparentForInput`,
  which gives it an empty input region). On X11 a full-screen transparent
  window would black out the screen without a compositor, so there are four
  frameless always-on-top tool windows per screen, one strip along each edge,
  each showing its part of the same screen-sized frame. The windows exist
  only while the glow is on (none and no timer otherwise) and are rebuilt
  when screens come or go. Without the layer-shell module, or on another
  platform, the glow falls back to `TelamonEdgeGlow` inside the window. It
  breathes: one opacity per window, 0.6 to 1 over 2.4 s, set at 30 frames per
  second from a timer, so the gradients are never redrawn. It is static (0.9)
  under reduced motion (`TelamonStyle.reducedMotion`, or Plasma's animation
  speed at instant) and with software rendering (`TelamonStyle.softwareRendering`:
  the software scene graph or a software GL driver such as llvmpipe, and
  `TELAMON_SOFTWARE_RENDERING=0/1` overrides it; with a Telamon.Ui older than
  1.5.0, the scene graph API and `Shell::watchRenderer`'s GL renderer check). The app never asks for the software scene graph,
  so it draws on the GPU wherever there is a hardware GL driver. Closing the
  window while it is on keeps the window's QML (hidden) alive until the
  operation ends, so the glow stays. Developer option, honoured in every
  build because it only draws: `TELAMON_UPDATER_GLOW_DEMO=1 telamon-updater`
  turns the glow on while the window is open, with nothing running (closing
  the window ends it).
- The tray owns the session bus name `net.eterneon.telamon.updater.Tray`
  (one tray per session; a second one exits) with one method, `Reload()`,
  at `/net/eterneon/telamon/updater/Tray`: read `telamon-updaterrc` and the
  crash report setting again. The window calls it after changing the
  scheduled restart, background app updates or crash reports, and once at
  start; a D-Bus service file starts the tray if the user quit it. The
  window follows what the tray changes by watching the config folder.
- Panel icon: a StatusNotifierItem (`org.kde.StatusNotifierItem-<pid>-1`,
  Id `net.eterneon.telamon.updater`, as KStatusNotifierItem exported it) with
  a com.canonical.dbusmenu menu: Open, Check for Updates, Restart to Update
  (while staged), Cancel Scheduled Restart (while one is set), Quit. It
  registers again whenever the StatusNotifierWatcher (Plasma) comes back.
  Icon names come from the icon theme in `kdeglobals` (Papirus's update
  icons when present, else ours), looked up once at start.
- Notifications go straight to `org.freedesktop.Notifications`, through
  the framework's sender (`telamon_framework_system::notify`, a 10 s
  timeout per call), with KNotification's hints (`desktop-entry`, `x-kde-appname=telamon-updater`,
  `x-kde-eventId`), so Plasma's per-event settings in
  `telamon-updater.notifyrc` keep working; an event whose popup the user
  turned off there is not sent. Action signals count only from the server
  that showed the notification.
- Staged-update detection at idle: an inotify watch on `/run/ostree/`
  (bootc/ostree creates `/run/ostree/staged-deployment` when an update is
  staged), plus a fallback `Status()` call every 6 h. When something new is
  staged, the tray sends a notification (event `updateStaged`) with a
  "Restart to Update" action, unless the window is open, and the panel
  icon goes to NeedsAttention.
- The background download and staging is the OS's job
  (`atlasos-update-stage.timer` in the Telamon OS image runs `bootc upgrade`, or
  `rpm-ostree upgrade` on a system with local rpm-ostree changes).
  The app only shows it.

Screens:
- **Updates**:
  - Current, staged and rollback versions, each with its date (from
    `status.booted/staged/rollback.image.{version,timestamp}`).
  - A "Check for Updates" button (`CheckForUpdate`), and "Download Update"
    (`Upgrade`) when an update is found but not staged. bootc records the
    result as the `cachedUpdate` of the entry whose commit the image's ostree
    ref points to (the image pulled last; after a rollback, the rollback
    entry), so the booted entry's can be stale: `Status::available_update`
    reads the ref heads from `/ostree/repo/refs/heads/ostree/container/image`
    and picks that entry.
  - An update whose digest is in `/var/lib/atlasos/bad-image-digests`
    (written by the Telamon OS image's greenboot red.d script when an image
    fails its boot health checks for the last time and is rolled back) is
    shown as a warning, "Version X didn't start properly", with "Download
    Anyway" behind a confirmation instead of "Download Update". The
    background stager skips it too. The file is read next to the ref heads,
    without root.
  - Release notes of the new version as Markdown, from
    `release_notes_url` with `{version}` filled in (default
    `https://api.github.com/repos/EternalCoder454/AtlasOS/releases/tags/{version}`,
    using the `.body` field). The URL can be overridden in
    `/etc/telamon-updater/updater.toml`. If no release exists: "No release
    notes for this version".
  - Flatpak app updates on the same screen, with an "Update Apps" button.
  - "Download app updates in the background" (Automatic Updates section),
    **off by default**, saved per user in `telamon-updaterrc`
    (`[AppUpdates] Automatic`). The tray looks for app updates 10 minutes
    after it starts, then every 6 hours, and a minute after the switch is
    turned on, each time in a short-lived `telamon-updater --worker
    apps-round` (no Qt; it prints its result as one JSON line, dies with
    the tray, and is ended after 6 hours). A notice counts as given once
    it is on screen: one that could not be shown comes again next round. One app operation runs at a time: the worker and the window
    take `$XDG_RUNTIME_DIR/telamon-updater-apps.lock` (a round finding it
    taken tries again in 5 minutes). A round's error is kept in
    `[AppUpdates] RoundError` for the window, and `RoundAt` tells an open
    window to list the apps again. Nothing is looked up while NetworkManager
    says the connection is metered or offline. Not knowing counts as
    metered: NetworkManager there but silent or saying "unknown", installed
    but not running at the moment, the system bus not answering, or the
    8-second limit on the whole read. Only a system without NetworkManager
    installed goes ahead (the download then says whether there is a
    network). The battery is checked only before installing, not before
    looking. Off: the waiting updates are checked the same way without
    downloading or installing anything (`check_only`: each run stops before
    it would start), as is every "Check for App Updates", so rows say what
    an update asks for before the user presses "Update Apps"; then a
    notification (`appUpdatesReady`, with "Update Apps" unless something
    asks for new permissions or the check failed, whose error the page
    shows) when updates wait, once per set
    (`[AppUpdates] Notified` holds a hash of the set, cleared when nothing
    waits). On: they install by themselves through `update` with both
    options set, unless UPower says the battery is below 30% while unplugged
    (then nothing is shown). A round that waited tries again in 30 minutes;
    failed rounds back off: 30 minutes, 1, 2, 4, then every 6 hours, until
    one succeeds. A failed run gets the notification too, and its error
    shows on the Updates page until a later round works or the user acts.
    A run that fails before it could check an app's permissions installs
    nothing of it, and pressing "Update Apps" then installs it as any
    manual update would: the user chose it. If the list can't be read again
    after installing, what waited before less what was installed stands in. Apps
    held back for new permissions get it without the "Update Apps" button,
    naming what they ask for; their rows on the Updates page say "Asks for
    new permissions: ..." (at most five, the weightiest first: the
    home folder or all files and ways out of the sandbox such as
    `org.freedesktop.Flatpak` or owning a bus name, then devices and
    sockets, then the rest) until they are updated, so the
    user sees it before pressing "Update Apps". Each notice is given once:
    the key covers the waiting set, the held apps and whether it failed.
    The held notes live in memory only: after a restart the next round
    finds them again. A round, a check or an "Update Apps" press that comes
    while another app operation runs waits for it; a queued "Update Apps"
    is dropped when that round held apps back, so nothing asking for new
    permissions installs before the user saw it. If the switch can't be
    saved, it stays as it was and the page says why. "Update Apps" on a
    notification, or one pressed while another app operation runs, leaves
    out what asks for new permissions, as a background run does (the check
    behind a notice may be hours old, or have failed); what it left out
    gets a notice and its row's note. The page's "Update Apps" installs
    everything listed, as the user sees the notes there, after checking
    again: if anything now asks for more than its row showed (a version
    published since), or the check fails, nothing installs and the rows
    get the new notes for the user to press again. It asks for a password
    as usual.
  - **Firmware**, below the apps, only when fwupd is there: one row per
    device with an update (device name, "1.2.2 → 1.2.4", the vendor, the
    summary, and "Important" for urgency high or critical), with the
    release notes behind "Details" and an "Install" button per row. Pending
    installs say "Restart to finish installing" (or "Shut down to finish")
    and failed ones say what fwupd said. With nothing to install: "Firmware
    is up to date". With metadata older than 30 days, or never fetched, a
    note says when the firmware list was last updated. "Install" asks
    first ("Keep the computer plugged in, and don't unplug <device> or turn
    the computer off until it finishes"; plus "It finishes when you
    restart" for needs-reboot, "…shut down" for needs-shutdown), then shows
    fwupd's progress and its requests ("Unplug the device and plug it back
    in") in the page; the screen-edge glow is on meanwhile. After a needs-reboot
    install it offers "Restart to Update" (the same restart as for the
    system). "Check for Updates" lists the firmware again too (fwupd's
    local state; no download). The tray lists firmware with each app round
    (10 minutes after start, then every 6 hours; it needs no network: the
    metadata is fwupd's) and sends `firmwareReady` ("Firmware updates are
    available for <devices>", action "Open Telamon Updater" to the Updates
    page) once per set: `[Firmware] Notified` in `telamon-updaterrc` holds
    `notice_key`, cleared when nothing waits. No notice while the window
    is open.
  - "Restart to Update", and "Restart Later…" (pick a time today or
    tomorrow; the tray restarts then, with a notification 5 minutes
    before, and never without it: a warning that could not be shown, or
    an update it could not check, turns the restart into a "did not
    happen" notice; before acting, the tray reads the saved time again; the setting persists in `~/.config/telamon-updaterrc`; can be
    cancelled, from the window, the menu or the notification).
  - Restart goes through `org.kde.Shutdown /Shutdown logoutAndReboot` on the
    session bus, so apps can save first.
- **Go Back**: "Go Back to <rollback version> (<date>)" → `Rollback()`, then
  offers the restart. When the rollback image is in `bad-image-digests`, the
  page and the confirmation say it failed its startup checks here.
- **Channel**: stable or testing (from the booted ref's tag) →
  `SwitchChannel`, then offers the restart.
- **History**: the versions this machine has booted, newest first, from
  `history.jsonl`; below them the app updates this user installed, by hand
  or in the background, from `~/.local/state/telamon-updater/app-updates.jsonl`
  (one JSON object per line, cut to the newest 500 past 256 KB). Any app
  with home access can write there, so the file is opened without following
  symlinks and without blocking, must be a regular file of this user, and
  only its last 512 KB are read; the folder is made 0700 (and tightened if
  it is looser). Entries are cleaned like remote text when read, and lines
  over 4 KB or dated in the future are dropped. Writers (the tray and the
  window) take an exclusive `flock`, giving up after 5 seconds in all, and
  check the file wasn't replaced meanwhile; the trim, under that lock,
  writes a temporary file with a fresh name (`create_new`, 0600) and
  renames it.

## Telamon OS side (the Telamon OS repo, not here)

- Image tags: `stable` (weekly) and `testing` (daily), each version tagged
  `44.YYYYMMDD-N` (N: the build's number that day; older ones are plain
  `44.YYYYMMDD`). The image label `org.opencontainers.image.version` = the
  version, which `bootc status` shows.
- Ships `atlasos-update-stage.timer`, whose condition skips the newest image
  when it is the rollback image, a bad image or a downgrade (as above), and
  whose service tries a download that failed with a network error again
  after 15 minutes (`update-stage` exits 75; at most 4 tries in 3 hours),
  and
  autostarts the tray (`telamon-updater-tray`; an image that still names
  `telamon-updater --tray` works too),
  keeps Discover's notifier out, and installs the RPMs built by
  `packaging/build-rpm.sh` during the container build.
- Ships fwupd with `fwupd-refresh.timer` enabled (Fedora's desktop
  editions leave it off for GNOME Software and Discover), and without
  Discover's fwupd backend, so firmware has one updater.

## Old names (0.3.0 only)

Atlas Updater became Telamon Updater in 0.3.0 (version 0.2.0 was the last
with the Atlas names). Apps and the OS image move one by one, so for this
release everything another program may still use is served **under both
names**, from the same code. **Remove the old names in the next release**
(0.4.0): the code and files below are marked "legacy" or "old" where they are.

| What | New | Still served (old) |
|---|---|---|
| RPMs | `telamon-updater`, `telamon-system-helper` | `Provides: atlas-updater`, `atlas-system-helper` (and `atlas-core`), `Obsoletes: ... < 0.3.0` |
| Programs | `/usr/bin/telamon-updater`, `telamon-updater-tray`, `/usr/libexec/telamon-system-helper` | links `/usr/bin/atlas-updater`, `atlas-updater-tray`, `/usr/libexec/atlas-system-helper` (the image's autostart runs `atlas-updater-tray` / `atlas-updater --tray`; its greenboot scripts run `atlas-system-helper record-event ...`; the helper's command line does not depend on argv[0]) |
| Helper, system bus | name `net.eterneon.telamon.SystemHelper`, object `/net/eterneon/telamon/SystemHelper`, interface `net.eterneon.telamon.SystemHelper1`, errors `net.eterneon.telamon.Error.*`, polkit `net.eterneon.telamon.system.{status,check,upgrade,rollback,switch-channel}` | the same six methods and `Progress` under `net.eterneon.atlas.SystemHelper` / `/net/eterneon/atlas/SystemHelper` / `net.eterneon.atlas.SystemHelper1`, errors `net.eterneon.atlas.Error.*`, polkit `net.eterneon.atlas.system.*` (same defaults; a call that arrives through the old name is checked against the old ids, so the image's polkit rules for them keep applying). Both D-Bus policy files and both activation files ship, and `rules.d/50-telamon-system.rules` covers both id sets (wheel: yes for check and upgrade) |
| Helper's client | `helper_client` calls the new name | when the new name has no owner and no activation file (an older helper is installed), it calls the old name, decided once when it connects |
| Tray, session bus | `net.eterneon.telamon.updater.Tray`, `/net/eterneon/telamon/updater/Tray`, `Reload` | the same under `net.eterneon.atlas.updater.Tray` (both names taken; both activation files ship). A tray from before the rename still running owns the old name: the new tray then exits at once, and the window's `Reload` falls back to the old name; the next login starts the new one. The status notifier Id is `net.eterneon.telamon.updater` |
| systemd units | `telamon-system-helper.service`, `telamon-record-boot.service`, `telamon-drivers.service`, `telamon-drivers.timer`, preset `50-telamon-system-helper.preset` | the old unit names are symlinks to the new files in the same directory (aliases: units the image enabled under them keep working). The old preset file is the image's. The helper starts `telamon-drivers.service` |
| Locks | `$XDG_RUNTIME_DIR/telamon-updater-{apps,crash,firmware}.lock`, `/run/telamon-system-helper.lock` | `atlas-updater-*.lock`, `/run/atlas-system-helper.lock`: **both** are taken, the new one first and then the old, and given up together, so old and new programs (the Store) exclude each other. The helper's lock still names its holder in both files |
| Settings and state | `~/.config/telamon-updaterrc`, `~/.local/state/telamon-updater/` (`app-updates.jsonl`), `~/.cache/telamon-updater/`, `/etc/telamon-updater/updater.toml` | moved once (below); `/etc/atlas-updater/updater.toml` is read when the new file is absent |
| Developer variables | `TELAMON_UPDATER_{FIXTURES,FIXTURE_HOLD,PAGE,GLOW_DEMO,FIRMWARE_FILES}` | `ATLAS_UPDATER_*` (the new name wins) |
| `build-rpm.sh` | `TELAMON_LOCAL_RPMS`, `TELAMON_BUILD_CACHE` | `ATLAS_LOCAL_RPMS`, `ATLAS_BUILD_CACHE` |

Not carried over, on purpose: the desktop files `net.eterneon.atlas.updater.desktop`
and `...-tray.desktop` (the OS image's autostart or masking by those names is
the image's to change), the icon names `net.eterneon.atlas.updater*`, the
journal identifiers (`journalctl -t atlas-updater` finds only old entries; use
`-t telamon-updater`), the single-instance name of the window
(`net.eterneon.telamon.updater`: the window goes away), the old preset file and
`/etc/dnf/protected.d/atlas.conf` (the old package's). The framework 2.0.0
moves its own files itself (`telamon-updaterrc`'s `[Atlas]` group is still
read, `~/.config/atlas-updater.notifyrc` choices, the crash-reporting settings
and state): nothing here repeats that.

**What stays as it is** because it is the image's or the registry's:
`ghcr.io/eternalcoder454/atlasos` and the `EternalCoder454/AtlasOS` GitHub
project, `/var/lib/atlasos/bad-image-digests`, `/var/lib/atlas-core`
(this directory), `atlasos-update-stage.timer`, the `atlasos_version` field
and the "Atlas app" category of crash reports, and the framework's repository
name `atlas-framework` until it is renamed.

**Moving the user's files** (`telamon_updater_base::migrate`, run by the tray,
the window and the worker when they start, and by the first use of the
settings or the history): `~/.config/atlas-updaterrc` to `telamon-updaterrc`,
`~/.local/state/atlas-updater/` to `telamon-updater/` and
`~/.cache/atlas-updater/` to `telamon-updater/`. Each is a **move** (one
atomic `rename`, never replacing anything), one way: nothing is written to
the old name afterwards. It happens only when the new name is not there; if
both folders exist the old one is merged file by file (a file the new folder
already has stays in the old one, nothing is overwritten or deleted). A link
is never followed or moved, and only what belongs to the user is touched. A
folder keeps its mode (0700 for the state folder).

## System app

telamon-system-helper and telamon-updater are required parts of Telamon OS, not optional apps.
They come with the image in the read-only `/usr`, which Discover and dnf
can't remove. `/etc/dnf/protected.d/telamon-updater.conf` (shipped by telamon-system-helper)
protects them from dnf in mutable contexts, and the image build fails without
them. Root can still `rpm-ostree override remove` them; that's the limit on
an open system.

## Privacy and crash reports

Crash reports are the only telemetry. `telamon_framework_system::crash` (opt-in, off by
default; when off nothing is collected or written):

- **Settings.** Per user, `~/.config/telamon/crash-reporting.toml`,
  `enabled = false` (`crash::Settings`; the framework still reads the
  `atlas/` files of before 2.0.0 until the new ones exist). **Endpoint:** a
  GlitchTip (Sentry compatible) DSN, `dsn = ""` in
  `/etc/telamon/crash-reporting.toml`, default shipped in
  `/usr/share/telamon/crash-reporting.toml`:
  `https://atlasos@telamon.eterneon.net/crash/1`, the Telamon OS relay (store
  URL `https://telamon.eterneon.net/crash/api/1/store/`). An empty `dsn` in
  `/etc` turns sending off; with no DSN `send()` fails with "no endpoint
  configured".
- **Sources.** Telamon app Rust panics (`crash::install`, `record_fatal` for Qt
  fatal messages); systemd-coredump entries of the user's own processes
  (`collect_coredumps`: journal fields COREDUMP_EXE/COMM/SIGNAL_NAME/
  TIMESTAMP/PACKAGE_NAME/PACKAGE_VERSION and the stack trace in MESSAGE only,
  never the core file, command line, environment or working directory);
  update and rollback events (`collect_events`) from
  `/var/lib/atlas-core/events.jsonl`, which the helper writes
  (`update-staged`, `update-failed`, `rollback-requested`, `rollback-failed`,
  `channel-switched`, `channel-switch-failed`; `record-boot` adds
  `update-applied`, `rollback-applied`, `automatic-rollback`; greenboot
  scripts call `telamon-system-helper record-event health-check-failed|
  health-check-passed`). Only failures become reports (`REPORTED_EVENTS`:
  `update-failed`, `rollback-failed`, `channel-switch-failed`,
  `automatic-rollback`, `health-check-failed`); other events are skipped
  and the marker moves past them. Pending reports of other helper events
  from older versions are deleted when pending reports are loaded.
- **Collected, only this.** Telamon OS version, channel and previous version;
  app name, version and category (Plasma, KWin, Atlas app, other); the stack
  trace; kernel; GPU model (pci.ids) and driver; uptime; CPU model, RAM total
  and use; a rotating random ID (new every 30 days; never `/etc/machine-id`),
  a timestamp and the report type. Never: core dumps, usernames, hostname,
  MAC/IP addresses, serials, installed apps, file contents, command lines,
  environment, working directory.
- **Scrubbing.** Case-insensitive. `/home/<name>` and `/var/home/<name>`
  become `.../USER`; the user name, full name (GECOS), `$HOME` and the host
  names (kernel, static, pretty; first label too) become `USER`/`HOST`
  (names of 4+ characters anywhere, shorter ones only at word boundaries);
  MAC and IP addresses (IPv6 with zones, MAC-named interfaces), 32-hex and
  UUID tokens become placeholders. Messages and stack traces also lose any
  path into user data (`/home`, `/run/user`, `/run/media`, `/tmp`, `file://`,
  `~/`, ...). Stored traces keep only frame lines and thread headers.
- **events.jsonl is system-wide.** It is world-readable and has no user, so
  the helper scrubs the `error` text fully (same scrubber, paths and
  addresses included) before writing, and `collect_events` scrubs every
  string again when it copies one into a report.
- **First opt-in.** Turning reporting on (or finding no marker) sets the
  coredump and event markers to "now": nothing from before the opt-in is ever
  queued. The journal is read with `--all`, `--output-fields` limited to the
  fields above, and `--since` from the marker.
- **Limits.** Panic reports: at most 5 an hour per app, the same top frames
  once; the hook runs the previous hook first and is guarded against
  reentrancy. Sent reports older than 90 days (or with a future mtime) are
  pruned on every send, collect, install and when reporting is turned off.
- **Sending.** Refused when reporting is off. `https` only (`http` only for
  localhost, 127.0.0.1, ::1). `/usr/bin/curl` runs with a cleared
  environment, `-q`, `--proto`, `--noproxy '*'`, no redirects, and the body
  from a 0600 temp file.
- **Consent.** Reports wait in `$XDG_STATE_HOME/telamon/crash-reports/pending/`.
  The app shows `Report::payload()` (the exact Sentry event JSON that `send()`
  posts to `{dsn host}/api/{project}/store/`) and only then calls `send()`,
  which moves the report to `sent/` (kept 90 days). The relay posts the
  report as a public issue in github.com/EternalCoder454/AtlasOS and
  answers `{"id", "url"}`; `url` is kept (`Report::issue_url`) only if it
  starts with `https://github.com/EternalCoder454/AtlasOS/issues/`, and the
  Sent Reports list shows it as "View on GitHub". The Send screens say
  before sending that the report becomes public. "Don't Send" calls
  `discard()`. "Report on GitHub" opens `github_issue_url()`.
