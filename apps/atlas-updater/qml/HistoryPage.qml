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
    readonly property var appEntries: page.backend.appHistoryJson.length > 0 ? JSON.parse(page.backend.appHistoryJson) : []

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
        visible: page.entries.length === 0 && page.appEntries.length === 0
        icon.name: "view-history"
        text: qsTr("No history yet")
        explanation: qsTr("Each version this computer starts, and each app update, is listed here, newest first.")
    }

    Section {
        visible: page.entries.length > 0
        title: qsTr("AtlasOS")
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

    Section {
        visible: page.appEntries.length > 0
        title: qsTr("App Updates")
        Layout.bottomMargin: Kirigami.Units.largeSpacing
        Repeater {
            model: page.appEntries
            delegate: SectionRow {
                id: appRow
                required property var modelData
                iconName: appRow.modelData.runtime ? "preferences-system-plugin" : "applications-all"
                title: appRow.modelData.to ? qsTr("%1 %2").arg(appRow.modelData.name).arg(appRow.modelData.to) : appRow.modelData.name
                subtitle: {
                    var when = Dates.longDate(new Date(appRow.modelData.at * 1000).toISOString());
                    var t = appRow.modelData.auto ? qsTr("Updated in the background on %1").arg(when) : qsTr("Updated on %1").arg(when);
                    if (appRow.modelData.from && appRow.modelData.to && appRow.modelData.from !== appRow.modelData.to) {
                        t += " · " + qsTr("was %1").arg(appRow.modelData.from);
                    }
                    return t;
                }
            }
        }
    }
}
