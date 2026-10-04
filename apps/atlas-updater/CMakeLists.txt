cmake_minimum_required(VERSION 3.24)
project(atlas-updater VERSION 0.1.0 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_AUTOMOC ON)

if(CMAKE_INSTALL_PREFIX_INITIALIZED_TO_DEFAULT)
    set(CMAKE_INSTALL_PREFIX /usr CACHE PATH "" FORCE)
endif()
include(GNUInstallDirs)
find_package(Qt6 6.8 REQUIRED COMPONENTS Core Gui Qml Quick QuickControls2 Widgets)
qt_policy(SET QTP0001 NEW)
qt_policy(SET QTP0004 NEW)
find_package(KF6DBusAddons REQUIRED)
find_package(Corrosion REQUIRED)
# atlas-framework-flatpak links libflatpak and GLib into the Rust library.
find_package(PkgConfig REQUIRED)
pkg_check_modules(FLATPAK REQUIRED IMPORTED_TARGET flatpak gio-2.0)

# Rust library (CXX-Qt QObjects, atlas-update-engine). cxx-qt-build finds Qt via $QMAKE.
find_program(QMAKE6 NAMES qmake6 qmake REQUIRED)
corrosion_import_crate(MANIFEST_PATH ${CMAKE_CURRENT_SOURCE_DIR}/../../Cargo.toml
    CRATES atlas-updater LOCKED)
corrosion_set_env_vars(atlas_updater QMAKE=${QMAKE6})

qt_add_executable(atlas-updater-app
    cpp/main.cpp cpp/shell.cpp cpp/shell.h)
set_target_properties(atlas-updater-app PROPERTIES OUTPUT_NAME atlas-updater)
target_compile_definitions(atlas-updater-app PRIVATE ATLAS_UPDATER_VERSION="${PROJECT_VERSION}")

# Atlas.Ui, the shared Atlas look, is installed like Kirigami (atlas-ui, from
# atlas-framework): QML imports it and nothing links it. qmlcachegen and
# qmllint read its types from the installed copy.
if(NOT EXISTS "${QT6_INSTALL_PREFIX}/${QT6_INSTALL_QML}/Atlas/Ui/qmldir")
    message(FATAL_ERROR "Atlas.Ui is not installed: install atlas-ui (atlas-framework)")
endif()

# qmlcachegen compiles every QML/JS file to C++ at build time; the app reads
# no .qml from disk at run time.
set_source_files_properties(qml/dates.js PROPERTIES QT_QML_SKIP_QMLLINT TRUE)
qt_add_qml_module(atlas-updater-app
    URI net.eterneon.atlas.updater
    VERSION 1.0
    QML_FILES
        qml/Main.qml qml/Cards.qml qml/UpdatesPage.qml qml/RollbackPage.qml
        qml/ChannelPage.qml qml/HistoryPage.qml qml/ChangelogPage.qml qml/SettingsPage.qml
        qml/ReportsPage.qml qml/SentReportsPage.qml qml/AboutPage.qml
        qml/dates.js
    RESOURCES
        qml/icons/update-arrow.svg
)

target_link_libraries(atlas-updater-app PRIVATE
    atlas_updater PkgConfig::FLATPAK
    Qt6::Core Qt6::Gui Qt6::Qml Qt6::Quick Qt6::QuickControls2 Qt6::Widgets
    KF6::DBusAddons)

install(TARGETS atlas-updater-app RUNTIME DESTINATION ${CMAKE_INSTALL_BINDIR})
install(FILES data/net.eterneon.atlas.updater.desktop
        DESTINATION ${CMAKE_INSTALL_DATADIR}/applications)
# atlas-updater-tray itself is a plain Cargo binary (packaging/atlas.spec).
install(FILES data/net.eterneon.atlas.updater-tray.desktop
        DESTINATION /etc/xdg/autostart)
install(FILES data/atlas-updater.notifyrc
        DESTINATION ${CMAKE_INSTALL_DATADIR}/knotifications6)
install(FILES data/net.eterneon.atlas.updater.metainfo.xml
        DESTINATION ${CMAKE_INSTALL_DATADIR}/metainfo)
install(FILES data/net.eterneon.atlas.updater.svg
        DESTINATION ${CMAKE_INSTALL_DATADIR}/icons/hicolor/scalable/apps)
# One-colour tray icons, which Plasma paints in the panel's text colour
install(FILES data/net.eterneon.atlas.updater-symbolic.svg
              data/net.eterneon.atlas.updater-ready-symbolic.svg
        DESTINATION ${CMAKE_INSTALL_DATADIR}/icons/hicolor/scalable/status)
