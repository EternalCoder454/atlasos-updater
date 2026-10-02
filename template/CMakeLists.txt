cmake_minimum_required(VERSION 3.24)
project(atlas-app-template VERSION 0.1.0 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_AUTOMOC ON)

include(GNUInstallDirs)

find_package(Qt6 6.8 REQUIRED COMPONENTS Core Gui Qml Quick QuickControls2 Widgets)
qt_policy(SET QTP0001 NEW)
find_package(Corrosion REQUIRED)

# Rust crate (static library). cxx-qt-build finds Qt through $QMAKE.
find_program(QMAKE6 NAMES qmake6 qmake REQUIRED)
corrosion_import_crate(MANIFEST_PATH ${CMAKE_CURRENT_SOURCE_DIR}/Cargo.toml CRATES atlas-app-template)
corrosion_set_env_vars(atlas_app_template QMAKE=${QMAKE6})

qt_add_executable(template-app main.cpp)
set_target_properties(template-app PROPERTIES OUTPUT_NAME atlas-app-template)

# qt_add_qml_module compiles the QML to C++ with qmlcachegen at build time and
# embeds it in the executable: no .qml files are read at run time.
qt_add_qml_module(template-app
    URI net.eterneon.atlas.apptemplate
    VERSION 1.0
    QML_FILES
        qml/Main.qml
        qml/MainPage.qml
)

target_link_libraries(template-app PRIVATE
    atlas_app_template
    Qt6::Core Qt6::Gui Qt6::Qml Qt6::Quick Qt6::QuickControls2 Qt6::Widgets)

install(TARGETS template-app RUNTIME DESTINATION ${CMAKE_INSTALL_BINDIR})
install(FILES data/net.eterneon.atlas.apptemplate.desktop
        DESTINATION ${CMAKE_INSTALL_DATADIR}/applications)
