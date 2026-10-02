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
        acceptText: qsTr("Go back to %1").arg(page.backend.rollbackVersion)
        focusReject: true
        onAccepted: page.backend.rollback()
    }

    Connections {
        target: page.backend
        function onRollbackQueuedChanged() {
            confirm.close();
        }
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

    readonly property bool queued: page.backend.rollbackQueued === true
    readonly property string target: queued && page.backend.rollbackTarget ? page.backend.rollbackTarget : page.backend.rollbackVersion

    function version(v, date) {
        return date.length > 0 ? qsTr("%1  (%2)").arg(v).arg(Dates.longDate(date)) : v;
    }

    // Already queued: restart, or change your mind.
    StatusHero {
        Layout.topMargin: Kirigami.Units.gridUnit
        visible: page.queued
        iconName: "edit-undo"
        headline: qsTr("Ready to go back to %1").arg(page.target)
        subtitle: qsTr("The next restart starts that version. Your files and settings stay as they are.")

        PrimaryButton {
            text: qsTr("Restart now")
            enabled: !page.backend.busy
            onClicked: page.backend.restartNow()
        }
        SecondaryButton {
            text: qsTr("Don't go back")
            enabled: !page.backend.busy
            onClicked: page.backend.cancelRollback()
        }
    }

    StatusHero {
        Layout.topMargin: Kirigami.Units.gridUnit
        visible: page.backend.hasRollback && !page.queued
        iconName: "edit-undo"
        headline: qsTr("Something not working after an update?")
        subtitle: qsTr("You can go back to the version you used before. Nothing is deleted: you can update again later.")

        PrimaryButton {
            text: page.backend.rollbackDate.length > 0 ? qsTr("Go back to %1 (%2)").arg(page.backend.rollbackVersion).arg(Dates.shortDate(page.backend.rollbackDate)) : qsTr("Go back to %1").arg(page.backend.rollbackVersion)
            enabled: !page.backend.busy
            onClicked: confirm.open()
        }
        SecondaryButton {
            text: qsTr("Restart to update")
            visible: page.backend.restartNeeded
            enabled: !page.backend.busy
            onClicked: page.backend.restartNow()
        }
    }

    Section {
        title: qsTr("Versions")
        visible: page.backend.hasRollback
        SectionRow {
            title: qsTr("Current")
            value: page.version(page.backend.currentVersion, page.backend.currentDate)
        }
        SectionRow {
            title: qsTr("Previous")
            value: page.version(page.backend.rollbackVersion, page.backend.rollbackDate)
        }
    }
}
