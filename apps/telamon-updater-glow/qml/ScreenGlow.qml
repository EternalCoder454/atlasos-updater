pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Window
import org.kde.kirigami as Kirigami
import Telamon.Ui

// The "the OS image is being changed" glow, drawn around the edges of every
// screen (not inside one window): one soft frame per screen (GlowFrame), no
// rim and no visible corner. It is all that telamon-updater-glow shows:
// Telamon Updater's tray starts that program while an update, a channel
// switch or a go back is being staged (not for app updates, firmware or
// checks), and ends it afterwards.
//
//   Wayland: one full-screen layer-shell overlay per screen (GlowLayer, needs
//            layer-shell-qt).
//   X11:     four frameless, always-on-top tool windows per screen, one strip
//            along each edge, each showing its part of the same frame.
//   Else, or without the layer-shell module: `usable` is false and nothing
//   is drawn (the program logs it).
//
// While `active` is false there are no windows and no timer. The windows take
// no input and are hidden from screen readers. The glow breathes (its
// opacity, 0.6 to 1, over 2.4 s) at 30 frames per second, and is static (0.9)
// under reduced motion, with `animated: false`, and with software rendering
// (where repainting whole screens would cost real CPU).
Item {
    id: root

    property bool active: false
    // False holds the pulse still.
    property bool animated: true
    // How far the glow reaches in from the screen edge, in grid units.
    property real depth: 1.4
    // The layer-shell scope (the surface's role name for the compositor).
    property string namespace: Qt.application.name + "-glow"
    // True when screen-edge windows can be made here.
    readonly property bool usable: onWayland ? layerLoader.status === Loader.Ready : onX11

    // Internal, read by the windows.
    readonly property real gridUnit: Kirigami.Units.gridUnit
    readonly property int extraFlags: onX11 ? (Qt.Tool | Qt.WindowStaysOnTopHint) : 0
    readonly property real level: _level
    readonly property var screens: Qt.application.screens
    readonly property var stripModel: {
        const m = [];
        for (const s of screens) {
            for (let p = 0; p < 4; ++p) {
                m.push({
                    "screen": s,
                    "part": p
                });
            }
        }
        return m;
    }

    readonly property bool onWayland: Qt.platform.pluginName.startsWith("wayland")
    readonly property bool onX11: Qt.platform.pluginName === "xcb"
    // TelamonStyle.softwareRendering covers the software scene graph and
    // software GL drivers (llvmpipe), and honours TELAMON_SOFTWARE_RENDERING=0/1.
    // This program never asks for the software scene graph, so it draws on
    // the GPU wherever there is a hardware GL driver.
    readonly property bool _software: TelamonStyle.softwareRendering === true
    // TelamonStyle.reducedMotion: the user's setting, or Plasma's animation
    // speed set to instant.
    readonly property bool _reducedMotion: TelamonStyle.reducedMotion === true
    readonly property bool _pulsing: animated && !_reducedMotion && !_software
    readonly property real _period: 2400
    readonly property real _static: 0.9

    // 0.6 to 1: the pulse (a cosine of the phase, brightest at phase 0).
    property real _level: _static
    property real _phase: 0
    on_PhaseChanged: _level = 0.8 + 0.2 * Math.cos(_phase * 2 * Math.PI)
    on_PulsingChanged: {
        if (!_pulsing) {
            _level = _static;
        }
    }

    // 30 frames per second (a NumberAnimation would run at the monitor rate,
    // and every frame recomposites whole screens).
    Timer {
        running: root.active && root.usable && root._pulsing
        interval: 33
        repeat: true
        triggeredOnStart: true
        onTriggered: root._phase = (Date.now() % root._period) / root._period
    }

    // Wayland: the layer-shell windows, loaded on start so that `usable` is
    // known; the windows themselves exist only while active.
    Loader {
        id: layerLoader
        active: root.onWayland
        source: "GlowLayer.qml"
        onLoaded: item.glow = root
    }

    // X11: plain tool windows, four strips per screen.
    Instantiator {
        active: root.active && root.onX11
        model: root.stripModel
        delegate: GlowWindow {
            required property var modelData
            glow: root
            part: modelData.part
            screen: modelData.screen
        }
    }
}
