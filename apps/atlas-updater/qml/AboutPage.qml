import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

Kirigami.ScrollablePage {
    id: page

    required property var backend

    title: qsTr("About")

    ColumnLayout {
        spacing: Kirigami.Units.largeSpacing

        Kirigami.Icon {
            Layout.alignment: Qt.AlignHCenter
            Layout.topMargin: Kirigami.Units.gridUnit
            source: "net.eterneon.atlas.updater"
            Layout.preferredWidth: Kirigami.Units.iconSizes.enormous
            Layout.preferredHeight: Kirigami.Units.iconSizes.enormous
        }
        Kirigami.Heading {
            Layout.alignment: Qt.AlignHCenter
            text: qsTr("Atlas Updater")
        }
        QQC2.Label {
            Layout.alignment: Qt.AlignHCenter
            opacity: 0.7
            text: qsTr("Version %1").arg(Qt.application.version)
        }
        QQC2.Label {
            Layout.alignment: Qt.AlignHCenter
            Layout.fillWidth: true
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.Wrap
            text: qsTr("Updates for AtlasOS: see what's new, go back, and pick a channel.")
        }
        Kirigami.Separator {
            Layout.fillWidth: true
        }
        Kirigami.Heading {
            level: 3
            text: qsTr("Privacy")
        }
        QQC2.Label {
            Layout.fillWidth: true
            wrapMode: Text.Wrap
            text: qsTr("Atlas Updater collects nothing. Crash reports are off unless you turn them on. Each report is shown to you before it's sent.")
        }
        QQC2.Label {
            Layout.fillWidth: true
            wrapMode: Text.Wrap
            opacity: 0.7
            text: qsTr("Released under the MIT licence. Made by Eterneon.")
        }
        QQC2.Button {
            Layout.alignment: Qt.AlignHCenter
            text: qsTr("Project page")
            icon.name: "internet-services"
            onClicked: Qt.openUrlExternally("https://github.com/EternalCoder454/AtlasOS")
        }
    }
}
