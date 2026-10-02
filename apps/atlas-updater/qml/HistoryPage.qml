pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Atlas.Ui
import "dates.js" as Dates

AtlasPage {
    id: page

    required property var backend

    title: qsTr("History")

    readonly property var entries: page.backend.historyJson.length > 0 ? JSON.parse(page.backend.historyJson) : []

    Component.onCompleted: backend.loadHistory()

    Connections {
        target: page.backend
        function onCurrentVersionChanged() {
            page.backend.loadHistory();
        }
    }

    Kirigami.PlaceholderMessage {
        Layout.fillWidth: true
        Layout.topMargin: Kirigami.Units.gridUnit * 3
        visible: page.entries.length === 0
        icon.name: "view-history"
        text: qsTr("No history yet")
        explanation: qsTr("Each version this computer starts is listed here, newest first.")
    }

    Section {
        visible: page.entries.length > 0
        Layout.bottomMargin: Kirigami.Units.largeSpacing
        Repeater {
            model: page.entries
            delegate: SectionRow {
                id: row
                required property var modelData
                iconName: row.modelData.current ? "checkmark" : "view-history"
                title: row.modelData.current ? qsTr("%1 (running now)").arg(row.modelData.version) : row.modelData.version
                subtitle: {
                    var t = qsTr("First started %1").arg(Dates.longDate(row.modelData.booted));
                    if (row.modelData.channel.length > 0) {
                        t += " · " + row.modelData.channel;
                    }
                    return t;
                }
            }
        }
    }
}
