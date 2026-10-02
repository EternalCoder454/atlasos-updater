pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Atlas.Ui

QQC2.ApplicationWindow {
    id: root

    // Both come from Shell::openWindow() (setInitialProperties).
    required property var backend
    required property string startPage

    title: qsTr("Atlas Updater")
    width: Kirigami.Units.gridUnit * 52
    height: Kirigami.Units.gridUnit * 38
    minimumWidth: Kirigami.Units.gridUnit * 24
    minimumHeight: Kirigami.Units.gridUnit * 24
    visible: true
    color: Kirigami.Theme.backgroundColor

    LayoutMirroring.enabled: Qt.application.layoutDirection === Qt.RightToLeft
    LayoutMirroring.childrenInherit: true

    property string currentPage: ""
    // When the app list was last checked, so revisiting Updates does not run a
    // Flatpak check every time.
    property double lastAppsCheck: 0
    // Icons only when the window is narrow.
    readonly property bool compact: width < Kirigami.Units.gridUnit * 38

    readonly property var pages: ({
            "updates": updatesPage,
            "rollback": rollbackPage,
            "channel": channelPage,
            "history": historyPage,
            "settings": settingsPage,
            "reports": reportsPage,
            "sent": sentPage,
            "about": aboutPage
        })

    function showPage(name) {
        if (name === currentPage) {
            return;
        }
        var c = pages[name] ? pages[name] : updatesPage;
        currentPage = pages[name] ? name : "updates";
        if (stack.depth === 0) {
            stack.push(c, {}, QQC2.StackView.Immediate);
        } else {
            stack.replace(c);
        }
    }

    // A restart did not happen. The Updates page explains it; the shell
    // sends a notification instead when the window is not active.
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
        selected: root.currentPage === page || (page === "settings" && root.currentPage === "sent")
        QQC2.ToolTip.visible: compact && hovered
        QQC2.ToolTip.text: text
        QQC2.ToolTip.delay: Kirigami.Units.toolTipDelay
        onClicked: root.showPage(page)
    }

    RowLayout {
        anchors.fill: parent
        spacing: 0

        Rectangle {
            id: sidebar
            Layout.fillHeight: true
            Layout.preferredWidth: root.compact ? Kirigami.Units.gridUnit * 3.6 : Kirigami.Units.gridUnit * 12.5
            color: Qt.tint(Kirigami.Theme.backgroundColor, Qt.alpha(Kirigami.Theme.highlightColor, 0.07))

            Behavior on Layout.preferredWidth {
                NumberAnimation {
                    duration: Kirigami.Units.shortDuration
                    easing.type: Easing.OutCubic
                }
            }

            Rectangle {
                anchors.right: parent.right
                height: parent.height
                width: 1
                color: Qt.alpha(Kirigami.Theme.textColor, 0.12)
            }

            ColumnLayout {
                anchors.fill: parent
                anchors.margins: Kirigami.Units.largeSpacing
                anchors.rightMargin: Kirigami.Units.largeSpacing + 1
                anchors.topMargin: Kirigami.Units.gridUnit
                spacing: 2

                NavItem {
                    page: "updates"
                    text: qsTr("Updates")
                    icon.name: "update-none"
                }
                NavItem {
                    page: "rollback"
                    text: qsTr("Go back")
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
                    text: qsTr("Crash reports")
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
                        duration: Kirigami.Units.longDuration
                        easing.type: Easing.OutCubic
                    }
                    NumberAnimation {
                        property: "y"
                        from: Kirigami.Units.gridUnit
                        to: 0
                        duration: Kirigami.Units.longDuration
                        easing.type: Easing.OutCubic
                    }
                }
            }
            replaceExit: Transition {
                NumberAnimation {
                    property: "opacity"
                    from: 1
                    to: 0
                    duration: Kirigami.Units.shortDuration
                }
            }
        }
    }

    Component {
        id: updatesPage
        UpdatesPage {
            backend: root.backend
            lastAppsCheck: root.lastAppsCheck
            onAppsChecked: root.lastAppsCheck = Date.now()
            onOpenReports: root.showPage("reports")
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
