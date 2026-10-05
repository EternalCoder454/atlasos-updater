pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Atlas.Ui

AtlasWindow {
    id: root

    // Both come from Shell::openWindow() (setInitialProperties).
    required property var backend
    required property string startPage

    title: qsTr("Atlas Updater")
    width: Kirigami.Units.gridUnit * 52
    height: Kirigami.Units.gridUnit * 38
    minimumWidth: Kirigami.Units.gridUnit * 24
    minimumHeight: Kirigami.Units.gridUnit * 24
    // Remembers the size and maximised state; the width and height above
    // are the first-run default.
    stateKey: "main"
    visible: true

    LayoutMirroring.enabled: Qt.application.layoutDirection === Qt.RightToLeft
    LayoutMirroring.childrenInherit: true

    property string currentPage: ""
    // The sidebar item a sub-page belongs to: Sent reports opens from both
    // Settings and Crash reports, and keeps the one it came from selected.
    property string sentFrom: "reports"
    // When the app list was last checked, so revisiting Updates does not run a
    // Flatpak check every time.
    property double lastAppsCheck: 0
    // Icons only when the window is narrow. Not AtlasWindow.sidebarCollapsed:
    // that folds at 30 grid units, and this sidebar folds at 38.
    readonly property bool compact: width < Kirigami.Units.gridUnit * 38

    readonly property var pages: ({
            "updates": updatesPage,
            "rollback": rollbackPage,
            "channel": channelPage,
            "history": historyPage,
            "changelog": changelogPage,
            "settings": settingsPage,
            "reports": reportsPage,
            "sent": sentPage,
            "about": aboutPage
        })

    function showPage(name) {
        if (name === currentPage) {
            return;
        }
        if (name === "sent") {
            sentFrom = currentPage === "settings" ? "settings" : "reports";
        }
        var c = pages[name] ? pages[name] : updatesPage;
        currentPage = pages[name] ? name : "updates";
        if (stack.depth === 0) {
            stack.push(c, {}, QQC2.StackView.Immediate);
        } else {
            stack.replace(c);
        }
    }

    // A restart did not happen (asked for on the Go Back or channel page):
    // the Updates page explains it.
    Connections {
        target: root.backend
        function onRestartProblem(text) {
            if (root.active) {
                root.showPage("updates");
            }
        }
    }

    component NavItem: SidebarItem {
        required property string page
        Layout.fillWidth: true
        compact: root.compact
        selected: root.currentPage === page || (root.currentPage === "sent" && page === root.sentFrom)
        onClicked: {
            // A confirmation belongs to the section it came from.
            if (page !== root.currentPage) {
                root.backend.dismissInfo();
            }
            root.showPage(page);
        }
    }

    RowLayout {
        anchors.fill: parent
        spacing: 0

        Rectangle {
            id: sidebar
            Layout.fillHeight: true
            Layout.preferredWidth: root.compact ? Kirigami.Units.gridUnit * 3.6 : Kirigami.Units.gridUnit * 12.5
            color: root.sidebarColor(AtlasStyle.base)

            Behavior on Layout.preferredWidth {
                NumberAnimation {
                    duration: AtlasStyle.duration
                    easing.type: Easing.OutCubic
                }
            }

            Rectangle {
                anchors.right: parent.right
                height: parent.height
                width: 1
                color: AtlasStyle.separator
            }

            ColumnLayout {
                anchors.fill: parent
                anchors.margins: AtlasStyle.spacingLarge
                anchors.rightMargin: AtlasStyle.spacingLarge + 1
                anchors.topMargin: AtlasStyle.spacingXLarge
                spacing: AtlasStyle.spacingXSmall

                NavItem {
                    page: "updates"
                    text: qsTr("Updates")
                    icon.name: "update-none"
                }
                NavItem {
                    page: "rollback"
                    text: qsTr("Go Back")
                    icon.name: "edit-undo"
                }
                NavItem {
                    page: "channel"
                    text: qsTr("Channel")
                    icon.name: "network-wireless-hotspot"
                }
                NavItem {
                    page: "history"
                    text: qsTr("History")
                    icon.name: "view-history"
                }
                NavItem {
                    page: "changelog"
                    text: qsTr("Changelog")
                    icon.name: "view-list-text"
                }
                Item {
                    Layout.fillHeight: true
                }
                NavItem {
                    page: "settings"
                    text: qsTr("Settings")
                    icon.name: "configure"
                }
                NavItem {
                    page: "reports"
                    text: qsTr("Crash Reports")
                    // Warning colours only while reports wait for a decision.
                    icon.name: root.backend.reportsCount > 0 ? "data-warning" : "tools-report-bug"
                    tintIcon: root.backend.reportsCount === 0
                }
                NavItem {
                    page: "about"
                    text: qsTr("About")
                    icon.name: "help-about"
                }
            }
        }

        QQC2.StackView {
            id: stack
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true

            replaceEnter: Transition {
                ParallelAnimation {
                    NumberAnimation {
                        property: "opacity"
                        from: 0
                        to: 1
                        duration: AtlasStyle.durationLong
                        easing.type: Easing.OutCubic
                    }
                    NumberAnimation {
                        property: "y"
                        from: Kirigami.Units.gridUnit
                        to: 0
                        duration: AtlasStyle.durationLong
                        easing.type: Easing.OutCubic
                    }
                }
            }
            replaceExit: Transition {
                NumberAnimation {
                    property: "opacity"
                    from: 1
                    to: 0
                    duration: AtlasStyle.durationShort
                }
            }
        }
    }

    // "The system is doing something for you now": an OS update downloading,
    // staging or installing, or an app update running. Only while the system
    // is being changed: an update, a channel switch or a rollback being
    // staged, or apps being updated. Not for checks, status reads, errors or
    // idle (ops.rs names the operations). The one source of truth for both
    // glows below.
    readonly property bool glowActive: (root.backend.busy && ["download", "switch", "rollback"].indexOf(root.backend.busyOp) >= 0) || (root.backend.appsBusy && root.backend.appsOp === "updateApps") || (root.backend.firmwareBusy && root.backend.firmwareOp === "installFirmware")
    // The glow shows on the screens' edges, which outlast this window: Shell
    // keeps the window's QML alive (hidden) while this is true.
    readonly property bool glowOutlivesWindow: glowActive && screenGlow.usable
    // Set by Shell from the render thread's GL_RENDERER (llvmpipe, softpipe).
    // Goes when Atlas.Ui 1.5.0 (AtlasStyle.softwareRendering) is the minimum.
    property bool softwareGl: false

    // Around every screen's edges. It takes no input and has no windows or
    // timers while inactive.
    ScreenGlow {
        id: screenGlow
        active: root.glowActive
        softwareGl: root.softwareGl
    }
    // Where screen-edge windows cannot be made (no layer-shell, an unknown
    // platform): the glow along this window's edges instead. Last, so it draws
    // over the content; it takes no input.
    AtlasEdgeGlow {
        anchors.fill: parent
        active: root.glowActive && !screenGlow.usable
    }

    Component {
        id: updatesPage
        UpdatesPage {
            backend: root.backend
            lastAppsCheck: root.lastAppsCheck
            onAppsChecked: root.lastAppsCheck = Date.now()
            onOpenReports: root.showPage("reports")
            onOpenChangelog: root.showPage("changelog")
        }
    }
    Component {
        id: rollbackPage
        RollbackPage {
            backend: root.backend
        }
    }
    Component {
        id: channelPage
        ChannelPage {
            backend: root.backend
        }
    }
    Component {
        id: historyPage
        HistoryPage {
            backend: root.backend
        }
    }
    Component {
        id: changelogPage
        ChangelogPage {
            backend: root.backend
        }
    }
    Component {
        id: settingsPage
        SettingsPage {
            backend: root.backend
            onOpenReports: root.showPage("reports")
            onOpenSent: root.showPage("sent")
        }
    }
    Component {
        id: reportsPage
        ReportsPage {
            backend: root.backend
            onOpenSent: root.showPage("sent")
        }
    }
    Component {
        id: sentPage
        SentReportsPage {
            backend: root.backend
        }
    }
    Component {
        id: aboutPage
        AboutPage {
            backend: root.backend
        }
    }

    Component.onCompleted: showPage(startPage)
}
