import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Atlas.Ui

AtlasPage {
    id: page

    required property var backend

    title: qsTr("About")

    ColumnLayout {
        Layout.fillWidth: true
        Layout.topMargin: Kirigami.Units.gridUnit
        spacing: Kirigami.Units.smallSpacing

        Kirigami.Icon {
            Layout.alignment: Qt.AlignHCenter
            source: "net.eterneon.atlas.updater"
            Layout.preferredWidth: Math.round(Kirigami.Units.gridUnit * 5)
            Layout.preferredHeight: Layout.preferredWidth
        }
        Kirigami.Heading {
            Layout.alignment: Qt.AlignHCenter
            Layout.topMargin: Kirigami.Units.smallSpacing
            text: qsTr("Atlas Updater")
        }
        QQC2.Label {
            Layout.alignment: Qt.AlignHCenter
            opacity: 0.7
            text: qsTr("Version %1").arg(Qt.application.version)
        }
        QQC2.Label {
            Layout.fillWidth: true
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.Wrap
            opacity: 0.7
            text: qsTr("Updates for AtlasOS: see what's new, go back, and pick a channel.")
        }
    }

    Section {
        title: qsTr("Privacy")
        footer: qsTr("Atlas Updater collects nothing. Crash reports are off unless you turn them on. Each report is shown to you before it's sent.")
        SectionRow {
            title: qsTr("Licence")
            value: qsTr("MIT")
        }
        SectionRow {
            title: qsTr("Made by")
            value: qsTr("Eterneon")
        }
        SectionRow {
            title: qsTr("Project page")
            chevron: true
            onClicked: {
                var url = "https://github.com/EternalCoder454/AtlasOS";
                if (page.backend.isSafeLink(url)) {
                    Qt.openUrlExternally(url);
                }
            }
        }
    }
}
