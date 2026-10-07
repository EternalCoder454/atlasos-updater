# Telamon Updater and its system helper

Rust + Qt 6.11 + Kirigami (CXX-Qt) apps for Telamon OS (it was AtlasOS), a Fedora
Kinoite 44 bootc image (repo `~/Documents/AtlasOS`, not renamed yet). Read `docs/DESIGN.md` first: it fixes the
layout, the system helper's D-Bus API and the telamon-update-engine API. Change it only
together with the code that implements the change.

## Hard rules

- **Build and test inside a `registry.fedoraproject.org/fedora:44` container**,
  never on the host: the host lacks the Qt/KF6/flatpak -devel packages and
  sudo. Mount the repo at `/src` (its host path contains a space, so quote it).
  Keep caches in named podman volumes so rebuilds are fast:
  `-v telamon-cargo:/root/.cargo/registry -v telamon-dnf:/var/cache/libdnf5`.
  Use a separate target dir per agent or task (`CARGO_TARGET_DIR=/src/target/<name>`).
- **Never run the GUI on the user's display.** For smoke tests inside the
  container, use `QT_QPA_PLATFORM=offscreen`, or `xvfb-run -a -s "-screen 0 1920x1080x24"`.
  Real end-to-end tests happen in the Telamon OS test VM, which the lead runs.
- **Never run bootc, flatpak transactions or the system helper against the
  host.** Unit-test the logic (argument validation, ref/tag rewriting, JSON
  parsing, history) with fixtures. The helper's D-Bus and polkit paths are
  tested in the VM.
- **Telamon.Ui is not here.** It lives in the Telamon framework
  (`EternalCoder454/atlas-framework`, `~/Documents/Atlas Framework`) and the
  app builds against the installed module (telamon-ui), so the build container
  needs its RPMs: build them there (`packaging/build-rpm.sh`), then pass
  `TELAMON_LOCAL_RPMS=<their dir>` (the old `ATLAS_LOCAL_RPMS` works too) to
  this repo's `build-rpm.sh`, or `dnf install` them before a CMake build.
- The system helper accepts only the six methods in DESIGN.md. Never add a
  method that takes a command, path, image ref or argv.
- Commit only the paths you own (`git commit -- <paths>`). Other agents may be
  committing in this repo at the same time; retry if `index.lock` exists.
  Don't push.
- Until the release after 0.3.0 everything is also served under its Atlas
  names (binaries, D-Bus names, polkit actions, units, lock files, files):
  DESIGN.md, "Old names". A change to one identity is a change to both, and
  the tests for the old one must stay until it is removed.
- Licence: MIT. App ID `net.eterneon.telamon.updater` (`net.eterneon.atlas.updater`
  until 0.3.0). Git identity is set in the repo.

## Commands

| Task | Command (inside the container, in /src) |
|---|---|
| Format | `cargo fmt --all --check` |
| Lint | `cargo clippy --workspace --all-targets -- -D warnings` |
| Tests | `TELAMON_REQUIRE_DBUS_TESTS=1 cargo test --workspace` (needs `dbus-daemon` for the private-bus tests; without the variable they skip) |
| RPMs | `packaging/build-rpm.sh /src/out` (host: `podman run --rm --security-opt label=disable -v "$PWD":/src ... fedora:44 /src/packaging/build-rpm.sh /src/out`) |
