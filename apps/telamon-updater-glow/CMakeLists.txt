cmake_minimum_required(VERSION 3.24)
project(telamon-updater-glow VERSION 0.3.0 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_AUTOMOC ON)

if(CMAKE_INSTALL_PREFIX_INITIALIZED_TO_DEFAULT)
    set(CMAKE_INSTALL_PREFIX /usr CACHE PATH "" FORCE)
endif()
include(GNUInstallDirs)
find_package(Qt6 6.8 REQUIRED COMPONENTS Core Gui Qml Quick)
qt_policy(SET QTP0001 NEW)
qt_policy(SET QTP0004 NEW)

# Telamon.Ui is installed like Kirigami (telamon-ui, from telamon-framework):
# QML imports it and nothing links it. qmlcachegen and qmllint read its types
# from the installed copy.
if(NOT EXISTS "${QT6_INSTALL_PREFIX}/${QT6_INSTALL_QML}/Telamon/Ui/qmldir")
    message(FATAL_ERROR "Telamon.Ui is not installed: install telamon-ui")
endif()

qt_add_executable(telamon-updater-glow cpp/main.cpp)
target_compile_definitions(telamon-updater-glow PRIVATE TELAMON_UPDATER_GLOW_VERSION="${PROJECT_VERSION}")

# qmlcachegen compiles the QML to C++ at build time; the program reads no
# .qml from disk at run time.
qt_add_qml_module(telamon-updater-glow
    URI net.eterneon.telamon.updater.glow
    VERSION 1.0
    QML_FILES
        qml/Main.qml qml/ScreenGlow.qml qml/GlowWindow.qml qml/GlowLayer.qml qml/GlowFrame.qml
)

target_link_libraries(telamon-updater-glow PRIVATE Qt6::Core Qt6::Gui Qt6::Qml Qt6::Quick)

# Started by the tray, never by the user: libexec, not bin.
install(TARGETS telamon-updater-glow RUNTIME DESTINATION ${CMAKE_INSTALL_LIBEXECDIR})
