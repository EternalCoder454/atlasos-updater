# Atlas core (system helper) and Atlas Updater for AtlasOS.
# Build without the app (atlas-core only):  rpmbuild --without app ...
%bcond app 1

# Rust binaries are stripped by the release profile; there is no debuginfo.
%global debug_package %{nil}

Name:           atlas
Version:        0.1.0
Release:        1%{?dist}
Summary:        Atlas core helper and Atlas Updater for AtlasOS
License:        MIT
URL:            https://github.com/EternalCoder454/atlasos-updater
Source0:        atlas-%{version}.tar.gz

BuildRequires:  cargo
BuildRequires:  rust
BuildRequires:  gcc
BuildRequires:  systemd-rpm-macros
%if %{with app}
BuildRequires:  cmake
BuildRequires:  ninja-build
BuildRequires:  extra-cmake-modules
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
BuildRequires:  cmake(KF6CoreAddons)
BuildRequires:  cmake(KF6I18n)
BuildRequires:  cmake(KF6Kirigami)
BuildRequires:  cmake(KF6Notifications)
BuildRequires:  cmake(KF6StatusNotifierItem)
BuildRequires:  cmake(KF6Config)
%endif

%description
Source package for atlas-core and atlas-updater.

%package -n atlas-core
Summary:        Shared library and system helper for Atlas apps
Requires:       bootc
Requires:       polkit
Requires:       dbus-common
%{?systemd_requires}

%description -n atlas-core
The Atlas system helper: a D-Bus activated service that runs bootc for Atlas
apps after a polkit check, with the polkit actions, D-Bus policy and systemd
units it needs. It also records the booted image in the update history at boot.
It exits after 60 seconds without calls. Also protects the Atlas packages from
removal by dnf.

%if %{with app}
%package -n atlas-updater
Summary:        Atlas Updater for AtlasOS
Requires:       atlas-core = %{version}-%{release}

%description -n atlas-updater
Atlas Updater shows, downloads and stages AtlasOS and Flatpak updates, lets you
go back to the previous version and switch update channel.
%endif

%prep
%autosetup -n atlas-%{version}
%if %{without app}
sed -i 's|^members = .*|members = ["crates/atlas-core"]|' Cargo.toml
%endif

%build
export CARGO_HOME=%{_builddir}/cargo-home
cargo build --release -p atlas-core --bin atlas-system-helper
%if %{with app}
cmake -S apps/atlas-updater -B %{_vpath_builddir}/app -G Ninja \
    -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX=/usr
cmake --build %{_vpath_builddir}/app
%endif

%install
d=crates/atlas-core/data
install -Dpm0755 target/release/atlas-system-helper %{buildroot}%{_libexecdir}/atlas-system-helper
install -Dpm0644 $d/dbus-1/system.d/net.eterneon.atlas.SystemHelper.conf \
    %{buildroot}%{_datadir}/dbus-1/system.d/net.eterneon.atlas.SystemHelper.conf
install -Dpm0644 $d/dbus-1/system-services/net.eterneon.atlas.SystemHelper.service \
    %{buildroot}%{_datadir}/dbus-1/system-services/net.eterneon.atlas.SystemHelper.service
install -Dpm0644 $d/systemd/atlas-system-helper.service %{buildroot}%{_unitdir}/atlas-system-helper.service
install -Dpm0644 $d/systemd/atlas-record-boot.service %{buildroot}%{_unitdir}/atlas-record-boot.service
install -Dpm0644 $d/systemd/50-atlas-core.preset %{buildroot}%{_presetdir}/50-atlas-core.preset
install -Dpm0644 $d/polkit-1/actions/net.eterneon.atlas.system.policy \
    %{buildroot}%{_datadir}/polkit-1/actions/net.eterneon.atlas.system.policy
install -Dpm0644 $d/polkit-1/rules.d/50-atlas-system.rules \
    %{buildroot}%{_datadir}/polkit-1/rules.d/50-atlas-system.rules
install -Dpm0644 $d/atlas/crash-reporting.toml %{buildroot}%{_datadir}/atlas/crash-reporting.toml
install -Dpm0644 $d/dnf/protected.d/atlas.conf %{buildroot}%{_sysconfdir}/dnf/protected.d/atlas.conf
%if %{with app}
DESTDIR=%{buildroot} cmake --install %{_vpath_builddir}/app
%endif

%post -n atlas-core
%systemd_post atlas-record-boot.service

%preun -n atlas-core
%systemd_preun atlas-record-boot.service

%postun -n atlas-core
%systemd_postun atlas-record-boot.service

%files -n atlas-core
%license LICENSE
%{_libexecdir}/atlas-system-helper
%{_datadir}/dbus-1/system.d/net.eterneon.atlas.SystemHelper.conf
%{_datadir}/dbus-1/system-services/net.eterneon.atlas.SystemHelper.service
%{_unitdir}/atlas-system-helper.service
%{_unitdir}/atlas-record-boot.service
%{_presetdir}/50-atlas-core.preset
%{_datadir}/polkit-1/actions/net.eterneon.atlas.system.policy
%{_datadir}/polkit-1/rules.d/50-atlas-system.rules
%{_datadir}/atlas/crash-reporting.toml
%config %{_sysconfdir}/dnf/protected.d/atlas.conf

%if %{with app}
%files -n atlas-updater
%license LICENSE
/usr/bin/atlas-updater
/usr/share/applications/net.eterneon.atlas.updater.desktop
/etc/xdg/autostart/net.eterneon.atlas.updater-tray.desktop
/usr/share/knotifications6/atlas-updater.notifyrc
/usr/share/metainfo/net.eterneon.atlas.updater.metainfo.xml
/usr/share/icons/hicolor/scalable/apps/net.eterneon.atlas.updater.svg
%endif

%changelog
* Fri Oct 02 2026 Atlas <atlas@eterneon.net> - 0.1.0-1
- First package
