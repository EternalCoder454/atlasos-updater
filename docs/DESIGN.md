# Atlas Updater and atlas-core: design

AtlasOS is a Fedora Kinoite 44 bootc image (`ghcr.io/eternalcoder454/atlasos`).
This repo holds three things:

- **atlas-core**: the shared Rust library and the privileged system helper that
  Atlas apps reuse (Atlas Store later depends on it through git).
- **atlas-updater**: the Atlas Updater app (`net.eterneon.atlas.updater`).
- **template/**: the starting point for new Atlas apps.

Stack: Rust + Qt 6.11 + Kirigami 6.30 through CXX-Qt. Everything builds and runs
on Fedora 44 (Qt 6.11.2, KF6 6.30, bootc 1.16.13, flatpak 1.18.2, polkit 127,
Rust 1.98).

## Layout

```
Cargo.toml                    workspace: crates/atlas-core, apps/atlas-updater
crates/atlas-core/            lib + bin `atlas-system-helper`
  src/bootc.rs                serde types for `bootc status --json`, ref/tag helpers
  src/helper_client.rs        zbus proxy for the system helper (what apps call)
  src/flatpak.rs              libflatpak wrapper (cargo feature "flatpak")
  src/history.rs              reads /var/lib/atlas-core/history.jsonl
  src/crash.rs                opt-in crash reports (see Privacy and crash reports)
  src/helper/                 helper logic: bootc runner, polkit, events.jsonl
  src/bin/atlas-system-helper.rs  (or src/helper/*) the D-Bus system service
  data/                       D-Bus, polkit, systemd files for the helper
apps/atlas-updater/           the app (CMake + Corrosion, or cxx-qt-build)
  qml/                        Kirigami UI, compiled ahead of time (qmlcachegen)
  data/                       .desktop, autostart .desktop, .notifyrc, metainfo, icon
template/                     minimal Atlas app skeleton (Kirigami window + atlas-core)
packaging/atlas.spec          one spec, subpackages `atlas-core` and `atlas-updater`
packaging/build-rpm.sh        builds the RPMs inside fedora:44: build-rpm.sh <out dir>
```

## System helper (atlas-core)

bootc has no D-Bus API and needs root, so apps go through a small system
service. It is D-Bus activated, so nothing runs at idle; it exits after 60 s
without calls.

- Binary: `/usr/libexec/atlas-system-helper`
- Bus name: `net.eterneon.atlas.SystemHelper` (system bus)
- Object: `/net/eterneon/atlas/SystemHelper`
- Interface: `net.eterneon.atlas.SystemHelper1`

| Method | Returns | Runs | polkit action | Default |
|---|---|---|---|---|
| `Status()` | `s` JSON | `bootc status --json` | `net.eterneon.atlas.system.status` | yes (any) |
| `CheckForUpdate()` | `s` JSON | `bootc upgrade --check`, then `bootc status --json` | `net.eterneon.atlas.system.check` | yes (active) |
| `Upgrade()` | `s` JSON | `bootc upgrade` (stages only; never `--apply`) | `net.eterneon.atlas.system.upgrade` | auth_admin_keep; wheel: yes (rules.d) |
| `Rollback()` | `s` JSON | `bootc rollback` | `net.eterneon.atlas.system.rollback` | auth_admin_keep |
| `SwitchChannel(s channel)` | `s` JSON | `bootc switch <ref with tag = channel>` | `net.eterneon.atlas.system.switch-channel` | auth_admin_keep |

The returned JSON string is `bootc status --json` after the action.

Rules:
- **Only these five methods.** No method takes a command, path, image ref or
  argument list. `channel` must be exactly `stable` or `testing`. The new ref
  is the booted image's own ref with only the tag replaced
  (`ghcr.io/eternalcoder454/atlasos:stable` → `:testing`; an `oci:` or
  local-registry ref in a test VM works the same way). The transport stays
  the same. Anything else is rejected with `net.eterneon.atlas.Error.InvalidArgument`.
- bootc runs by absolute path (`/usr/bin/bootc`) with a fixed argv and a clean
  environment (no inherited PATH or LD_*).
- polkit: `CheckAuthorization` on `org.freedesktop.PolicyKit1.Authority`, with
  subject `system-bus-name` (the caller's unique name) and the
  AllowUserInteraction flag. A failed check gives `net.eterneon.atlas.Error.NotAuthorized`.
- One operation at a time. A second call while one runs gets `net.eterneon.atlas.Error.Busy`.
- bootc failures give `net.eterneon.atlas.Error.Failed`, with bootc's stderr
  (last 4 KB) as the message.
- Calls take minutes (`Upgrade` downloads the image), so clients use no method
  timeout.
- D-Bus policy: anyone may call the interface (polkit decides). Only root may own the name.

**History.** `atlas-system-helper record-boot` (CLI mode, run as root by
`atlas-record-boot.service`, a oneshot at boot) appends one line to
`/var/lib/atlas-core/history.jsonl` when the booted image digest differs from
the last line:
`{"version":"44.20261002","digest":"sha256:…","image":"ghcr.io/…:stable","timestamp":"<image build time>","first_booted":"<RFC3339 now>"}`.
The file is world-readable (0644), and the directory comes from
`StateDirectory=atlas-core`.

## Flatpak wrapper (atlas-core, feature `flatpak`)

This uses libflatpak through the `libflatpak` crate (gtk-rs style). System
installs go through flatpak's own system helper, which asks polkit itself, so
our helper is not involved.

```rust
pub struct AppUpdate { pub id: String, pub name: String, pub branch: String,
                       pub installation: Installation /* System | User */,
                       pub download_size: u64, pub current_version: Option<String>,
                       pub new_version: Option<String> }
pub fn list_updates() -> Result<Vec<AppUpdate>>;   // system + user, apps and runtimes
pub fn update_all(progress: impl FnMut(Progress)) -> Result<()>;  // one transaction per installation
```

`list_updates` reads cached metadata unless asked to refresh: it takes
`refresh: bool`, which updates appstream and summary first.

## Atlas Updater app

- Binary and package: `atlas-updater`. App ID: `net.eterneon.atlas.updater`.
- `atlas-updater`: opens the window. `atlas-updater --tray`: autostarts at
  login, puts a KStatusNotifierItem in the tray, and loads no QML until the
  window opens. Closing the window frees the QML engine. A second launch
  raises the existing instance (single instance through D-Bus on the session
  bus, `net.eterneon.atlas.updater`).
- Staged-update detection at idle: an inotify watch on `/run/ostree/`
  (bootc/ostree creates `/run/ostree/staged-deployment` when an update is
  staged), plus a fallback `Status()` call every 6 h. When something new is
  staged, it sends a KNotification (event `updateStaged` in
  `atlas-updater.notifyrc`) with a "Restart to update" action, and the tray
  icon goes to NeedsAttention.
- The background download and staging is the OS's job
  (`atlasos-update-stage.timer` in the AtlasOS image runs `bootc upgrade`).
  The app only shows it.

Screens:
- **Updates**:
  - Current, staged and rollback versions, each with its date (from
    `status.booted/staged/rollback.image.{version,timestamp}`).
  - A "Check for updates" button (`CheckForUpdate`; the result is in
    `status.booted.cachedUpdate`), and "Download update" (`Upgrade`) when an
    update is found but not staged.
  - Release notes of the new version as Markdown, from
    `release_notes_url` with `{version}` filled in (default
    `https://api.github.com/repos/EternalCoder454/AtlasOS/releases/tags/{version}`,
    using the `.body` field). The URL can be overridden in
    `/etc/atlas-updater/updater.toml`. If no release exists: "No release
    notes for this version".
  - Flatpak app updates on the same screen, with an "Update apps" button.
  - "Restart to update", and "Restart later…" (pick a time today or
    tomorrow; the tray process restarts then, with a notification 5 minutes
    before; the setting persists in `~/.config/atlas-updaterrc`; can be
    cancelled).
  - Restart goes through `org.kde.Shutdown /Shutdown logoutAndReboot` on the
    session bus, so apps can save first.
- **Go back**: "Go back to <rollback version> (<date>)" → `Rollback()`, then
  offers the restart.
- **Channel**: stable or testing (from the booted ref's tag) →
  `SwitchChannel`, then offers the restart.
- **History**: the versions this machine has booted, newest first, from
  `history.jsonl`.

## AtlasOS side (the AtlasOS repo, not here)

- Image tags: `stable` (weekly) and `testing` (daily), each version tagged
  `44.YYYYMMDD`. The image label `org.opencontainers.image.version` = the
  version, which `bootc status` shows.
- Ships `atlasos-update-stage.timer`, autostarts `atlas-updater --tray`,
  keeps Discover's notifier out, and installs the RPMs built by
  `packaging/build-rpm.sh` during the container build.

## System app

atlas-core and atlas-updater are required parts of AtlasOS, not optional apps.
They come with the image in the read-only `/usr`, which Discover and dnf
can't remove. `/etc/dnf/protected.d/atlas.conf` (shipped by atlas-core)
protects them from dnf in mutable contexts, and the image build fails without
them. Root can still `rpm-ostree override remove` them; that's the limit on
an open system.

## Privacy and crash reports

Crash reports are the only telemetry. `atlas_core::crash` (opt-in, off by
default; when off nothing is collected or written):

- **Settings.** Per user, `~/.config/atlas/crash-reporting.toml`,
  `enabled = false` (`crash::Settings`). **Endpoint:** a GlitchTip (Sentry
  compatible) DSN, `dsn = ""` in `/etc/atlas/crash-reporting.toml`, default
  shipped in `/usr/share/atlas/crash-reporting.toml`. With no DSN `send()`
  fails with "no endpoint configured".
- **Sources.** Atlas app Rust panics (`crash::install`, `record_fatal` for Qt
  fatal messages); systemd-coredump entries of the user's own processes
  (`collect_coredumps`: journal fields COREDUMP_EXE/COMM/SIGNAL_NAME/
  TIMESTAMP/PACKAGE_NAME/PACKAGE_VERSION and the stack trace in MESSAGE only,
  never the core file, command line, environment or working directory);
  update and rollback events (`collect_events`) from
  `/var/lib/atlas-core/events.jsonl`, which the helper writes
  (`update-staged`, `update-failed`, `rollback-requested`, `rollback-failed`,
  `channel-switched`, `channel-switch-failed`; `record-boot` adds
  `update-applied`, `rollback-applied`, `automatic-rollback`; greenboot
  scripts call `atlas-system-helper record-event health-check-failed|
  health-check-passed`).
- **Collected, only this.** AtlasOS version, channel and previous version;
  app name, version and category (Plasma, KWin, Atlas app, other); the stack
  trace; kernel; GPU model (pci.ids) and driver; uptime; CPU model, RAM total
  and use; a rotating random ID (new every 30 days; never `/etc/machine-id`),
  a timestamp and the report type. Never: core dumps, usernames, hostname,
  MAC/IP addresses, serials, installed apps, file contents, command lines,
  environment, working directory.
- **Scrubbing.** `/home/<name>` and `/var/home/<name>` become `.../USER`, the
  username `USER`, the hostname `HOST`; MAC and IP addresses are removed;
  panic messages also lose paths into user data.
- **Consent.** Reports wait in `$XDG_STATE_HOME/atlas/crash-reports/pending/`.
  The app shows `Report::payload()` (the exact Sentry event JSON that `send()`
  posts to `{dsn host}/api/{project}/store/`) and only then calls `send()`,
  which moves the report to `sent/` (kept 90 days). "Don't send" calls
  `discard()`. "Report on GitHub" opens `github_issue_url()`.
