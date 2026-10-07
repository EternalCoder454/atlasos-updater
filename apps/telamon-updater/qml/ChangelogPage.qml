pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Telamon.Ui
import "dates.js" as Dates

// What changed in every version this computer went through (and the one
// waiting or offered), newest first, with each version's release notes.
TelamonPage {
    id: page

    required property var backend

    title: qsTr("Changelog")

    readonly property var items: page.backend.changelogJson.length > 0 ? JSON.parse(page.backend.changelogJson) : []
    readonly property string loadState: page.backend.changelogState
    // Versions the user opened or closed; the newest starts open.
    property var toggled: ({})

    function isOpen(version, index) {
        var t = page.toggled[version] === true;
        return index === 0 ? !t : t;
    }
    function toggle(version) {
        var t = Object.assign({}, page.toggled);
        t[version] = !(t[version] === true);
        page.toggled = t;
    }

    function badge(item) {
        switch (item.state) {
        case "current":
            return qsTr("Running now");
        case "staged":
            return qsTr("Downloaded, starts after a restart");
        case "available":
            return qsTr("Available to download");
        case "ran":
            return qsTr("Ran on this computer from %1").arg(Dates.longDate(item.first_booted));
        default:
            return qsTr("Came with a later update");
        }
    }

    function icon(item) {
        switch (item.state) {
        case "current":
            return "checkmark";
        case "staged":
            return "system-reboot";
        case "available":
            return "update-medium";
        default:
            return "view-history";
        }
    }

    Component.onCompleted: backend.loadChangelog()

    // One status read can change all three versions: one reload for them.
    Connections {
        target: page.backend
        function onCurrentVersionChanged() {
            Qt.callLater(page.backend.loadChangelog);
        }
        function onStagedVersionChanged() {
            Qt.callLater(page.backend.loadChangelog);
        }
        function onAvailableVersionChanged() {
            Qt.callLater(page.backend.loadChangelog);
        }
    }

    TelamonEmptyState {
        Layout.fillWidth: true
        Layout.topMargin: Kirigami.Units.gridUnit * 3
        visible: page.items.length === 0 && (page.loadState === "loading" || page.loadState === "")
        iconName: "view-list-text"
        title: qsTr("Loading the changelog…")
    }

    TelamonEmptyState {
        Layout.fillWidth: true
        Layout.topMargin: Kirigami.Units.gridUnit * 3
        visible: page.items.length === 0 && page.loadState === "error"
        iconName: "dialog-error"
        title: qsTr("Could not load the changelog")
        text: page.backend.changelogNote.length > 0 ? page.backend.changelogNote : qsTr("Check your internet connection.")
        actionText: qsTr("Try Again")
        actionSymbol: Symbols.Refresh
        onTriggered: page.backend.loadChangelog()
    }

    TelamonEmptyState {
        Layout.fillWidth: true
        Layout.topMargin: Kirigami.Units.gridUnit * 3
        visible: page.items.length === 0 && page.loadState === "ready"
        iconName: "view-list-text"
        title: qsTr("No versions to show yet")
        text: qsTr("Each version this computer starts, and the one waiting for it, is listed here with its release notes.")
    }

    // A list saved earlier, shown while offline.
    QQC2.Label {
        Layout.fillWidth: true
        visible: page.items.length > 0 && page.backend.changelogNote.length > 0
        text: page.backend.changelogNote
        wrapMode: Text.Wrap
        opacity: 0.7
        textFormat: Text.PlainText
    }

    Repeater {
        model: page.items
        delegate: Section {
            id: card
            required property var modelData
            required property int index
            readonly property bool open: page.isOpen(card.modelData.version, card.index)
            Layout.bottomMargin: card.index === page.items.length - 1 ? Kirigami.Units.largeSpacing : 0

            SectionRow {
                iconName: page.icon(card.modelData)
                title: qsTr("Telamon OS %1").arg(card.modelData.version)
                subtitle: {
                    var t = page.badge(card.modelData);
                    if (card.modelData.date.length > 0) {
                        t += " · " + qsTr("Released %1").arg(Dates.longDate(card.modelData.date));
                    }
                    return t;
                }
                chevron: true
                disclosure: true
                expanded: card.open
                Accessible.name: title + ", " + subtitle
                onClicked: page.toggle(card.modelData.version)
            }
            Item {
                visible: card.open
                Layout.fillWidth: true
                implicitHeight: (card.modelData.html.length > 0 ? notes.implicitHeight : none.implicitHeight) + Kirigami.Units.largeSpacing * 2
                NotesText {
                    id: notes
                    visible: card.modelData.html.length > 0
                    anchors.fill: parent
                    anchors.margins: Kirigami.Units.largeSpacing
                    html: card.modelData.html
                    plain: card.modelData.plain
                    onLinkClicked: link => {
                        if (page.backend.isSafeLink(link)) {
                            Qt.openUrlExternally(link);
                        }
                    }
                }
                QQC2.Label {
                    id: none
                    visible: card.modelData.html.length === 0
                    anchors.fill: parent
                    anchors.margins: Kirigami.Units.largeSpacing
                    text: qsTr("No release notes for this version.")
                    opacity: 0.7
                    wrapMode: Text.Wrap
                }
            }
        }
    }
}
