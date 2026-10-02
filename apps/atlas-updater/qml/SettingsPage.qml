import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Atlas.Ui

AtlasPage {
    id: page

    required property var backend

    signal openReports
    signal openSent

    title: qsTr("Settings")

    Component.onCompleted: backend.loadReports()

    ConfirmDialog {
        id: collected
        title: qsTr("What's Collected")
        acceptText: qsTr("Close")
        showReject: false

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

    Cards {
        Layout.fillWidth: true
        backend: page.backend
    }

    Section {
        title: qsTr("Crash Reports")
        footer: page.backend.crashHasServer ? qsTr("When an app or the system crashes, a report is saved here and you can choose to send it. Nothing is sent without asking you first.") : qsTr("When an app or the system crashes, a report is saved here and you can choose to send it. No crash report server is set up on this system, so reports can't be sent yet.")

        SectionRow {
            title: qsTr("Send crash reports")
            subtitle: qsTr("Off by default. You review every report first.")
            showSwitch: true
            switchChecked: page.backend.crashEnabled
            onSwitchToggled: checked => page.backend.enableCrashReports(checked)
        }
        SectionRow {
            title: qsTr("What's Collected")
            chevron: true
            onClicked: collected.open()
        }
        SectionRow {
            visible: page.backend.reportsCount > 0
            title: qsTr("Review Reports")
            value: page.backend.reportsCount
            chevron: true
            onClicked: page.openReports()
        }
        SectionRow {
            title: qsTr("Sent Reports")
            chevron: true
            onClicked: page.openSent()
        }
    }
}
