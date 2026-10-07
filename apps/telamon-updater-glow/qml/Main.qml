import QtQuick

// What telamon-updater-glow shows: the glow around the edges of every screen,
// and nothing else. It runs only while the system is being changed; Telamon
// Updater's tray starts and ends it (apps/telamon-updater-tray).
Item {
    ScreenGlow {
        active: true
        onUsableChanged: if (!usable)
            console.warn("no screen-edge windows can be made on this platform: no glow")
    }
}
