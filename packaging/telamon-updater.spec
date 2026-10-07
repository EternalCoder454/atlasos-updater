# Telamon Updater and its system helper, for Telamon OS (they were the Atlas
# system helper and Atlas Updater, atlas-system-helper and atlas-updater).
# Build without the app (telamon-system-helper only):  rpmbuild --without app ...
# The main package is Telamon Updater's background part (tray, glow, the app
# update worker; its window is Telamon Settings' Updates page); the helper is
# the subpackage telamon-system-helper.
%bcond app 1

# No debuginfo subpackage: the Rust flags below keep symbols (debuginfo=2,
# strip=none) and the binaries are shipped as built.
%global debug_package %{nil}

# No LTO for the C++ glow program.
%global _lto_cflags %{nil}

# No annobin notes: they record each C file's absolute path (the crates' C,
# compiled under the temporary build directory), which made every build
# differ. They only serve annocheck; the hardening flags stay.
%undefine _annotated_build

# --define "_telamon_build_cache <dir>" (packaging/build-rpm.sh passes it when
# TELAMON_BUILD_CACHE (or ATLAS_BUILD_CACHE) is set) keeps cargo's downloads, cargo's output and the
# glow's CMake build in <dir>, so a rebuild only compiles what changed.
%if 0%{?_telamon_build_cache:1}
%global cargo_target_dir %{_telamon_build_cache}/target
%global cargo_home %{_telamon_build_cache}/cargo-home
%global _vpath_builddir %{_telamon_build_cache}/cmake
%else
%global cargo_target_dir target
%global cargo_home %{_builddir}/cargo-home
%endif

Name:           telamon-updater
Version:        0.3.0
Release:        1%{?dist}
Summary:        Telamon Updater for Telamon OS
License:        MIT
URL:            https://github.com/EternalCoder454/atlasos-updater
Source0:        %{name}-%{version}.tar.gz

BuildRequires:  cargo
BuildRequires:  rust
# %%build_rustflags
BuildRequires:  rust-srpm-macros
BuildRequires:  gcc
BuildRequires:  systemd-rpm-macros
%if %{with app}
BuildRequires:  cmake
BuildRequires:  ninja-build
BuildRequires:  desktop-file-utils
BuildRequires:  libappstream-glib
BuildRequires:  pkgconfig(flatpak)
BuildRequires:  pkgconfig(glib-2.0)
BuildRequires:  cmake(Qt6Core)
BuildRequires:  cmake(Qt6Gui)
BuildRequires:  cmake(Qt6Qml)
BuildRequires:  cmake(Qt6Quick)
BuildRequires:  cmake(Qt6QmlTools)
BuildRequires:  qt6-qtbase-devel
BuildRequires:  pkgconfig(gio-2.0)
# QML modules qmlcachegen resolves at build time (not linked). telamon-ui comes
# from the Telamon framework, which is in no repository: install its RPMs
# first (build-rpm.sh does, given TELAMON_LOCAL_RPMS).
BuildRequires:  kf6-kirigami-devel
BuildRequires:  telamon-ui >= 2.0.0
# org.kde.layershell, for the screen-edge glow (ScreenGlow.qml)
BuildRequires:  layer-shell-qt
# (flatpak and glib above are for `telamon-updater --worker`: libflatpak,
# through telamon-updater-core; the Rust programs are plain cargo builds.)
%endif

%if %{with app}
# Was atlas-updater (0.2.0 and before). Provides keeps anything that asks for
# atlas-updater working; Obsoletes replaces it on upgrade.
Provides:       atlas-updater = %{version}-%{release}
Obsoletes:      atlas-updater < 0.3.0
Requires:       telamon-system-helper = %{version}-%{release}
# The window: Telamon Settings' Updates page (0.3.0 is the first with it).
# Every way to open updates (the panel icon, the notifications, the old
# telamon-updater and atlas-updater commands) starts telamon-settings.
Requires:       telamon-settings >= 0.3.0
# Telamon.Ui, the shared look (the Telamon framework), for the glow. 2.0.0 is
# the first with the Telamon names; it also ships
# /usr/share/telamon/crash-reporting.toml, the crash report server.
Requires:       telamon-ui >= 2.0.0
Requires:       kf6-kirigami
# The update glow runs around the screens' edges as layer-shell overlays.
Requires:       layer-shell-qt
Requires:       qt6-qtdeclarative
%endif

%description
Telamon Updater keeps Telamon OS and Flatpak apps up to date in the
background: it checks for updates, tells you when one is ready, restarts when
you scheduled it, draws a glow around the screen edges while the system is
being changed and collects crash reports for your review. The window (update,
go back, switch channel) is the Updates page of Telamon Settings. Packaged
with its system helper, telamon-system-helper. (Both were Atlas Updater and
atlas-system-helper before 0.3.0.)

%package -n telamon-system-helper
Summary:        System helper behind Telamon Updater
# Was atlas-system-helper (0.2.0 and before), and before that atlas-core
# (0.1.0-1). Provides keeps anything that asks for either working; Obsoletes
# replaces an installed one on upgrade. (atlas-core's release went to 2 so
# that 0.1.0-1 counts as older.)
Provides:       atlas-system-helper = %{version}-%{release}
Obsoletes:      atlas-system-helper < 0.3.0
Provides:       atlas-core = %{version}-%{release}
Obsoletes:      atlas-core < %{version}-%{release}
Requires:       bootc
# stand in for bootc on a system with local rpm-ostree changes
Requires:       rpm-ostree
Requires:       skopeo
Requires:       polkit
Requires:       dbus-common
# Crash reports are posted with curl (only when the user sends one).
Requires:       curl
%{?systemd_requires}

%description -n telamon-system-helper
The Telamon system helper: a D-Bus activated service that runs bootc for
Telamon apps after a polkit check, with the polkit actions, D-Bus policy and
systemd units it needs. It also records the booted image in the update history
at boot. It exits after 60 seconds without calls. Also protects the Telamon
packages from removal by dnf.

For this release it also answers under its old names, for what has not moved
yet (the OS image's scripts, older apps): the D-Bus name
net.eterneon.atlas.SystemHelper and the polkit actions net.eterneon.atlas.system.*,
/usr/libexec/atlas-system-helper, and the units atlas-*.service (aliases).

%prep
%autosetup -n %{name}-%{version}

%build
# NETWORK: cargo fetches the crates from crates.io during %%build. That works in podman and with
# `rpmbuild` on a networked machine, but not in an offline mock/Koji build;
# for that, vendor the crates into the source tarball first.
export CARGO_HOME=%{cargo_home}
%if 0%{?_telamon_build_cache:1}
export CARGO_TARGET_DIR=%{cargo_target_dir}
%endif
# Fedora's Rust flags (hardening, build-id, ...). Their -Ccodegen-units=1 gives way to Cargo.toml's 4 (the last one wins), and
# there is no LTO. With them, rebuilding after a change to the engine took 3
# times as long, for binaries a fifth smaller.
#
# Reproducible: build-rpm.sh builds in a new temporary directory each time,
# and its path would end up in the binaries (panic and assert locations), so
# every image would carry a new updater. Every build path is mapped to a
# fixed name: the sources, cargo's home and output, and the glow's CMake build.
remap="--remap-path-prefix=$PWD=. --remap-path-prefix=%{cargo_home}=cargo"
prefixmap="-ffile-prefix-map=$PWD=. -ffile-prefix-map=%{cargo_home}=cargo"
%if 0%{?_telamon_build_cache:1}
# Outside the sources only with the build cache.
remap="$remap --remap-path-prefix=%{cargo_target_dir}=target --remap-path-prefix=%{_vpath_builddir}=build"
prefixmap="$prefixmap -ffile-prefix-map=%{cargo_target_dir}=target -ffile-prefix-map=%{_vpath_builddir}=build"
%endif
export RUSTFLAGS="%{build_rustflags} -Ccodegen-units=4 $remap"
export CFLAGS="%{build_cflags} $prefixmap"
export CXXFLAGS="%{build_cxxflags} $prefixmap"
export CARGO_PROFILE_RELEASE_STRIP=none
export CARGO_PROFILE_RELEASE_LTO=false
# Beside the glow's build: each leaves CPUs idle at times. --locked, with or
# without the app: the root helper is built from exactly the commits in
# Cargo.lock, never from wherever a framework tag points now. -p builds only
# the named crates, so the rest of the workspace costs nothing.
# The Rust programs are plain cargo builds: the tray (telamon-updater-tray, no
# Qt) and telamon-updater (the forwarder and the app update worker; libflatpak).
cargo build --release --locked -p telamon-update-engine --bin telamon-system-helper \
    %{?with_app:-p telamon-updater-tray --bin telamon-updater-tray -p telamon-updater --bin telamon-updater} &
helper=$!
%if %{with app}
# The glow (telamon-updater-glow, Qt) is the only CMake project.
# (checked with rpmspec --eval: %%cmake honours _vpath_srcdir, not __cmake_source_dir)
%global _vpath_srcdir apps/telamon-updater-glow
%cmake -G Ninja -DCMAKE_BUILD_TYPE=Release
%cmake_build
%endif
wait $helper

%install
d=crates/telamon-update-engine/data
install -Dpm0755 %{cargo_target_dir}/release/telamon-system-helper %{buildroot}%{_libexecdir}/telamon-system-helper
# What the OS image's scripts call (`atlas-system-helper record-event ...`).
ln -s telamon-system-helper %{buildroot}%{_libexecdir}/atlas-system-helper
# Both D-Bus identities: one helper, two names (see docs/DESIGN.md).
for n in telamon atlas; do
    install -Dpm0644 $d/dbus-1/system.d/net.eterneon.$n.SystemHelper.conf \
        %{buildroot}%{_datadir}/dbus-1/system.d/net.eterneon.$n.SystemHelper.conf
    install -Dpm0644 $d/dbus-1/system-services/net.eterneon.$n.SystemHelper.service \
        %{buildroot}%{_datadir}/dbus-1/system-services/net.eterneon.$n.SystemHelper.service
done
for u in system-helper.service record-boot.service drivers.service drivers.timer; do
    install -Dpm0644 $d/systemd/telamon-$u %{buildroot}%{_unitdir}/telamon-$u
    # the old unit names stay as aliases: units the image enabled under them work
    ln -s telamon-$u %{buildroot}%{_unitdir}/atlas-$u
done
install -Dpm0644 $d/systemd/50-telamon-system-helper.preset %{buildroot}%{_presetdir}/50-telamon-system-helper.preset
install -Dpm0644 $d/polkit-1/actions/net.eterneon.telamon.system.policy \
    %{buildroot}%{_datadir}/polkit-1/actions/net.eterneon.telamon.system.policy
install -Dpm0644 $d/polkit-1/rules.d/50-telamon-system.rules \
    %{buildroot}%{_datadir}/polkit-1/rules.d/50-telamon-system.rules
install -Dpm0644 $d/dnf/protected.d/telamon-updater.conf %{buildroot}%{_sysconfdir}/dnf/protected.d/telamon-updater.conf
%if %{with app}
# /usr/libexec/telamon-updater-glow: started by the tray, never by the user.
%cmake_install
install -Dpm0755 %{cargo_target_dir}/release/telamon-updater %{buildroot}%{_bindir}/telamon-updater
install -Dpm0755 %{cargo_target_dir}/release/telamon-updater-tray %{buildroot}%{_bindir}/telamon-updater-tray
a=apps/telamon-updater/data
install -Dpm0644 $a/net.eterneon.telamon.updater.desktop %{buildroot}%{_datadir}/applications/net.eterneon.telamon.updater.desktop
install -Dpm0644 $a/net.eterneon.telamon.updater-tray.desktop %{buildroot}%{_sysconfdir}/xdg/autostart/net.eterneon.telamon.updater-tray.desktop
install -Dpm0644 $a/telamon-updater.notifyrc %{buildroot}%{_datadir}/knotifications6/telamon-updater.notifyrc
install -Dpm0644 $a/net.eterneon.telamon.updater.metainfo.xml %{buildroot}%{_datadir}/metainfo/net.eterneon.telamon.updater.metainfo.xml
install -Dpm0644 $a/net.eterneon.telamon.updater.svg %{buildroot}%{_datadir}/icons/hicolor/scalable/apps/net.eterneon.telamon.updater.svg
for i in net.eterneon.telamon.updater-symbolic.svg net.eterneon.telamon.updater-ready-symbolic.svg; do
    install -Dpm0644 $a/$i %{buildroot}%{_datadir}/icons/hicolor/scalable/status/$i
done
# The old program names, which the OS image's autostart and older scripts use.
ln -s telamon-updater %{buildroot}%{_bindir}/atlas-updater
ln -s telamon-updater-tray %{buildroot}%{_bindir}/atlas-updater-tray
# The tray answers to both session D-Bus names; each has its activation file.
install -Dpm0644 apps/telamon-updater-tray/data/net.eterneon.telamon.updater.Tray.service \
    %{buildroot}%{_datadir}/dbus-1/services/net.eterneon.telamon.updater.Tray.service
install -Dpm0644 apps/telamon-updater-tray/data/net.eterneon.atlas.updater.Tray.service \
    %{buildroot}%{_datadir}/dbus-1/services/net.eterneon.atlas.updater.Tray.service
%endif

%check
# The helper's old names must still resolve to what they stand for.
test "$(readlink %{buildroot}%{_libexecdir}/atlas-system-helper)" = telamon-system-helper
test -x %{buildroot}%{_libexecdir}/telamon-system-helper
for u in system-helper.service record-boot.service drivers.service drivers.timer; do
    test "$(readlink %{buildroot}%{_unitdir}/atlas-$u)" = telamon-$u
    test -f %{buildroot}%{_unitdir}/telamon-$u
done
# Both D-Bus names start the one unit; both polkit action sets are there.
for n in telamon atlas; do
    grep -qx 'SystemdService=telamon-system-helper.service' \
        %{buildroot}%{_datadir}/dbus-1/system-services/net.eterneon.$n.SystemHelper.service
    grep -q "allow own=\"net.eterneon.$n.SystemHelper\"" \
        %{buildroot}%{_datadir}/dbus-1/system.d/net.eterneon.$n.SystemHelper.conf
    test "$(grep -c "<action id=\"net.eterneon.$n.system\." %{buildroot}%{_datadir}/polkit-1/actions/net.eterneon.telamon.system.policy)" = 5
    grep -q "net.eterneon.$n.system.upgrade" %{buildroot}%{_datadir}/polkit-1/rules.d/50-telamon-system.rules
done
# No path of the build tree in what is shipped (builds must not differ by where
# they ran).
for f in %{buildroot}%{_libexecdir}/telamon-system-helper \
%if %{with app}
    %{buildroot}%{_bindir}/telamon-updater %{buildroot}%{_bindir}/telamon-updater-tray \
    %{buildroot}%{_libexecdir}/telamon-updater-glow \
%endif
    ; do
    if grep -aF -e "%{_builddir}" -e "%{cargo_home}" "$f" >/dev/null; then
        echo "$f contains a path of the build tree" >&2
        exit 1
    fi
done
%if %{with app}
test "$(readlink %{buildroot}%{_bindir}/atlas-updater)" = telamon-updater
test "$(readlink %{buildroot}%{_bindir}/atlas-updater-tray)" = telamon-updater-tray
test -x %{buildroot}%{_bindir}/telamon-updater
test -x %{buildroot}%{_bindir}/telamon-updater-tray
# The glow: built (its QML is compiled in), and answers without a screen.
test -x %{buildroot}%{_libexecdir}/telamon-updater-glow
test "$(%{buildroot}%{_libexecdir}/telamon-updater-glow --version)" = "telamon-updater-glow %{version}"
# Telamon Updater has no window: its launcher is hidden (the menu entries are
# Telamon Settings'), kept for pins and old launchers.
grep -qx 'NoDisplay=true' %{buildroot}%{_datadir}/applications/net.eterneon.telamon.updater.desktop
grep -qx 'Exec=telamon-updater' %{buildroot}%{_datadir}/applications/net.eterneon.telamon.updater.desktop
# (The forwarder's hand-over to telamon-settings is tested by cargo test: the
# program to start can only be replaced in a debug build.)
desktop-file-validate %{buildroot}%{_datadir}/applications/net.eterneon.telamon.updater.desktop
desktop-file-validate %{buildroot}%{_sysconfdir}/xdg/autostart/net.eterneon.telamon.updater-tray.desktop
appstream-util validate-relax --nonet \
    %{buildroot}%{_datadir}/metainfo/net.eterneon.telamon.updater.metainfo.xml
for n in telamon atlas; do
    grep -qx "Name=net.eterneon.$n.updater.Tray" %{buildroot}%{_datadir}/dbus-1/services/net.eterneon.$n.updater.Tray.service
    grep -qx 'Exec=%{_bindir}/telamon-updater-tray' %{buildroot}%{_datadir}/dbus-1/services/net.eterneon.$n.updater.Tray.service
done
%endif

%post -n telamon-system-helper
%systemd_post telamon-record-boot.service telamon-drivers.timer

%preun -n telamon-system-helper
%systemd_preun telamon-record-boot.service telamon-drivers.service telamon-drivers.timer

%postun -n telamon-system-helper
%systemd_postun telamon-record-boot.service telamon-drivers.service telamon-drivers.timer

# Replacing atlas-core or atlas-system-helper: their %%preun runs after our
# %%post (they are being erased, not upgraded) and disables the record-boot
# unit and the drivers timer (through the old names, which are aliases of ours
# now). Preset them again once the old package is gone.
%triggerpostun -n telamon-system-helper -- atlas-core < 0.1.0-2
systemctl --no-reload preset telamon-record-boot.service telamon-drivers.timer >/dev/null 2>&1 || :

%triggerpostun -n telamon-system-helper -- atlas-system-helper < 0.3.0
systemctl --no-reload preset telamon-record-boot.service telamon-drivers.timer >/dev/null 2>&1 || :

%files -n telamon-system-helper
%license LICENSE
%{_libexecdir}/telamon-system-helper
%{_libexecdir}/atlas-system-helper
%{_datadir}/dbus-1/system.d/net.eterneon.telamon.SystemHelper.conf
%{_datadir}/dbus-1/system.d/net.eterneon.atlas.SystemHelper.conf
%{_datadir}/dbus-1/system-services/net.eterneon.telamon.SystemHelper.service
%{_datadir}/dbus-1/system-services/net.eterneon.atlas.SystemHelper.service
%{_unitdir}/telamon-system-helper.service
%{_unitdir}/telamon-record-boot.service
%{_unitdir}/telamon-drivers.service
%{_unitdir}/telamon-drivers.timer
%{_unitdir}/atlas-system-helper.service
%{_unitdir}/atlas-record-boot.service
%{_unitdir}/atlas-drivers.service
%{_unitdir}/atlas-drivers.timer
%{_presetdir}/50-telamon-system-helper.preset
%{_datadir}/polkit-1/actions/net.eterneon.telamon.system.policy
%{_datadir}/polkit-1/rules.d/50-telamon-system.rules
%config(noreplace) %{_sysconfdir}/dnf/protected.d/telamon-updater.conf

%if %{with app}
%files
%license LICENSE
%{_bindir}/telamon-updater
%{_bindir}/telamon-updater-tray
%{_libexecdir}/telamon-updater-glow
%{_bindir}/atlas-updater
%{_bindir}/atlas-updater-tray
%{_datadir}/dbus-1/services/net.eterneon.telamon.updater.Tray.service
%{_datadir}/dbus-1/services/net.eterneon.atlas.updater.Tray.service
%{_datadir}/applications/net.eterneon.telamon.updater.desktop
%{_sysconfdir}/xdg/autostart/net.eterneon.telamon.updater-tray.desktop
%{_datadir}/knotifications6/telamon-updater.notifyrc
%{_datadir}/metainfo/net.eterneon.telamon.updater.metainfo.xml
%{_datadir}/icons/hicolor/scalable/apps/net.eterneon.telamon.updater.svg
%{_datadir}/icons/hicolor/scalable/status/net.eterneon.telamon.updater-symbolic.svg
%{_datadir}/icons/hicolor/scalable/status/net.eterneon.telamon.updater-ready-symbolic.svg
%endif

%changelog
* Wed Oct 07 2026 Telamon <telamon@eterneon.net> - 0.3.0-1
- Renamed to Telamon Updater; the window is Telamon Settings' Updates page.
  Telamon Updater is the background part only: the panel icon and its menu,
  the notifications, the automatic checks, the screen-edge glow while the
  system is being changed, the system helper and crash report collection
- Everything that opened the window starts telamon-settings now (the panel
  icon, the menu, the notifications, and the old telamon-updater and
  atlas-updater commands, which still run the app update worker)
- The glow is a program of its own, /usr/libexec/telamon-updater-glow, which
  the tray starts and ends: it shows while Settings says so (the new session
  D-Bus method SetWorking(b) of the tray) or the system helper reports an
  update running, also with Settings closed
- Requires telamon-settings >= 0.3.0 and telamon-ui >= 2.0.0 (Telamon.Ui 2.0.0)
- The binaries are telamon-updater, telamon-updater-tray and
  /usr/libexec/telamon-system-helper; the app ID is net.eterneon.telamon.updater;
  the settings and state files, the systemd units, the D-Bus names and polkit
  actions are renamed too
- The old names still work for this release, to be removed in the next:
  atlas-updater, atlas-updater-tray and atlas-system-helper (links), the
  atlas-*.service units (aliases), the helper's D-Bus name, object path,
  interface, errors and polkit actions net.eterneon.atlas.*, the tray's session
  D-Bus name (Reload), the lock files (both names are taken), ATLAS_UPDATER_*
  variables
- Settings and state under the old names are moved to the new ones
- Provides and Obsoletes atlas-updater and atlas-system-helper

* Mon Oct 05 2026 Atlas <atlas@eterneon.net> - 0.2.0-1
- Firmware updates through fwupd
- The progress glow runs around the screens' edges
- Drivers: switch to the atlasos-nvidia image on NVIDIA Turing or newer, and back

* Sun Oct 04 2026 Atlas <atlas@eterneon.net> - 0.1.0-2
- atlas-core is now atlas-system-helper (Provides and Obsoletes atlas-core)

* Fri Oct 02 2026 Atlas <atlas@eterneon.net> - 0.1.0-1
- First package
