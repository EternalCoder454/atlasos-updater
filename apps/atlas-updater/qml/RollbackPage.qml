import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import "dates.js" as Dates

Kirigami.ScrollablePage {
    id: page

    required property var backend

    title: qsTr("Go back")

    Kirigami.PromptDialog {
        id: confirm
        title: qsTr("Go back to %1?").arg(page.backend.rollbackVersion)
        subtitle: qsTr("The next restart starts the previous version. Your files and settings stay as they are.")
        standardButtons: Kirigami.Dialog.NoButton
        customFooterActions: [
            Kirigami.Action {
                text: qsTr("Go back")
                icon.name: "edit-undo"
                onTriggered: {
                    confirm.close();
                    page.backend.rollback();
                }
            },
            Kirigami.Action {
                text: qsTr("Cancel")
                icon.name: "dialog-cancel"
                onTriggered: confirm.close()
            }
        ]
    }

    ColumnLayout {
        spacing: Kirigami.Units.largeSpacing

        Cards {
            Layout.fillWidth: true
            backend: page.backend
        }

        Kirigami.PlaceholderMessage {
            Layout.fillWidth: true
            Layout.topMargin: Kirigami.Units.gridUnit * 2
            visible: page.backend.loaded && !page.backend.hasRollback
            icon.name: "edit-undo"
            text: qsTr("There is no previous version to go back to")
            explanation: qsTr("After the next update, the version you have now is kept here.")
        }

        Kirigami.AbstractCard {
            Layout.fillWidth: true
            visible: page.backend.hasRollback
            contentItem: ColumnLayout {
                spacing: Kirigami.Units.largeSpacing
                Kirigami.Heading {
                    level: 2
                    Layout.fillWidth: true
                    wrapMode: Text.Wrap
                    text: qsTr("Something not working after an update?")
                }
                QQC2.Label {
                    Layout.fillWidth: true
                    wrapMode: Text.Wrap
                    text: qsTr("You can go back to the version you used before. Nothing is deleted: you can update again later.")
                }
                Kirigami.FormLayout {
                    Layout.fillWidth: true
                    wideMode: true
                    QQC2.Label {
                        Kirigami.FormData.label: qsTr("Current:")
                        text: page.backend.currentVersion
                    }
                    QQC2.Label {
                        Kirigami.FormData.label: qsTr("Previous:")
                        text: qsTr("%1  (%2)").arg(page.backend.rollbackVersion).arg(Dates.longDate(page.backend.rollbackDate))
                        wrapMode: Text.Wrap
                        Layout.fillWidth: true
                    }
                }
                Flow {
                    Layout.fillWidth: true
                    spacing: Kirigami.Units.smallSpacing
                    QQC2.Button {
                        text: qsTr("Go back to %1 (%2)").arg(page.backend.rollbackVersion).arg(Dates.longDate(page.backend.rollbackDate))
                        icon.name: "edit-undo"
                        enabled: !page.backend.busy
                        onClicked: confirm.open()
                    }
                    QQC2.Button {
                        text: qsTr("Restart to update")
                        icon.name: "system-reboot"
                        visible: page.backend.restartNeeded
                        highlighted: true
                        enabled: !page.backend.busy
                        onClicked: page.backend.restartNow()
                    }
                }
            }
        }
    }
}
