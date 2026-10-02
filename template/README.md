# Atlas app template

A minimal Atlas app: Kirigami UI (QML compiled ahead of time by `qt_add_qml_module`),
one Rust QObject exposed through CXX-Qt, built with CMake and Corrosion, and a
dependency on `atlas-core`. It is not part of the atlas-updater workspace and
builds on its own.

## Start a new app from it

1. Copy this directory to the new app's repository.
2. Rename (search and replace in every file, and rename the files that carry it):
   - crate `atlas-app-template` / lib `atlas_app_template` (Cargo.toml, CMakeLists.txt)
   - QML module URI `net.eterneon.atlas.apptemplate` (CMakeLists.txt, main.cpp)
   - app ID and desktop file `net.eterneon.atlas.apptemplate` (main.cpp, data/)
   - binary name `atlas-app-template` (CMakeLists.txt, main.cpp)
   - `atlas_backend_new` and the `atlas_app` C++ namespace if you like
3. In `Cargo.toml`, switch `atlas-core` from the path form to the git form
   (the comment shows it).
4. Add properties and invokables to `src/backend.rs`, pages to `qml/` and to the
   `QML_FILES` list in `CMakeLists.txt`.

## Build (Fedora 44)

```sh
dnf install cmake ninja-build gcc-c++ cargo corrosion qt6-qtbase-devel \
  qt6-qtdeclarative-devel kf6-kirigami-devel kf6-qqc2-desktop-style
cmake -S . -B build -G Ninja -DCMAKE_BUILD_TYPE=Release
cmake --build build
QT_QPA_PLATFORM=offscreen ./build/atlas-app-template
```

## How it fits together

- `main.cpp` is the only C++: it starts Qt and loads the QML module.
- `src/lib.rs` exports `atlas_backend_new()`, which hands the Rust `Backend`
  QObject to the QML engine (`required property var backend` in `qml/Main.qml`).
- Slow work runs on a thread and posts back with `qt_thread().queue(..)`; never
  block the GUI thread.
- Follow the system Plasma theme: no hard-coded colours.
