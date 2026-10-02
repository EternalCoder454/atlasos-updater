import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

Kirigami.ScrollablePage {
    id: page

    required property var backend

    title: qsTr("Settings")

    Component.onCompleted: backend.loadReports()

    Kirigami.Dialog {
        id: collected
        title: qsTr("What's collected")
        standardButtons: Kirigami.Dialog.Close
        padding: Kirigami.Units.largeSpacing
        preferredWidth: Kirigami.Units.gridUnit * 28
        ColumnLayout {
            spacing: Kirigami.Units.smallSpacing
            QQC2.Label {
                Layout.fillWidth: true
                wrapMode: Text.Wrap
                text: qsTr("Only this: the AtlasOS version, channel and previous version; the app's name, version and category (Plasma, KWin, Atlas app, other); the stack trace; the kernel; the GPU model and driver; uptime; the CPU model and how much RAM there is and is used; a random ID that changes every 30 days; the time; and the type of report.")
            }
            QQC2.Label {
                Layout.fillWidth: true
                wrapMode: Text.Wrap
                text: qsTr("Never: core dumps, your user name, the computer's name, MAC or IP addresses, serial numbers, installed apps, file contents, command lines or environment. Home folder paths are replaced with USER.")
            }
            QQC2.Label {
                Layout.fillWidth: true
                wrapMode: Text.Wrap
                text: qsTr("You always see the exact data before anything is sent, and you decide each time.")
            }
        }
    }

    ColumnLayout {
        spacing: Kirigami.Units.largeSpacing

        Cards {
            Layout.fillWidth: true
            backend: page.backend
        }

        Kirigami.AbstractCard {
            Layout.fillWidth: true
            contentItem: ColumnLayout {
                spacing: Kirigami.Units.largeSpacing
                Kirigami.Heading {
                    level: 3
                    text: qsTr("Crash reports")
                }
                QQC2.Switch {
                    text: qsTr("Send crash reports")
                    checked: page.backend.crashEnabled
                    onToggled: page.backend.enableCrashReports(checked)
                    Accessible.name: qsTr("Send crash reports")
                }
                QQC2.Label {
                    Layout.fillWidth: true
                    wrapMode: Text.Wrap
                    text: qsTr("When an app or the system crashes, a report is saved here and you can choose to send it. Nothing is sent without asking you first.")
                }
                Kirigami.InlineMessage {
                    Layout.fillWidth: true
                    visible: !page.backend.crashHasServer
                    type: Kirigami.MessageType.Information
                    text: qsTr("No crash report server is set up on this system, so reports can't be sent yet.")
                }
                Flow {
                    Layout.fillWidth: true
                    spacing: Kirigami.Units.smallSpacing
                    QQC2.Button {
                        text: qsTr("What's collected")
                        icon.name: "documentinfo"
                        flat: true
                        onClicked: collected.open()
                    }
                    QQC2.Button {
                        text: qsTr("Review reports")
                        icon.name: "tools-report-bug"
                        visible: page.backend.reportsCount > 0
                        onClicked: page.Window.window.showPage("reports")
                    }
                }
            }
        }
    }
}
