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

    signal openSent

    title: qsTr("Crash reports")

    readonly property var reports: page.backend.reportsJson.length > 0 ? JSON.parse(page.backend.reportsJson) : []

    Component.onCompleted: backend.loadReports()

    Cards {
        Layout.fillWidth: true
        backend: page.backend
    }

    Kirigami.PlaceholderMessage {
        Layout.fillWidth: true
        Layout.topMargin: Kirigami.Units.gridUnit * 3
        visible: page.reports.length === 0
        icon.name: "tools-report-bug"
        text: qsTr("No crash reports waiting")
        explanation: qsTr("When something crashes, the report shows up here and nothing is sent unless you say so.")
    }

    Repeater {
        model: page.reports
        delegate: ColumnLayout {
            id: card
            required property var modelData
            required property int index
            property bool showPayload: false
            Layout.fillWidth: true
            spacing: Kirigami.Units.largeSpacing

            Section {
                title: qsTr("%1 %2").arg(card.modelData.appName).arg(card.modelData.appVersion)
                footer: card.modelData.message

                SectionRow {
                    title: qsTr("Category")
                    value: card.modelData.category + " · " + card.modelData.type
                }
                SectionRow {
                    title: qsTr("When")
                    value: Dates.longDate(card.modelData.time)
                }
                SectionRow {
                    title: qsTr("AtlasOS")
                    value: card.modelData.atlasosVersion + (card.modelData.channel ? " (" + card.modelData.channel + ")" : "")
                }
                SectionRow {
                    title: qsTr("Previous version")
                    value: card.modelData.previousVersion || qsTr("none")
                }
                SectionRow {
                    title: qsTr("Kernel")
                    value: card.modelData.kernel
                }
                SectionRow {
                    title: qsTr("Graphics")
                    value: card.modelData.gpu + (card.modelData.gpuDriver ? " · " + card.modelData.gpuDriver : "")
                }
                SectionRow {
                    title: qsTr("Uptime")
                    value: card.modelData.uptime
                }
            }

            Section {
                title: qsTr("Stack trace")
                visible: card.modelData.stacktrace.length > 0
                Item {
                    Layout.fillWidth: true
                    implicitHeight: Kirigami.Units.gridUnit * 10
                    QQC2.ScrollView {
                        anchors.fill: parent
                        anchors.margins: Kirigami.Units.smallSpacing
                        QQC2.TextArea {
                            readOnly: true
                            text: card.modelData.stacktrace
                            font: Kirigami.Theme.fixedWidthFont
                            wrapMode: TextEdit.NoWrap
                            background: null
                            padding: Kirigami.Units.smallSpacing
                            Accessible.name: qsTr("Stack trace")
                        }
                    }
                }
            }

            Section {
                SectionRow {
                    title: card.showPayload ? qsTr("Hide exact data") : qsTr("Show exact data")
                    subtitle: qsTr("Exactly what would be sent")
                    chevron: true
                    disclosure: true
                    expanded: card.showPayload
                    onClicked: card.showPayload = !card.showPayload
                }
                Item {
                    visible: card.showPayload
                    Layout.fillWidth: true
                    implicitHeight: Kirigami.Units.gridUnit * 14
                    QQC2.ScrollView {
                        anchors.fill: parent
                        anchors.margins: Kirigami.Units.smallSpacing
                        QQC2.TextArea {
                            readOnly: true
                            text: card.modelData.payload
                            font: Kirigami.Theme.fixedWidthFont
                            wrapMode: TextEdit.NoWrap
                            background: null
                            padding: Kirigami.Units.smallSpacing
                            Accessible.name: qsTr("Exact data")
                        }
                    }
                }
            }

            Flow {
                Layout.fillWidth: true
                spacing: Kirigami.Units.largeSpacing
                PrimaryButton {
                    text: qsTr("Send")
                    enabled: !page.backend.busy
                    onClicked: page.backend.sendReport(card.modelData.eventId)
                }
                SecondaryButton {
                    text: qsTr("Don't send")
                    enabled: !page.backend.busy
                    onClicked: page.backend.discardReport(card.modelData.eventId)
                }
                TextButton {
                    text: qsTr("Report on GitHub instead")
                    visible: card.modelData.githubUrl.length > 0 && page.backend.isSafeLink(card.modelData.githubUrl)
                    enabled: !page.backend.busy
                    onClicked: {
                        Qt.openUrlExternally(card.modelData.githubUrl);
                        page.backend.discardReport(card.modelData.eventId);
                    }
                }
            }
        }
    }

    Section {
        Layout.bottomMargin: Kirigami.Units.largeSpacing
        SectionRow {
            title: qsTr("Sent reports")
            chevron: true
            onClicked: page.openSent()
        }
    }
}
