# The Atlas system helper and Atlas Updater for AtlasOS.
# Build without the app (atlas-system-helper only):  rpmbuild --without app ...
%bcond app 1

# No debuginfo subpackage: the Rust flags below keep symbols (debuginfo=2,
# strip=none) and the binaries are shipped as built.
%global debug_package %{nil}

# No LTO for the C++ app: its link took longer than compiling it.
%global _lto_cflags %{nil}

# No annobin notes: they record each C file's absolute path (the crates' C,
# compiled under the temporary build directory), which made every build
# differ. They only serve annocheck; the hardening flags stay.
%undefine _annotated_build

# --define "_atlas_build_cache <dir>" (packaging/build-rpm.sh passes it when
# ATLAS_BUILD_CACHE is set) keeps cargo's downloads, cargo's output and the
# CMake build in <dir>, so a rebuild only compiles what changed.
%if 0%{?_atlas_build_cache:1}
%global cargo_target_dir %{_atlas_build_cache}/target
%global cargo_home %{_atlas_build_cache}/cargo-home
%global _vpath_builddir %{_atlas_build_cache}/cmake
%else
%global cargo_target_dir target
%global cargo_home %{_builddir}/cargo-home
%endif

Name:           atlas
Version:        0.1.0
Release:        2%{?dist}
Summary:        Atlas system helper and Atlas Updater for AtlasOS
License:        MIT
URL:            https://github.com/EternalCoder454/atlasos-updater
Source0:        atlas-%{version}.tar.gz

BuildRequires:  cargo
BuildRequires:  rust
# %%build_rustflags
BuildRequires:  rust-srpm-macros
BuildRequires:  gcc
BuildRequires:  systemd-rpm-macros
%if %{with app}
BuildRequires:  cmake
BuildRequires:  ninja-build
BuildRequires:  corrosion
BuildRequires:  desktop-file-utils
BuildRequires:  libappstream-glib
BuildRequires:  pkgconfig(flatpak)
BuildRequires:  pkgconfig(glib-2.0)
BuildRequires:  cmake(Qt6Core)
BuildRequires:  cmake(Qt6Gui)
BuildRequires:  cmake(Qt6Qml)
BuildRequires:  cmake(Qt6Quick)
BuildRequires:  cmake(Qt6QuickControls2)
BuildRequires:  cmake(Qt6Widgets)
BuildRequires:  cmake(Qt6QmlTools)
BuildRequires:  qt6-qtbase-devel
BuildRequires:  cmake(KF6DBusAddons)
BuildRequires:  pkgconfig(gio-2.0)
# QML modules qmlcachegen resolves at build time (not linked). atlas-ui comes
# from atlas-framework, which is in no repository: install its RPMs first
# (build-rpm.sh does, given ATLAS_LOCAL_RPMS).
BuildRequires:  kf6-kirigami-devel
BuildRequires:  atlas-ui >= 1.1.0
%endif

%description
Source package for atlas-system-helper and atlas-updater.

%package -n atlas-system-helper
Summary:        System helper behind Atlas Updater
# Was atlas-core (0.1.0-1). Provides keeps anything that asks for atlas-core
# working; Obsoletes replaces an installed atlas-core on upgrade. The release
# went to 2 so that 0.1.0-1 counts as older.
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

%description -n atlas-system-helper
The Atlas system helper: a D-Bus activated service that runs bootc for Atlas
apps after a polkit check, with the polkit actions, D-Bus policy and systemd
units it needs. It also records the booted image in the update history at boot.
It exits after 60 seconds without calls. Also protects the Atlas packages from
removal by dnf.

%if %{with app}
%package -n atlas-updater
Summary:        Atlas Updater for AtlasOS
Requires:       atlas-system-helper = %{version}-%{release}
# Atlas.Ui, the shared look (atlas-framework). 1.1.0 also ships
# %{_datadir}/atlas/crash-reporting.toml, the crash report server.
Requires:       atlas-ui >= 1.1.0
Requires:       kf6-kirigami
Requires:       kf6-qqc2-desktop-style
Requires:       qt6-qtdeclarative

%description -n atlas-updater
Atlas Updater shows, downloads and stages AtlasOS and Flatpak updates, lets you
go back to the previous version and switch update channel.
%endif

%prep
%autosetup -n atlas-%{version}
%if %{without app}
sed -i 's|^members = .*|members = ["crates/atlas-update-engine"]|' Cargo.toml
%endif

%build
# NETWORK: cargo (and Corrosion, which runs cargo with --locked) fetch the
# crates from crates.io during %%build. That works in podman and with
# `rpmbuild` on a networked machine, but not in an offline mock/Koji build;
# for that, vendor the crates into the source tarball first.
export CARGO_HOME=%{cargo_home}
%if 0%{?_atlas_build_cache:1}
export CARGO_TARGET_DIR=%{cargo_target_dir}
%endif
# Fedora's Rust flags (hardening, build-id, ...), also used by Corrosion's cargo.
# Their -Ccodegen-units=1 gives way to Cargo.toml's 4 (the last one wins), and
# there is no LTO. With them, rebuilding after a change to the engine took 3
# times as long, for binaries a fifth smaller.
#
# Reproducible: build-rpm.sh builds in a new temporary directory each time,
# and its path would end up in the binaries (panic and assert locations), so
# every image would carry a new updater. Every build path is mapped to a
# fixed name: the sources, cargo's home and output, and the CMake build (for
# the C++ that cxx-qt, moc and Corrosion generate there).
remap="--remap-path-prefix=$PWD=. --remap-path-prefix=%{cargo_home}=cargo"
prefixmap="-ffile-prefix-map=$PWD=. -ffile-prefix-map=%{cargo_home}=cargo"
%if 0%{?_atlas_build_cache:1}
# Outside the sources only with the build cache.
remap="$remap --remap-path-prefix=%{cargo_target_dir}=target --remap-path-prefix=%{_vpath_builddir}=build"
prefixmap="$prefixmap -ffile-prefix-map=%{cargo_target_dir}=target -ffile-prefix-map=%{_vpath_builddir}=build"
%endif
export RUSTFLAGS="%{build_rustflags} -Ccodegen-units=4 $remap"
export CFLAGS="%{build_cflags} $prefixmap"
export CXXFLAGS="%{build_cxxflags} $prefixmap"
export CARGO_PROFILE_RELEASE_STRIP=none
export CARGO_PROFILE_RELEASE_LTO=false
# Beside the app's build: each leaves CPUs idle at times. --locked: the root
# helper is built from exactly the crates in Cargo.lock, never newer ones
# (--without app drops the app from the workspace, which changes the lock).
# The tray (atlas-updater-tray, no Qt) is a plain Cargo binary: built here too.
cargo build --release %{?with_app:--locked} -p atlas-update-engine --bin atlas-system-helper \
    %{?with_app:-p atlas-updater-tray --bin atlas-updater-tray} &
helper=$!
%if %{with app}
# (checked with rpmspec --eval: %%cmake honours _vpath_srcdir, not __cmake_source_dir)
%global _vpath_srcdir apps/atlas-updater
%cmake -G Ninja -DCMAKE_BUILD_TYPE=Release
%cmake_build
%endif
wait $helper

%install
d=crates/atlas-update-engine/data
install -Dpm0755 %{cargo_target_dir}/release/atlas-system-helper %{buildroot}%{_libexecdir}/atlas-system-helper
install -Dpm0644 $d/dbus-1/system.d/net.eterneon.atlas.SystemHelper.conf \
    %{buildroot}%{_datadir}/dbus-1/system.d/net.eterneon.atlas.SystemHelper.conf
install -Dpm0644 $d/dbus-1/system-services/net.eterneon.atlas.SystemHelper.service \
    %{buildroot}%{_datadir}/dbus-1/system-services/net.eterneon.atlas.SystemHelper.service
install -Dpm0644 $d/systemd/atlas-system-helper.service %{buildroot}%{_unitdir}/atlas-system-helper.service
install -Dpm0644 $d/systemd/atlas-record-boot.service %{buildroot}%{_unitdir}/atlas-record-boot.service
install -Dpm0644 $d/systemd/50-atlas-system-helper.preset %{buildroot}%{_presetdir}/50-atlas-system-helper.preset
install -Dpm0644 $d/polkit-1/actions/net.eterneon.atlas.system.policy \
    %{buildroot}%{_datadir}/polkit-1/actions/net.eterneon.atlas.system.policy
install -Dpm0644 $d/polkit-1/rules.d/50-atlas-system.rules \
    %{buildroot}%{_datadir}/polkit-1/rules.d/50-atlas-system.rules
install -Dpm0644 $d/dnf/protected.d/atlas.conf %{buildroot}%{_sysconfdir}/dnf/protected.d/atlas.conf
%if %{with app}
%cmake_install
install -Dpm0755 %{cargo_target_dir}/release/atlas-updater-tray %{buildroot}%{_bindir}/atlas-updater-tray
install -Dpm0644 apps/atlas-updater-tray/data/net.eterneon.atlas.updater.Tray.service \
    %{buildroot}%{_datadir}/dbus-1/services/net.eterneon.atlas.updater.Tray.service
%endif

%if %{with app}
%check
desktop-file-validate %{buildroot}%{_datadir}/applications/net.eterneon.atlas.updater.desktop
desktop-file-validate %{buildroot}%{_sysconfdir}/xdg/autostart/net.eterneon.atlas.updater-tray.desktop
appstream-util validate-relax --nonet \
    %{buildroot}%{_datadir}/metainfo/net.eterneon.atlas.updater.metainfo.xml
%endif

%post -n atlas-system-helper
%systemd_post atlas-record-boot.service

%preun -n atlas-system-helper
%systemd_preun atlas-record-boot.service

%postun -n atlas-system-helper
%systemd_postun atlas-record-boot.service

# Replacing atlas-core: its %%preun runs after our %%post (it is being erased,
# not upgraded) and disables atlas-record-boot. Preset it again once it's gone.
%triggerpostun -n atlas-system-helper -- atlas-core < 0.1.0-2
systemctl --no-reload preset atlas-record-boot.service >/dev/null 2>&1 || :

%files -n atlas-system-helper
%license LICENSE
%{_libexecdir}/atlas-system-helper
%{_datadir}/dbus-1/system.d/net.eterneon.atlas.SystemHelper.conf
%{_datadir}/dbus-1/system-services/net.eterneon.atlas.SystemHelper.service
%{_unitdir}/atlas-system-helper.service
%{_unitdir}/atlas-record-boot.service
%{_presetdir}/50-atlas-system-helper.preset
%{_datadir}/polkit-1/actions/net.eterneon.atlas.system.policy
%{_datadir}/polkit-1/rules.d/50-atlas-system.rules
%config(noreplace) %{_sysconfdir}/dnf/protected.d/atlas.conf

%if %{with app}
%files -n atlas-updater
%license LICENSE
%{_bindir}/atlas-updater
%{_bindir}/atlas-updater-tray
%{_datadir}/dbus-1/services/net.eterneon.atlas.updater.Tray.service
%{_datadir}/applications/net.eterneon.atlas.updater.desktop
%{_sysconfdir}/xdg/autostart/net.eterneon.atlas.updater-tray.desktop
%{_datadir}/knotifications6/atlas-updater.notifyrc
%{_datadir}/metainfo/net.eterneon.atlas.updater.metainfo.xml
%{_datadir}/icons/hicolor/scalable/apps/net.eterneon.atlas.updater.svg
%{_datadir}/icons/hicolor/scalable/status/net.eterneon.atlas.updater-symbolic.svg
%{_datadir}/icons/hicolor/scalable/status/net.eterneon.atlas.updater-ready-symbolic.svg
%endif

%changelog
* Sun Oct 04 2026 Atlas <atlas@eterneon.net> - 0.1.0-2
- atlas-core is now atlas-system-helper (Provides and Obsoletes atlas-core)

* Fri Oct 02 2026 Atlas <atlas@eterneon.net> - 0.1.0-1
- First package
