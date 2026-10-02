pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import "dates.js" as Dates

Kirigami.ScrollablePage {
    id: page

    required property var backend

    title: qsTr("Crash reports")

    readonly property var reports: page.backend.reportsJson.length > 0 ? JSON.parse(page.backend.reportsJson) : []

    Component.onCompleted: backend.loadReports()

    Kirigami.PlaceholderMessage {
        anchors.centerIn: parent
        width: parent.width - Kirigami.Units.gridUnit * 4
        visible: page.reports.length === 0
        icon.name: "tools-report-bug"
        text: qsTr("No crash reports waiting")
    }

    ColumnLayout {
        spacing: Kirigami.Units.largeSpacing

        Cards {
            Layout.fillWidth: true
            backend: page.backend
        }

        Repeater {
            model: page.reports
            delegate: Kirigami.AbstractCard {
                id: card
                required property var modelData
                required property int index
                property bool showPayload: false
                Layout.fillWidth: true
                contentItem: ColumnLayout {
                    spacing: Kirigami.Units.smallSpacing
                    Kirigami.Heading {
                        level: 3
                        Layout.fillWidth: true
                        wrapMode: Text.Wrap
                        text: qsTr("%1 %2").arg(card.modelData.appName).arg(card.modelData.appVersion)
                    }
                    QQC2.Label {
                        Layout.fillWidth: true
                        wrapMode: Text.Wrap
                        text: card.modelData.message
                    }
                    Kirigami.FormLayout {
                        Layout.fillWidth: true
                        wideMode: true
                        QQC2.Label {
                            Kirigami.FormData.label: qsTr("Category:")
                            text: card.modelData.category + " · " + card.modelData.type
                        }
                        QQC2.Label {
                            Kirigami.FormData.label: qsTr("When:")
                            text: Dates.longDate(card.modelData.time)
                        }
                        QQC2.Label {
                            Kirigami.FormData.label: qsTr("AtlasOS:")
                            text: card.modelData.atlasosVersion + (card.modelData.channel ? " (" + card.modelData.channel + ")" : "")
                        }
                        QQC2.Label {
                            Kirigami.FormData.label: qsTr("Previous version:")
                            text: card.modelData.previousVersion || qsTr("none")
                        }
                        QQC2.Label {
                            Kirigami.FormData.label: qsTr("Kernel:")
                            text: card.modelData.kernel
                        }
                        QQC2.Label {
                            Kirigami.FormData.label: qsTr("Graphics:")
                            text: card.modelData.gpu + (card.modelData.gpuDriver ? " · " + card.modelData.gpuDriver : "")
                            wrapMode: Text.Wrap
                            Layout.fillWidth: true
                        }
                        QQC2.Label {
                            Kirigami.FormData.label: qsTr("Uptime:")
                            text: card.modelData.uptime
                        }
                    }
                    QQC2.Label {
                        text: qsTr("Stack trace")
                        font.bold: true
                        visible: card.modelData.stacktrace.length > 0
                    }
                    QQC2.ScrollView {
                        Layout.fillWidth: true
                        Layout.preferredHeight: Kirigami.Units.gridUnit * 9
                        visible: card.modelData.stacktrace.length > 0
                        QQC2.TextArea {
                            readOnly: true
                            text: card.modelData.stacktrace
                            font.family: "monospace"
                            wrapMode: TextEdit.NoWrap
                            Accessible.name: qsTr("Stack trace")
                        }
                    }
                    QQC2.Button {
                        text: card.showPayload ? qsTr("Hide exact data") : qsTr("Show exact data")
                        icon.name: "view-list-text"
                        flat: true
                        onClicked: card.showPayload = !card.showPayload
                    }
                    QQC2.ScrollView {
                        Layout.fillWidth: true
                        Layout.preferredHeight: Kirigami.Units.gridUnit * 14
                        visible: card.showPayload
                        QQC2.TextArea {
                            readOnly: true
                            text: card.modelData.payload
                            font.family: "monospace"
                            wrapMode: TextEdit.NoWrap
                            Accessible.name: qsTr("Exact data")
                        }
                    }
                    Flow {
                        Layout.fillWidth: true
                        spacing: Kirigami.Units.smallSpacing
                        QQC2.Button {
                            text: qsTr("Send")
                            icon.name: "mail-send"
                            highlighted: true
                            enabled: !page.backend.busy
                            onClicked: page.backend.sendReport(card.index)
                        }
                        QQC2.Button {
                            text: qsTr("Don't send")
                            icon.name: "edit-delete"
                            enabled: !page.backend.busy
                            onClicked: page.backend.discardReport(card.index)
                        }
                        QQC2.Button {
                            text: qsTr("Report on GitHub instead")
                            icon.name: "internet-services"
                            visible: card.modelData.githubUrl.length > 0
                            onClicked: {
                                Qt.openUrlExternally(card.modelData.githubUrl);
                                page.backend.discardReport(card.index);
                            }
                        }
                    }
                }
            }
        }
    }
}
