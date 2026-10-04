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

    title: qsTr("Sent Reports")

    readonly property var sent: page.backend.sentJson.length > 0 ? JSON.parse(page.backend.sentJson) : []

    Component.onCompleted: backend.loadSentReports()

    ConfirmDialog {
        id: payloadDialog
        property string payload: ""
        title: qsTr("What Was Sent")
        acceptText: qsTr("Close")
        showReject: false
        width: Math.min(parent ? parent.width - Kirigami.Units.gridUnit * 2 : 0, Kirigami.Units.gridUnit * 36)

        QQC2.ScrollView {
            Layout.fillWidth: true
            Layout.preferredHeight: Math.min(Kirigami.Units.gridUnit * 18, payloadDialog.parent ? payloadDialog.parent.height * 0.5 : 100)
            AtlasTextArea {
                readOnly: true
                text: payloadDialog.payload
                font: Kirigami.Theme.fixedWidthFont
                wrapMode: TextEdit.NoWrap
                Accessible.name: qsTr("Sent data")
            }
        }
    }

    AtlasEmptyState {
        Layout.fillWidth: true
        Layout.topMargin: Kirigami.Units.gridUnit * 3
        visible: page.sent.length === 0
        iconName: "mail-sent"
        title: qsTr("No reports sent")
        text: qsTr("Reports you send are listed here for 90 days.")
    }

    Section {
        visible: page.sent.length > 0
        Layout.bottomMargin: Kirigami.Units.largeSpacing
        Repeater {
            model: page.sent
            delegate: SectionRow {
                id: row
                required property var modelData
                title: row.modelData.appName + " " + row.modelData.appVersion
                subtitle: Dates.longDate(row.modelData.time) + " · AtlasOS " + row.modelData.atlasosVersion + (row.modelData.sentEventId ? " · " + qsTr("event %1").arg(row.modelData.sentEventId) : "")
                chevron: true
                onClicked: {
                    payloadDialog.payload = row.modelData.payload;
                    payloadDialog.open();
                }
                // A trailing item of the row, so Section's separators and
                // corners still see plain SectionRows.
                TextButton {
                    text: qsTr("View on GitHub")
                    visible: row.modelData.issueUrl.length > 0 && page.backend.isSafeLink(row.modelData.issueUrl)
                    onClicked: Qt.openUrlExternally(row.modelData.issueUrl)
                }
            }
        }
    }
}
