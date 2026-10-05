pragma ComponentBehavior: Bound

import QtQuick
import org.kde.layershell as LayerShell

// The Wayland strips: GlowStrip windows made layer-shell overlays. This is the
// only file that imports org.kde.layershell, and ScreenGlow loads it through a
// Loader, so a missing module (Loader.Error) leaves the in-window glow instead
// of breaking the app.
Instantiator {
    id: layer

    // The ScreenGlow, set by its Loader.
    property Item glow: null

    active: glow !== null && glow.active
    model: glow ? glow.stripModel : []
    delegate: GlowStrip {
        required property var modelData
        glow: layer.glow
        edge: modelData.edge
        screen: modelData.screen

        // Above everything, on the screen's edge, not moved by panels, no keys.
        LayerShell.Window.layer: LayerShell.Window.LayerOverlay
        LayerShell.Window.scope: layer.glow.namespace
        LayerShell.Window.exclusionZone: -1
        LayerShell.Window.keyboardInteractivity: LayerShell.Window.KeyboardInteractivityNone
        LayerShell.Window.activateOnShow: false
        LayerShell.Window.anchors: {
            const W = LayerShell.Window;
            switch (edge) {
            case 0:
                return W.AnchorTop | W.AnchorLeft | W.AnchorRight;
            case 1:
                return W.AnchorBottom | W.AnchorLeft | W.AnchorRight;
            case 2:
                return W.AnchorLeft | W.AnchorTop | W.AnchorBottom;
            default:
                return W.AnchorRight | W.AnchorTop | W.AnchorBottom;
            }
        }
    }
}
