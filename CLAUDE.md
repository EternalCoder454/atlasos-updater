# Atlas Updater and atlas-core

Rust + Qt 6.11 + Kirigami (CXX-Qt) apps for AtlasOS, a Fedora Kinoite 44 bootc
image (repo `~/Documents/AtlasOS`). Read `docs/DESIGN.md` first: it fixes the
layout, the system helper's D-Bus API and the atlas-core API. Change it only
together with the code that implements the change.

## Hard rules

- **Build and test inside a `registry.fedoraproject.org/fedora:44` container**,
  never on the host: the host lacks the Qt/KF6/flatpak -devel packages and
  sudo. Mount the repo at `/src` (its host path contains a space, so quote it).
  Keep caches in named podman volumes so rebuilds are fast:
  `-v atlas-cargo:/root/.cargo/registry -v atlas-dnf:/var/cache/libdnf5`.
  Use a separate target dir per agent or task (`CARGO_TARGET_DIR=/src/target/<name>`).
- **Never run the GUI on the user's display.** For smoke tests inside the
  container, use `QT_QPA_PLATFORM=offscreen`, or `xvfb-run -a -s "-screen 0 1920x1080x24"`.
  Real end-to-end tests happen in the AtlasOS test VM, which the lead runs.
- **Never run bootc, flatpak transactions or the system helper against the
  host.** Unit-test the logic (argument validation, ref/tag rewriting, JSON
  parsing, history) with fixtures. The helper's D-Bus and polkit paths are
  tested in the VM.
- The system helper accepts only the six methods in DESIGN.md. Never add a
  method that takes a command, path, image ref or argv.
- Commit only the paths you own (`git commit -- <paths>`). Other agents may be
  committing in this repo at the same time; retry if `index.lock` exists.
  Don't push.
- Licence: MIT. App ID `net.eterneon.atlas.updater`. Git identity is set in the repo.

## Commands

| Task | Command (inside the container, in /src) |
|---|---|
| Format | `cargo fmt --all --check` |
| Lint | `cargo clippy --workspace --all-targets -- -D warnings` |
| Tests | `cargo test --workspace` |
| RPMs | `packaging/build-rpm.sh /src/out` (host: `podman run --rm -v "$PWD":/src:Z ... fedora:44 /src/packaging/build-rpm.sh /src/out`) |
