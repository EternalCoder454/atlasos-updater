# Telamon Updater: security

The Secure phase of this repository: what the privileged parts are, who may
ask them for what, what an attacker on the machine can and cannot do, what was
fixed, and what is left. The design of the pieces is in [DESIGN.md](DESIGN.md);
this file is about their trust boundaries. The OS image's own scripts and units
(the background stager, the PIN verifier, polkit, PAM, kconf_update) are in the
image repository's `docs/SECURITY.md`.

## What is privileged

| Piece | Runs as | How it starts | Reached by |
|---|---|---|---|
| `telamon-system-helper` (`telamon-system-helper.service`) | root | D-Bus activation, exits after 60 s idle | the system bus, names `net.eterneon.telamon.SystemHelper` and (until the release after 0.3.0) `net.eterneon.atlas.SystemHelper` |
| `telamon-system-helper drivers` (`telamon-drivers.service`, `.timer`) | root | timer, or the helper asks systemd to start it after an update check | nothing: no arguments, no input |
| `telamon-system-helper record-boot` (`telamon-record-boot.service`) | root | at boot | nothing |
| `telamon-system-helper record-event <name>` | root | greenboot scripts of the image | a fixed list of two names |
| `telamon-updater-tray` | the user | session autostart | the session bus: `Reload`, `SetWorking` |
| `telamon-updater --worker` | the user | the tray | the tray (JSON on stdout) |
| `telamon-updater-glow` | the user | the tray | the tray |

Everything else (Flatpak app updates, firmware through fwupd, release notes,
crash reports) runs as the user and talks to root only through fwupd's and
flatpak's own polkit-guarded services.

## Threat model

**Attacker.** An unprivileged local user, or a process running as one user (a
Flatpak app with `--filesystem=home` is the realistic one), or a remote party
who controls a registry response, a release-notes page or a firmware
repository. Not in scope: someone who is already root, or who holds the OS
image signing key; a user who is an administrator and chooses to run
`bootc switch` themselves.

**Assets.** The OS that boots next (what is staged); the root helper's
integrity and availability; other users' data; the user's consent for
restarts, firmware flashes and crash reports.

### Who may call what (system bus)

Every method is checked by polkit for the caller's own bus name
(`system-bus-name` subject, so the caller cannot be another process by pid
reuse). The D-Bus policy lets anyone *send*; only root may *own* the names.

| Method | polkit action (`net.eterneon.telamon.system.*`) | no session / remote | inactive local | active local | argument |
|---|---|---|---|---|---|
| `Status` | `status` | yes | yes | yes | none |
| `CheckForUpdate` | `check` | no | no | yes | none |
| `Upgrade` | `upgrade` | admin | admin | admin (kept); wheel in an active local session: no password (rule) | none |
| `Rollback`, `CancelRollback` | `rollback` | admin | admin | admin (kept) | none |
| `SwitchChannel` | `switch-channel` | admin | admin | admin (kept) | `stable` or `testing`, nothing else |

The old identity (`net.eterneon.atlas.*`) has the same defaults; a test keeps
the two sets identical. `Progress` is a read-only property (JSON of the
operation's progress, no secrets).

Why each default:

- `Status` only shows what `bootc status` shows; it is rate-limited (one
  `bootc status` at a time, shared for 2 s, at most 8 waiting callers).
- `CheckForUpdate` asks the registry for a manifest; a user at the machine may
  (Settings shows the update to standard users), a remote session may not. It
  can also make the helper ask systemd to start `telamon-drivers.service`,
  which switches to the driver image the *hardware* needs (decided from sysfs,
  not from the caller) and only when the booted origin is signature-checked.
  The caller chooses nothing but the moment.
- `Upgrade` only stages the image the system already follows, never reboots,
  and refuses an image older than the booted or staged one before downloading
  it. The one rule lets an administrator at the machine do that without
  typing a password; it does not cover `Rollback` or `SwitchChannel`.
- Nothing the caller sends reaches a command line except the channel, which is
  parsed into an enum (`stable`/`testing`) before it is used and refused above
  16 bytes without being echoed. No method takes a path, an image reference, a
  command or an argv (a test reads the interface and fails if one appears).

### Image reference tampering (confused deputy)

The helper never takes an image from a caller. A switch is built from the
booted reference: same transport, same repository, the tag replaced by the
channel, the signature setting carried over (or upgraded to
`--enforce-container-sigpolicy` where `/etc/containers/policy.json` demands a
signature anyway). The drivers path builds its target from a fixed table in
`drivers.rs` (`ghcr.io/eternalcoder454/` plus a name from the table) and the
booted tag, and only if the pull will be signature-checked. A reference that
starts with `-` or holds whitespace or control characters is refused. A
digest in the booted reference is dropped, not followed.

Signature policy lives in the image (`/etc/containers/policy.json`, cosign key
`/etc/pki/containers/telamon.pub`, tests in the image repository's
`tests/signing`): every pull of `ghcr.io/eternalcoder454/telamonos`,
`telamonos-nvidia` and the old `atlasos` names needs a signature from the
project key, whatever the origin says. Pointing the system at another registry
needs root (`bootc switch`); nothing in this repository does it. A signature
does not prove freshness, so the helper and the stager also refuse an older
build of the same image (a replayed tag), before the download when the
registry can be asked and again on what was staged (taken out with
`rpm-ostree cleanup -p`).

### The helper process

- Runs bootc, rpm-ostree and skopeo by absolute path, with a cleared
  environment (`PATH`, `LANG` only), in their own process group, no stdin, a
  wall-clock limit, and capped output (4 MiB; the stderr tail kept for errors
  is 4 KiB). The D-Bus caller's text never becomes an argument.
- One changing operation at a time (an in-process flag and a root-only lock
  file in `/run`, opened without following links), shared with the image's
  own scripts under both lock names.
- Files it writes (`/var/lib/atlas-core/*`: the drivers state, the saved
  update check here; the history and events files through the framework) are
  world-readable (the tray and Settings read them) and hold versions, digests
  and the tail of bootc errors, no secrets. The two written here go to a new
  file opened `O_NOFOLLOW | O_EXCL`, are synced and renamed, and the directory
  is synced; reads are size-capped, and the framework's reader refuses
  anything but a regular file of at most 16 MiB.
- Sandboxed as far as bootc allows; see "Units".

### Release notes, firmware, Flatpak, crash reports (the user side)

- **Release notes** are fetched over https by default (an administrator may
  configure `http://` or `file://` in `/etc/telamon-updater/updater.toml`; a
  `file://` read is not size-capped), redirects stay on https, at most 2 MiB
  for a download, 15 s, rendered through an allow-list (raw HTML shown as text,
  images dropped, only https links, a link whose text names another host gets
  that host appended). The cache file under `~/.cache` is read as a regular
  file of at most 8 MiB, written through a new `O_EXCL | O_NOFOLLOW` file.
- **Firmware** goes through fwupd, which asks polkit itself. The client calls
  `GetDevices`, `GetUpgrades`, `GetRemotes`, `SetFeatureFlags` and
  `Install(id, fd, {})` only (no reinstall, downgrade or branch options);
  device ids are checked against the live listing; a release without fwupd's
  trusted flag is refused; the download is https, at most 256 MiB, hashed in a
  sealed memfd that is then handed to fwupd (the hashed bytes are the bytes
  installed).
- **Apps** are updated through the framework's libflatpak wrapper; no command
  is built from a string. Background updates never ask questions, and an
  update that widens permissions is held back for the user.
- **Crash reports** are in the framework (`telamon-framework-system::crash`,
  see its `docs/reference`). Opt-in, off by default, the exact payload shown
  before every send, sent with `curl` over https to the project's relay. The
  collector trusts a systemd-coredump journal entry only if journald says it
  came from a `systemd-coredump@` unit and from the user's own uid, so another
  user's process cannot forge a report that is shown to or sent for this user.
- **Restarts** go through `org.kde.Shutdown`; a scheduled restart needs its
  warning notification to have been shown and is postponed during a firmware
  flash.

## Units

Exposure is `systemd-analyze security --offline=yes` on the shipped unit
(lower is better; checked in a Telamon OS container).

| Unit | Exposure | Kept off, and why |
|---|---|---|
| `telamon-system-helper.service` | 6.7 | `ProtectSystem`, `PrivateMounts`, `PrivateDevices`, `RestrictNamespaces`, `RestrictSUIDSGID`, `MemoryDenyWriteExecute`, `NoNewPrivileges`: bootc and ostree need mount namespaces, writable `/sysroot` `/ostree` `/etc`, setuid checkouts and SELinux transitions. `CAP_SYS_PTRACE` stays because bootc joins `/proc/1/ns/mnt` |
| `telamon-drivers.service` | 6.7 | same |
| `telamon-record-boot.service` | 6.0 | same; it has no network (`PrivateNetwork`, `AF_UNIX` only) |

All three drop the capabilities nothing here uses (`CAP_SYS_MODULE`,
`CAP_SYS_BOOT`, `CAP_SYS_RAWIO`, `CAP_SYS_TIME`, `CAP_NET_ADMIN`,
`CAP_NET_RAW`, `CAP_BPF`, `CAP_PERFMON`, ...), hide `/home`, give the service
its own `/tmp`, and fix the architecture. `tests/data_files.rs` fails if any
of that is removed.

Further options (`ProtectKernelTunables`, `SystemCallFilter=~@reboot @swap
@module`, `ProtectProc`) were not added: each could stop bootc or ostree in a
way only a run in the test VM shows, and the capabilities that those
syscalls need are already gone. They are a VM exercise, not a guess.

## Findings of this phase

| # | Issue | Severity | Fix | Test |
|---|---|---|---|---|
| U1 | `SwitchChannel` validated its argument before polkit and echoed the whole string in the error: an unauthorized caller could make the root helper copy and return a message of up to 128 MB | low | refused above 16 bytes, fixed text | `an_oversized_channel_is_refused_and_not_echoed_back` |
| U2 | The progress parser added registry-reported layer sizes with `+=` (a panic in a debug build, wrong numbers in release) | info | `saturating_add` | `rpm_ostree_sizes_that_overflow_do_not_panic` |
| U3 | `~/.cache/telamon-updater/releases.json` was read without a size cap or link check, and written through a fixed temp name that followed links | low | `O_NOFOLLOW`, regular file, 8 MiB cap; unique `O_EXCL` 0600 temp file | `the_cache_is_never_a_link_nor_huge_and_a_write_leaves_only_the_cache` |
| U4 | The tray honoured `TELAMON_SETTINGS_BIN` and `TELAMON_UPDATER_GLOW_BIN` in release builds (`telamon-updater` already did not) | info | debug builds only | the tray's bus tests (debug) still use them |
| U5 | No CI at all; nothing checked advisories, licences or sources | medium (process) | `.github/workflows/ci.yml`: fmt, clippy `-D warnings`, tests with D-Bus, `cargo audit`, `cargo deny` (weekly too); `deny.toml` | the workflow |
| U6 | The shipped polkit, D-Bus and unit files were only tested for parity between the two identities | process | tests for the defaults, the rule, root-only ownership, the method list and argument shapes, and the unit floor | `tests/data_files.rs` (5 new tests) |
| F-1..F-11 | Crash-report scrubber gaps found in the framework audit (key names, bearer tokens, long hex, paths outside home, name words, exact uptime, control characters in panic messages, unbounded reports and events) | low | framework PR "Secure phase: crash reports" | its tests |

Checked and fine: the polkit subject, the six-method interface, the parse of
every enum argument, bootc/skopeo/rpm-ostree invocation, the lock files, the
state-file writes, the downgrade refusal, the drivers decision, fwupd's call
set and trust flags, the release-notes renderer, the Flatpak calls, every
`unsafe` outside the helper (including flock, memfd seals, fcntl, prctl, alarm,
kill, setlocale and the time and locale calls), and the
absence of `sh -c` anywhere.

## Accepted, and what is left

| Item | Why it stays | What would close it |
|---|---|---|
| The tray's dbusmenu `Event` is not tied to the shell: a session-bus peer with access to the tray's name can click "Restart" | a same-user process can already call `org.kde.Shutdown`; only a sandboxed app with a broad `--talk-name` gains | accept only from the StatusNotifierWatcher's owner (needs a Plasma test) |
| `ScheduledAt` in `~/.config/telamon-updaterrc` is unauthenticated | the user is warned at login with a Cancel button; writing it needs `~/.config` access | confirm a time the tray did not set |
| A firmware lock that cannot be read counts as "no flash" | `XDG_RUNTIME_DIR` unset means no session bus either | postpone on error |
| The https filter of firmware URLs checks literal IPs and `localhost` only | the URL comes from signed LVFS metadata or a root-added remote | a resolver that refuses private addresses |
| `ureq` uses the bundled `webpki-roots`, not the system store | admin-installed CAs are ignored; the list ages with the package | `rustls-platform-verifier` |
| An unauthorized caller can still make the bus deliver (and zbus parse) a message of up to 128 MB before the helper sees the argument | the cap stops the echo and the copy, not the receive | lower `max_message_size` in the D-Bus policy |
| `check` is allowed to any active local user | Settings must show updates to standard users | a cached result per boot |
| The helper's units are at 6.0-6.7 | see "Units" | VM-verified sandboxing options |
| Firmware installs in Settings must hold `lock::FIRMWARE` for the tray's restart gate to work | lives in Telamon Settings | check there |
| Attestations (SBOM) are signed in CI but no client enforces them | they describe, they do not gate | `cosign verify-attestation` in the image repo's `scripts/verify-image.sh` (already there) for manual checks |

## Supply chain

`Cargo.lock` is committed and built with `--locked`. The only git dependency
is the Telamon framework, pinned by tag (and by commit in the lock). CI runs
`cargo audit` (RustSec) and `cargo deny` (`deny.toml`: licences, banned crates
such as OpenSSL and native-tls, only crates.io and the framework as sources,
yanked crates denied) on every push and weekly.

## Reporting

Private vulnerability reporting is not enabled on this repository yet; until it
is, contact the maintainer (EternalCoder454 on GitHub) directly rather than
opening a public issue for a way to gain root.
