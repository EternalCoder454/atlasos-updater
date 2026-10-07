import QtQuick
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Telamon.Ui
import "dates.js" as Dates

TelamonPage {
    id: page

    required property var backend

    title: qsTr("Go Back")

    ConfirmDialog {
        id: confirm
        title: qsTr("Go Back to %1?").arg(page.backend.rollbackVersion)
        text: page.rollbackIsBad ? qsTr("This version didn't pass its startup checks on this computer and was undone, so it will probably fail again. Your files and settings stay as they are.") : qsTr("The next restart starts the previous version. Your files and settings stay as they are.")
        acceptText: qsTr("Go Back to %1").arg(page.backend.rollbackVersion)
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

    TelamonEmptyState {
        Layout.fillWidth: true
        Layout.topMargin: Kirigami.Units.gridUnit * 3
        visible: page.backend.loaded && !page.backend.hasRollback
        iconName: "edit-undo"
        title: qsTr("There is no previous version to go back to")
        text: qsTr("After the next update, the version you have now is kept here.")
    }

    readonly property bool queued: page.backend.rollbackQueued === true
    // The previous version failed its startup checks here and was undone.
    readonly property bool rollbackIsBad: page.backend.rollbackIsBad === true
    readonly property string target: queued && page.backend.rollbackTarget ? page.backend.rollbackTarget : page.backend.rollbackVersion

    function version(v, date) {
        return date.length > 0 ? qsTr("%1  (%2)").arg(v).arg(Dates.longDate(date)) : v;
    }

    // A restart is under way (whatever started it).
    StatusHero {
        Layout.topMargin: Kirigami.Units.gridUnit
        visible: page.backend.restarting === true
        busy: true
        iconName: "view-refresh"
        headline: qsTr("Restarting System…")
        subtitle: qsTr("Saving your session…")
    }

    // Already queued: restart, or change your mind.
    StatusHero {
        Layout.topMargin: Kirigami.Units.gridUnit
        visible: page.queued && page.backend.restarting !== true
        iconName: "edit-undo"
        headline: qsTr("Ready to go back to %1").arg(page.target)
        subtitle: qsTr("The next restart starts that version. Your files and settings stay as they are.")

        PrimaryButton {
            text: qsTr("Restart Now")
            enabled: !page.backend.busy && !page.backend.restarting
            onClicked: page.backend.restartNow()
        }
        SecondaryButton {
            text: qsTr("Don't Go Back")
            enabled: !page.backend.busy && !page.backend.restarting
            onClicked: page.backend.cancelRollback()
        }
    }

    StatusHero {
        Layout.topMargin: Kirigami.Units.gridUnit
        visible: page.backend.hasRollback && !page.queued && page.backend.restarting !== true
        iconName: page.rollbackIsBad ? "dialog-warning" : "edit-undo"
        tint: page.rollbackIsBad ? Kirigami.Theme.neutralTextColor : TelamonStyle.accent
        headline: page.rollbackIsBad ? qsTr("The previous version didn't start properly") : qsTr("Something not working after an update?")
        subtitle: page.rollbackIsBad ? qsTr("Version %1 failed its startup checks on this computer and was undone, so going back to it will probably fail again.").arg(page.backend.rollbackVersion) : qsTr("You can go back to the version you used before. Nothing is deleted: you can update again later.")

        PrimaryButton {
            text: page.backend.rollbackDate.length > 0 ? qsTr("Go Back to %1 (%2)").arg(page.backend.rollbackVersion).arg(Dates.shortDate(page.backend.rollbackDate)) : qsTr("Go Back to %1").arg(page.backend.rollbackVersion)
            visible: !page.rollbackIsBad
            enabled: !page.backend.busy && !page.backend.restarting
            onClicked: confirm.open()
        }
        // Not the obvious next step: the confirmation says why.
        SecondaryButton {
            variant: TelamonButton.Destructive
            text: qsTr("Go Back Anyway")
            visible: page.rollbackIsBad
            enabled: !page.backend.busy && !page.backend.restarting
            onClicked: confirm.open()
        }
        SecondaryButton {
            text: qsTr("Restart to Update")
            visible: page.backend.restartNeeded
            enabled: !page.backend.busy && !page.backend.restarting
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
