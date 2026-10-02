pragma ComponentBehavior: Bound

import QtQuick
import org.kde.kirigami as Kirigami

Kirigami.ApplicationWindow {
    id: root

    // Both come from Shell::openWindow() (setInitialProperties).
    required property var backend
    required property string startPage

    title: qsTr("Atlas Updater")
    width: Kirigami.Units.gridUnit * 48
    height: Kirigami.Units.gridUnit * 36
    minimumWidth: Kirigami.Units.gridUnit * 22
    minimumHeight: Kirigami.Units.gridUnit * 20

    property string currentPage: ""

    function showPage(name) {
        if (name === currentPage) {
            return;
        }
        currentPage = name;
        pageStack.clear();
        var c = {
            "updates": updatesPage,
            "rollback": rollbackPage,
            "channel": channelPage,
            "history": historyPage,
            "settings": settingsPage,
            "reports": reportsPage,
            "sent": sentPage,
            "about": aboutPage
        }[name];
        pageStack.push(c ? c : updatesPage);
    }

    globalDrawer: Kirigami.GlobalDrawer {
        id: drawer
        title: qsTr("Atlas Updater")
        titleIcon: "net.eterneon.atlas.updater"
        isMenu: false
        collapsible: true
        collapsed: false
        modal: false
        showHeaderWhenCollapsed: false

        actions: [
            Kirigami.Action {
                text: qsTr("Updates")
                icon.name: "update-none"
                checked: root.currentPage === "updates"
                onTriggered: root.showPage("updates")
            },
            Kirigami.Action {
                text: qsTr("Go back")
                icon.name: "edit-undo"
                checked: root.currentPage === "rollback"
                onTriggered: root.showPage("rollback")
            },
            Kirigami.Action {
                text: qsTr("Channel")
                icon.name: "network-wireless-hotspot"
                checked: root.currentPage === "channel"
                onTriggered: root.showPage("channel")
            },
            Kirigami.Action {
                text: qsTr("History")
                icon.name: "view-history"
                checked: root.currentPage === "history"
                onTriggered: root.showPage("history")
            },
            Kirigami.Action {
                text: qsTr("Settings")
                icon.name: "configure"
                checked: root.currentPage === "settings"
                onTriggered: root.showPage("settings")
            },
            Kirigami.Action {
                text: qsTr("Sent reports")
                icon.name: "mail-sent"
                checked: root.currentPage === "sent"
                onTriggered: root.showPage("sent")
            },
            Kirigami.Action {
                text: qsTr("About")
                icon.name: "help-about"
                checked: root.currentPage === "about"
                onTriggered: root.showPage("about")
            }
        ]
    }

    Component {
        id: updatesPage
        UpdatesPage {
            backend: root.backend
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
        }
    }
    Component {
        id: reportsPage
        ReportsPage {
            backend: root.backend
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
