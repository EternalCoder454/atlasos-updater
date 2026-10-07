pragma ComponentBehavior: Bound

import QtQuick
import org.kde.layershell as LayerShell

// The Wayland glow: one GlowWindow per screen, made a full-screen layer-shell
// overlay. This is the only file that imports org.kde.layershell, and
// ScreenGlow loads it through a Loader, so a missing module (Loader.Error)
// leaves the in-window glow instead of breaking the app.
Instantiator {
    id: layer

    // The ScreenGlow, set by its Loader.
    property ScreenGlow glow: null

    active: glow !== null && glow.active
    model: glow ? glow.screens : []
    delegate: GlowWindow {
        required property var modelData
        glow: layer.glow
        screen: modelData

        // Above everything, over the whole screen (anchored to all four
        // edges, so the compositor gives it the screen's size), not moved by
        // panels and not moving them, no keys.
        LayerShell.Window.layer: LayerShell.Window.LayerOverlay
        LayerShell.Window.scope: layer.glow.namespace
        LayerShell.Window.exclusionZone: -1
        LayerShell.Window.keyboardInteractivity: LayerShell.Window.KeyboardInteractivityNone
        LayerShell.Window.activateOnShow: false
        LayerShell.Window.anchors: LayerShell.Window.AnchorTop | LayerShell.Window.AnchorBottom | LayerShell.Window.AnchorLeft | LayerShell.Window.AnchorRight
    }
}
