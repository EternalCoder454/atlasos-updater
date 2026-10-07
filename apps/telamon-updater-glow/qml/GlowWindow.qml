import QtQuick
import QtQuick.Window

// A window of the screen-edge glow: transparent, input-transparent, showing
// the GlowFrame of one screen. On Wayland there is one per screen, covering
// all of it (`part` -1; GlowLayer makes it a layer-shell overlay), so the
// frame is drawn in one piece. On X11 there are four per screen, one strip
// along each edge (`part` 0 to 3), each showing its part of the same
// screen-sized frame, so they still meet without a seam: there a full-screen
// window would turn the whole screen black without a compositor.
Window {
    id: win

    // The ScreenGlow that owns this window (its depth, level and flags).
    required property ScreenGlow glow
    // -1 the whole screen; 0 top, 1 bottom, 2 left, 3 right (physical, not
    // mirrored): the top and bottom strips are full width, the left and right
    // ones fit between them.
    property int part: -1

    readonly property int px: Math.max(1, Math.round(glow.depth * glow.gridUnit))
    // This window's place on its screen (device-independent pixels).
    readonly property rect area: {
        const w = screen.width;
        const h = screen.height;
        switch (part) {
        case 0:
            return Qt.rect(0, 0, w, px);
        case 1:
            return Qt.rect(0, h - px, w, px);
        case 2:
            return Qt.rect(0, px, px, h - 2 * px);
        case 3:
            return Qt.rect(w - px, px, px, h - 2 * px);
        default:
            return Qt.rect(0, 0, w, h);
        }
    }

    title: Qt.application.displayName + " glow"
    color: "transparent"
    // Nothing is clickable or focusable: input goes to the window behind
    // (on Wayland, Qt gives the surface an empty input region). ScreenGlow
    // adds Tool and StaysOnTop on X11.
    flags: Qt.FramelessWindowHint | Qt.WindowTransparentForInput | Qt.WindowDoesNotAcceptFocus | glow.extraFlags
    // Shown when the whole object is built, so that `screen` (and the layer
    // properties) are set before the window is mapped.
    visible: false
    Component.onCompleted: visible = true

    // `screen` is a Qt.application.screens entry: it has virtualX/virtualY
    // and width/height (in device-independent pixels), not a geometry. A
    // layer-shell compositor sizes the whole-screen window itself.
    x: screen.virtualX + area.x
    y: screen.virtualY + area.y
    width: area.width
    height: area.height

    GlowFrame {
        // The whole screen's frame, moved so that this window shows its part.
        x: win.part < 0 ? 0 : -win.area.x
        y: win.part < 0 ? 0 : -win.area.y
        width: win.part < 0 ? win.width : win.screen.width
        height: win.part < 0 ? win.height : win.screen.height
        depth: win.px
        // The pulse: one opacity, nothing redrawn.
        opacity: win.glow.level
    }
}
