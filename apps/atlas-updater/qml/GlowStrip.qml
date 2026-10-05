import QtQuick
import QtQuick.Window
import Atlas.Ui

// One strip of the screen-edge glow: a transparent, input-transparent window
// along one edge of one screen, drawn as a gradient from the accent colour at
// the screen edge to nothing inward. ScreenGlow makes four per screen (the
// top and bottom ones full width, the left and right ones full height with
// the corners left empty) and, on Wayland, GlowStripLayer turns them into
// layer-shell overlays.
Window {
    id: strip

    // The ScreenGlow that owns this strip (its depth, level and title).
    required property Item glow
    // 0 top, 1 bottom, 2 left, 3 right (physical, not mirrored).
    required property int edge

    readonly property bool horizontal: edge < 2 // a strip along the top or bottom
    readonly property int px: Math.max(1, Math.round(glow.depth * glow.gridUnit))

    title: Qt.application.displayName + " glow"
    color: "transparent"
    // Nothing is clickable or focusable: input goes to the window behind.
    // (ScreenGlow adds Tool and StaysOnTop on X11.)
    flags: Qt.FramelessWindowHint | Qt.WindowTransparentForInput | Qt.WindowDoesNotAcceptFocus | glow.extraFlags
    // Shown when the whole object is built, so that `screen` (and the layer
    // properties) are set before the window is mapped.
    visible: false
    Component.onCompleted: visible = true

    // `screen` is a Qt.application.screens entry: it has virtualX/virtualY
    // and width/height (in device-independent pixels), not a geometry.
    x: screen.virtualX + (edge === 3 ? screen.width - px : 0)
    y: screen.virtualY + (edge === 1 ? screen.height - px : 0)
    width: horizontal ? screen.width : px
    height: horizontal ? px : screen.height

    Item {
        anchors.fill: parent
        Accessible.ignored: true
        opacity: strip.glow.level

        Rectangle {
            id: band
            // The left and right strips leave the corners to the top and bottom ones.
            x: 0
            y: strip.horizontal ? 0 : strip.px
            width: parent.width
            height: strip.horizontal ? parent.height : parent.height - 2 * strip.px
            // Strongest at the screen edge: position 0 is the top or left.
            readonly property bool reversed: strip.edge === 1 || strip.edge === 3
            readonly property color tone: AtlasStyle.accent
            function at(p) {
                return reversed ? 1 - p : p;
            }
            gradient: Gradient {
                orientation: strip.horizontal ? Gradient.Vertical : Gradient.Horizontal
                GradientStop {
                    position: band.at(0)
                    color: Qt.alpha(band.tone, 0.6)
                }
                GradientStop {
                    position: band.at(0.4)
                    color: Qt.alpha(band.tone, 0.22)
                }
                GradientStop {
                    position: band.at(0.7)
                    color: Qt.alpha(band.tone, 0.06)
                }
                GradientStop {
                    position: band.at(1)
                    color: Qt.alpha(band.tone, 0)
                }
            }
        }
    }
}
