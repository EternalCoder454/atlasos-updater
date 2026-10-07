# Telamon Updater and its system helper

Rust (zbus) + one small Qt 6.11 / QML program for Telamon OS (it was AtlasOS), a
Fedora Kinoite 44 bootc image (repo `~/Documents/AtlasOS`, not renamed yet).
Telamon Updater is the **background part** of updating: it has no window. The
window (update, go back, switch channel, history, crash report review) is the
Updates page of Telamon Settings (the `telamon-settings` repo, its
`docs/DESIGN.md` "Updates"). Read `docs/DESIGN.md` first: it fixes the layout,
the system helper's D-Bus API, the tray's session interface, the glow and the
telamon-update-engine API. Change it only together with the code that
implements the change.

## What is where

- `crates/telamon-update-engine`: the root system helper (`telamon-system-helper`)
  and its D-Bus client.
- `crates/telamon-updater-base`: no Qt, no libflatpak; the tray links it:
  settings (rc), ops, schedule, restart, locks, fwupd client, the tray's session
  names and client calls (`tray::call_reload`, `tray::call_set_working`).
- `crates/telamon-updater-core`: the window's Qt-free code that Telamon Settings'
  Updates page uses (`apphistory, apps, changelog, firmware, notes, power,
  worker`, plus re-exports of `base, engine, config, errors, lock, notify, ops,
  rc, restart, schedule, tray, view`). Its public API is Settings' contract:
  don't change it without telling the Settings side. The `TELAMON_UPDATER_FIXTURES`
  states live in `crates/telamon-updater-core/fixtures-states`.
- `apps/telamon-updater-tray`: the resident tray (panel icon, schedule,
  notifications). It opens `telamon-settings` (`src/open.rs`: the one mapping of
  what opens which page), serves `Reload` and `SetWorking` on the session bus, and
  supervises the glow (`src/working.rs` state machine, `src/glow.rs` process,
  `src/bus.rs` what it follows on the buses).
- `apps/telamon-updater`: `telamon-updater`: `--worker` for the tray, `--tray`,
  everything else hands over to `telamon-settings`. Also the `data/` of the
  package (hidden `.desktop`, autostart, notifyrc, metainfo, icons).
- `apps/telamon-updater-glow`: `telamon-updater-glow`, Qt/QML, draws the
  screen-edge glow only. The only CMake project.

## Hard rules

- **Build and test inside a `registry.fedoraproject.org/fedora:44` container**,
  never on the host: the host lacks the Qt/KF6/flatpak -devel packages and
  sudo. Mount the repo at `/src` (its host path contains a space, so quote it).
  Keep caches in named podman volumes so rebuilds are fast:
  `-v telamon-cargo:/root/.cargo/registry -v telamon-dnf:/var/cache/libdnf5`.
  Use a separate target dir per agent or task (`CARGO_TARGET_DIR=/src/target/<name>`).
- **Never run the glow or anything with a GUI on the user's display.** Smoke
  tests inside the container: `xvfb-run -a -s "-screen 0 1280x800x24"` (the glow
  on X11 is four tool windows per screen; `QT_QPA_PLATFORM=offscreen` draws
  nothing visible). Tests that need buses use private `dbus-daemon`s.
  Real end-to-end tests happen in the Telamon OS test VM, which the lead runs.
- **Never run bootc, flatpak transactions or the system helper against the
  host.** Unit-test the logic (argument validation, ref/tag rewriting, JSON
  parsing, history) with fixtures. The helper's D-Bus and polkit paths are
  tested in the VM.
- **The tray never activates the helper.** It follows the helper's `Progress` with
  a match rule and `Properties.Get` to the helper's unique name when it is
  already on the bus; no proxy, no method call. (The scheduled status poll is
  the existing exception.)
- **Telamon.Ui is not here.** It lives in the Telamon framework
  (`EternalCoder454/atlas-framework`, `~/Documents/Atlas Framework`) and the
  glow builds against the installed module (telamon-ui), so the build container
  needs its RPMs: build them there (`packaging/build-rpm.sh`), then pass
  `TELAMON_LOCAL_RPMS=<their dir>` (the old `ATLAS_LOCAL_RPMS` works too) to
  this repo's `build-rpm.sh`, or `dnf install` them before a CMake build.
- The system helper accepts only the six methods in DESIGN.md. Never add a
  method that takes a command, path, image ref or argv. `SetWorking` takes one
  boolean and nothing else.
- Commit only the paths you own (`git commit -- <paths>`). Other agents may be
  committing in this repo at the same time; retry if `index.lock` exists.
  Don't push.
- Until the release after 0.3.0 everything is also served under its Atlas
  names (binaries, D-Bus names, polkit actions, units, lock files, files):
  DESIGN.md, "Old names". A change to one identity is a change to both, and
  the tests for the old one must stay until it is removed. (`SetWorking` is
  new-name only.)
- Licence: MIT. App ID `net.eterneon.telamon.updater` (`net.eterneon.atlas.updater`
  until 0.3.0). Git identity is set in the repo.

## Commands

| Task | Command (inside the container, in /src) |
|---|---|
| Format | `cargo fmt --all --check` |
| Lint | `cargo clippy --workspace --all-targets --locked -- -D warnings` |
| Tests | `TELAMON_REQUIRE_DBUS_TESTS=1 cargo test --workspace --locked` (needs `dbus-daemon` for the private-bus tests, the tray's `tests/tray_bus.rs` among them; without the variable they skip) |
| Glow | `cmake -S apps/telamon-updater-glow -B /build/glow -G Ninja && cmake --build /build/glow` (needs telamon-ui, layer-shell-qt, Qt 6 devel; `--target all_qmllint` for qmllint) |
| RPMs | `packaging/build-rpm.sh /src/out` (host: `podman run --rm --security-opt label=disable -v "$PWD":/src ... fedora:44 /src/packaging/build-rpm.sh /src/out`; unset `CARGO_TARGET_DIR` in the container) |
