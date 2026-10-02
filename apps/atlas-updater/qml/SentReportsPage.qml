pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import "dates.js" as Dates

Kirigami.ScrollablePage {
    id: page

    required property var backend

    title: qsTr("Sent reports")

    readonly property var sent: page.backend.sentJson.length > 0 ? JSON.parse(page.backend.sentJson) : []

    Component.onCompleted: backend.loadSentReports()

    Kirigami.Dialog {
        id: payloadDialog
        property string payload: ""
        title: qsTr("What was sent")
        standardButtons: Kirigami.Dialog.Close
        preferredWidth: Kirigami.Units.gridUnit * 32
        preferredHeight: Kirigami.Units.gridUnit * 24
        QQC2.ScrollView {
            QQC2.TextArea {
                readOnly: true
                text: payloadDialog.payload
                font.family: "monospace"
                wrapMode: TextEdit.NoWrap
                Accessible.name: qsTr("Sent data")
            }
        }
    }

    Kirigami.PlaceholderMessage {
        anchors.centerIn: parent
        width: parent.width - Kirigami.Units.gridUnit * 4
        visible: page.sent.length === 0
        icon.name: "mail-sent"
        text: qsTr("No reports sent")
        explanation: qsTr("Reports you send are listed here for 90 days.")
    }

    ListView {
        model: page.sent
        delegate: QQC2.ItemDelegate {
            id: row
            required property var modelData
            width: ListView.view.width
            onClicked: {
                payloadDialog.payload = row.modelData.payload;
                payloadDialog.open();
            }
            contentItem: ColumnLayout {
                spacing: 0
                QQC2.Label {
                    Layout.fillWidth: true
                    text: row.modelData.appName + " " + row.modelData.appVersion
                    elide: Text.ElideRight
                }
                QQC2.Label {
                    Layout.fillWidth: true
                    opacity: 0.7
                    font: Kirigami.Theme.smallFont
                    elide: Text.ElideRight
                    text: Dates.longDate(row.modelData.time) + " · AtlasOS " + row.modelData.atlasosVersion + (row.modelData.sentEventId ? " · " + qsTr("event %1").arg(row.modelData.sentEventId) : "")
                }
            }
        }
    }
}
