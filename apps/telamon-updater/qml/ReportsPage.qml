pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Telamon.Ui
import "dates.js" as Dates

TelamonPage {
    id: page

    required property var backend

    signal openSent

    title: qsTr("Crash Reports")

    readonly property var reports: page.backend.reportsJson.length > 0 ? JSON.parse(page.backend.reportsJson) : []

    Component.onCompleted: backend.loadReports()

    Cards {
        Layout.fillWidth: true
        backend: page.backend
    }

    TelamonEmptyState {
        Layout.fillWidth: true
        Layout.topMargin: Kirigami.Units.gridUnit * 3
        visible: page.reports.length === 0
        iconName: "tools-report-bug"
        title: qsTr("No crash reports waiting")
        text: qsTr("When something crashes, the report shows up here and nothing is sent unless you say so.")
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
                    title: qsTr("Telamon OS")
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
                title: qsTr("Stack Trace")
                visible: card.modelData.stacktrace.length > 0
                TelamonCodeView {
                    Layout.fillWidth: true
                    // Text lines up with the rows' text (SectionRow pads 12).
                    Layout.leftMargin: TelamonStyle.spacingLarge
                    Layout.rightMargin: TelamonStyle.spacingSmall
                    Layout.topMargin: TelamonStyle.spacingSmall
                    Layout.bottomMargin: TelamonStyle.spacingSmall
                    framed: false
                    showCopy: true
                    maximumHeight: Kirigami.Units.gridUnit * 10
                    // Telamon.Ui 1.4.0 draws the sideways scroll bar of a long
                    // line over the last line, even scrolled to the end: end
                    // on an empty line for it to cover (until Telamon.Ui 1.5.0).
                    text: card.modelData.stacktrace.replace(/\n?$/, "\n")
                    Accessible.name: qsTr("Stack Trace")
                }
            }

            Section {
                SectionRow {
                    title: card.showPayload ? qsTr("Hide Exact Data") : qsTr("Show Exact Data")
                    subtitle: qsTr("Exactly what would be sent")
                    chevron: true
                    disclosure: true
                    expanded: card.showPayload
                    onClicked: card.showPayload = !card.showPayload
                }
                TelamonCodeView {
                    visible: card.showPayload
                    Layout.fillWidth: true
                    // Text lines up with the rows' text (SectionRow pads 12).
                    Layout.leftMargin: TelamonStyle.spacingLarge
                    Layout.rightMargin: TelamonStyle.spacingSmall
                    Layout.topMargin: TelamonStyle.spacingSmall
                    Layout.bottomMargin: TelamonStyle.spacingSmall
                    framed: false
                    showCopy: true
                    maximumHeight: Kirigami.Units.gridUnit * 14
                    // An empty last line for the scroll bar (see above).
                    text: card.modelData.payload.replace(/\n?$/, "\n")
                    Accessible.name: qsTr("Exact data")
                }
            }

            QQC2.Label {
                Layout.fillWidth: true
                Layout.leftMargin: Kirigami.Units.largeSpacing
                text: qsTr("Sending posts this report as a public issue on GitHub. Anyone can read it, including the stack trace and your Telamon OS version, kernel, CPU, GPU and memory.")
                color: Kirigami.Theme.disabledTextColor
                font: Kirigami.Theme.smallFont
                wrapMode: Text.WordWrap
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
                    variant: TelamonButton.Destructive
                    text: qsTr("Don't Send")
                    enabled: !page.backend.busy
                    onClicked: page.backend.discardReport(card.modelData.eventId)
                }
                TextButton {
                    text: qsTr("Report on GitHub Instead")
                    visible: card.modelData.githubUrl.length > 0 && page.backend.isSafeLink(card.modelData.githubUrl)
                    enabled: !page.backend.busy
                    onClicked: {
                        // Discard only if a browser really opened.
                        if (Qt.openUrlExternally(card.modelData.githubUrl)) {
                            page.backend.discardReport(card.modelData.eventId);
                        }
                    }
                }
            }
        }
    }

    Section {
        Layout.bottomMargin: Kirigami.Units.largeSpacing
        SectionRow {
            title: qsTr("Sent Reports")
            chevron: true
            onClicked: page.openSent()
        }
    }
}
