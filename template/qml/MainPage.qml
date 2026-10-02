import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

Kirigami.ScrollablePage {
    id: page

    required property var backend

    title: qsTr("Home")

    ColumnLayout {
        spacing: Kirigami.Units.largeSpacing

        Kirigami.Heading {
            text: qsTr("Hello from an Atlas app")
        }
        QQC2.Label {
            Layout.fillWidth: true
            wrapMode: Text.Wrap
            text: page.backend.status
        }
        QQC2.Button {
            text: qsTr("Refresh")
            icon.name: "view-refresh"
            enabled: !page.backend.busy
            onClicked: page.backend.refresh()
        }
    }
}
