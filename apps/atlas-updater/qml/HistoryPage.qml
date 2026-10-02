pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import "dates.js" as Dates

Kirigami.ScrollablePage {
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
        anchors.centerIn: parent
        width: parent.width - Kirigami.Units.gridUnit * 4
        visible: page.entries.length === 0
        icon.name: "view-history"
        text: qsTr("No history yet")
        explanation: qsTr("Each version this computer starts is listed here, newest first.")
    }

    ListView {
        id: list
        model: page.entries
        delegate: QQC2.ItemDelegate {
            id: row
            required property var modelData
            width: ListView.view.width
            hoverEnabled: false
            down: false
            contentItem: RowLayout {
                spacing: Kirigami.Units.largeSpacing
                Kirigami.Icon {
                    source: row.modelData.current ? "emblem-checked" : "view-history"
                    Layout.preferredWidth: Kirigami.Units.iconSizes.smallMedium
                    Layout.preferredHeight: Kirigami.Units.iconSizes.smallMedium
                }
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 0
                    QQC2.Label {
                        Layout.fillWidth: true
                        text: row.modelData.current ? qsTr("%1 (running now)").arg(row.modelData.version) : row.modelData.version
                        font.bold: row.modelData.current
                        elide: Text.ElideRight
                    }
                    QQC2.Label {
                        Layout.fillWidth: true
                        opacity: 0.7
                        font: Kirigami.Theme.smallFont
                        elide: Text.ElideRight
                        text: {
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
    }
}
