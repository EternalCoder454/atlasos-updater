import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

Kirigami.ScrollablePage {
    id: page

    required property var backend

    title: qsTr("Channel")

    // What the user picked; starts at the channel the system follows.
    property string choice: backend.channel

    Connections {
        target: page.backend
        function onChannelChanged() {
            page.choice = page.backend.channel;
        }
    }

    Kirigami.PromptDialog {
        id: confirm
        title: qsTr("Switch to the %1 channel?").arg(page.choice === "testing" ? qsTr("Testing") : qsTr("Stable"))
        subtitle: qsTr("The new channel's latest version downloads and waits for a restart. Your files and settings stay as they are.")
        standardButtons: Kirigami.Dialog.NoButton
        customFooterActions: [
            Kirigami.Action {
                text: qsTr("Switch channel")
                icon.name: "dialog-ok"
                onTriggered: {
                    confirm.close();
                    page.backend.switchChannel(page.choice);
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

        Kirigami.InlineMessage {
            Layout.fillWidth: true
            type: Kirigami.MessageType.Information
            visible: page.backend.loaded && page.backend.channel === ""
            text: qsTr("This system follows a custom image, not Stable or Testing. Pick a channel below to switch to it.")
        }

        Kirigami.AbstractCard {
            Layout.fillWidth: true
            contentItem: ColumnLayout {
                spacing: Kirigami.Units.largeSpacing
                Kirigami.Heading {
                    level: 2
                    Layout.fillWidth: true
                    wrapMode: Text.Wrap
                    text: qsTr("How early do you want updates?")
                }

                QQC2.RadioButton {
                    id: stableButton
                    Layout.fillWidth: true
                    text: qsTr("Stable")
                    checked: page.choice === "stable"
                    onClicked: page.choice = "stable"
                    Accessible.name: qsTr("Stable")
                }
                QQC2.Label {
                    Layout.fillWidth: true
                    Layout.leftMargin: Kirigami.Units.gridUnit * 2
                    wrapMode: Text.Wrap
                    opacity: 0.7
                    text: qsTr("A new version about once a week. Recommended.")
                }

                QQC2.RadioButton {
                    id: testingButton
                    Layout.fillWidth: true
                    text: qsTr("Testing")
                    checked: page.choice === "testing"
                    onClicked: page.choice = "testing"
                    Accessible.name: qsTr("Testing")
                }
                QQC2.Label {
                    Layout.fillWidth: true
                    Layout.leftMargin: Kirigami.Units.gridUnit * 2
                    wrapMode: Text.Wrap
                    opacity: 0.7
                    text: qsTr("A new version every day. You get changes first, and things may break more often.")
                }

                Flow {
                    Layout.fillWidth: true
                    spacing: Kirigami.Units.smallSpacing
                    QQC2.Button {
                        text: qsTr("Switch channel")
                        icon.name: "dialog-ok"
                        enabled: !page.backend.busy && page.choice !== "" && page.choice !== page.backend.channel
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
