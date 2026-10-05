pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Window
import org.kde.kirigami as Kirigami
import Atlas.Ui

// The "the system is being changed" glow, drawn around the edges of every
// screen (not inside one window), as four thin transparent strips per screen.
// Same meaning and API as AtlasEdgeGlow; to be replaced by it in Atlas.Ui
// 1.5.0 (AtlasScreenGlow), when this file goes.
//
//   Wayland: layer-shell overlays (GlowStripLayer, needs layer-shell-qt).
//   X11:     frameless, always-on-top tool windows.
//   Else, or without the layer-shell module: `usable` is false and the caller
//   shows the in-window AtlasEdgeGlow instead.
//
// While `active` is false there are no windows and no timer. The strips take
// no input and are hidden from screen readers. The glow pulses slowly (2.4 s)
// at 30 frames per second, and is static under reduced motion, with
// `animated: false`, and with software rendering (where a pulse across
// several screens would cost real CPU).
Item {
    id: root

    property bool active: false
    // False holds the pulse still.
    property bool animated: true
    // How far the glow reaches in from the screen edge, in grid units.
    property real depth: 3
    // The layer-shell scope (the surface's role name for the compositor).
    property string namespace: Qt.application.name + "-glow"
    // Set by the app: the main window's GL renderer is llvmpipe or softpipe
    // (a software GL driver). Goes when Atlas.Ui 1.5.0 is the minimum.
    property bool softwareGl: false

    // True when screen-edge windows can be made here; false means the caller
    // must show its in-window glow instead.
    readonly property bool usable: onWayland ? layerLoader.status === Loader.Ready : onX11

    // Internal, read by the strips.
    readonly property real gridUnit: Kirigami.Units.gridUnit
    readonly property int extraFlags: onX11 ? (Qt.Tool | Qt.WindowStaysOnTopHint) : 0
    readonly property real level: _level
    readonly property var stripModel: {
        const m = [];
        for (const s of Qt.application.screens) {
            for (let e = 0; e < 4; ++e) {
                m.push({
                    "screen": s,
                    "edge": e
                });
            }
        }
        return m;
    }

    readonly property bool onWayland: Qt.platform.pluginName.startsWith("wayland")
    readonly property bool onX11: Qt.platform.pluginName === "xcb"
    // AtlasStyle.softwareRendering exists from Atlas.Ui 1.5.0; before that,
    // the scene graph API and the GL renderer tell.
    readonly property bool _software: AtlasStyle["softwareRendering"] === true || GraphicsInfo.api === GraphicsInfo.Software || softwareGl
    readonly property bool _pulsing: animated && Kirigami.Units.longDuration > 0 && !_software
    readonly property real _period: 2400

    // 0.55 to 1: the pulse (a cosine of the phase); static is the middle.
    property real _level: 0.8
    property real _phase: 0
    on_PhaseChanged: _level = 0.775 + 0.225 * Math.cos(_phase * 2 * Math.PI)
    on_PulsingChanged: {
        if (!_pulsing) {
            _level = 0.8;
        }
    }

    // 30 frames per second (a NumberAnimation would run at the monitor rate).
    Timer {
        running: root.active && root.usable && root._pulsing
        interval: 33
        repeat: true
        onTriggered: root._phase = (Date.now() % root._period) / root._period
    }

    // Wayland: the layer-shell strips, loaded on start so that `usable` is
    // known; the windows themselves exist only while active.
    Loader {
        id: layerLoader
        active: root.onWayland
        source: "GlowStripLayer.qml"
        onLoaded: item.glow = root
    }

    // X11: plain tool windows.
    Instantiator {
        active: root.active && root.onX11
        model: root.stripModel
        delegate: GlowStrip {
            required property var modelData
            glow: root
            edge: modelData.edge
            screen: modelData.screen
        }
    }
}
