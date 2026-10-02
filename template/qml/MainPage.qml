import QtQuick
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Atlas.Ui

AtlasPage {
    id: page

    required property var backend

    title: qsTr("Home")

    StatusHero {
        iconName: "checkmark"
        headline: qsTr("Hello from an Atlas app")
        subtitle: page.backend.status

        PrimaryButton {
            text: qsTr("Refresh")
            enabled: !page.backend.busy
            onClicked: page.backend.refresh()
        }
    }

    Section {
        title: qsTr("Example")
        SectionRow {
            title: qsTr("A row")
            subtitle: qsTr("Put your settings and lists in Sections.")
            value: qsTr("Value")
        }
    }
}
