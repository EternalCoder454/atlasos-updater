import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Atlas.Ui
import "dates.js" as Dates

AtlasPage {
    id: page

    required property var backend

    title: qsTr("Go back")

    ConfirmDialog {
        id: confirm
        title: qsTr("Go back to %1?").arg(page.backend.rollbackVersion)
        text: qsTr("The next restart starts the previous version. Your files and settings stay as they are.")
        acceptText: qsTr("Go back")
        onAccepted: page.backend.rollback()
    }

    Cards {
        Layout.fillWidth: true
        backend: page.backend
    }

    Kirigami.PlaceholderMessage {
        Layout.fillWidth: true
        Layout.topMargin: Kirigami.Units.gridUnit * 3
        visible: page.backend.loaded && !page.backend.hasRollback
        icon.name: "edit-undo"
        text: qsTr("There is no previous version to go back to")
        explanation: qsTr("After the next update, the version you have now is kept here.")
    }

    StatusHero {
        Layout.topMargin: Kirigami.Units.gridUnit
        visible: page.backend.hasRollback
        iconName: "edit-undo"
        headline: qsTr("Something not working after an update?")
        subtitle: qsTr("You can go back to the version you used before. Nothing is deleted: you can update again later.")

        PrimaryButton {
            text: qsTr("Go back to %1 (%2)").arg(page.backend.rollbackVersion).arg(Dates.longDate(page.backend.rollbackDate))
            enabled: !page.backend.busy
            onClicked: confirm.open()
        }
        PrimaryButton {
            text: qsTr("Restart to update")
            visible: page.backend.restartNeeded
            enabled: !page.backend.busy
            onClicked: page.backend.restartNow()
        }
    }

    Section {
        visible: page.backend.hasRollback
        SectionRow {
            title: qsTr("Current")
            value: page.backend.currentVersion
        }
        SectionRow {
            title: qsTr("Previous")
            value: qsTr("%1  (%2)").arg(page.backend.rollbackVersion).arg(Dates.longDate(page.backend.rollbackDate))
        }
    }
}
