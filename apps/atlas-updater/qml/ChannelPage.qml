import QtQuick
import QtQuick.Layouts
import Atlas.Ui

AtlasPage {
    id: page

    required property var backend

    title: qsTr("Channel")

    // What the user picked; starts at the channel the system follows.
    property string choice: backend.channel

    Connections {
        target: page.backend
        function onChannelChanged() {
            page.choice = page.backend.channel;
            confirm.close();
        }
    }

    ConfirmDialog {
        id: confirm
        title: qsTr("Switch to the %1 Channel?").arg(page.choice === "testing" ? qsTr("Testing") : qsTr("Stable"))
        text: qsTr("The new channel's latest version downloads and waits for a restart. Your files and settings stay as they are.")
        acceptText: qsTr("Switch Channel")
        focusReject: true
        onAccepted: page.backend.switchChannel(page.choice)
    }

    Cards {
        Layout.fillWidth: true
        backend: page.backend
    }

    Section {
        title: qsTr("How early do you want updates?")
        footer: page.backend.loaded && page.backend.channel === "" ? qsTr("This system follows a custom image, not Stable or Testing. Pick a channel to switch to it.") : ""

        SectionRow {
            title: qsTr("Stable")
            subtitle: qsTr("A new version about once a week. Recommended.")
            clickable: true
            radio: true
            checkmark: page.choice === "stable"
            Accessible.role: Accessible.RadioButton
            Accessible.checked: page.choice === "stable"
            onClicked: page.choice = "stable"
        }
        SectionRow {
            title: qsTr("Testing")
            subtitle: qsTr("A new version every day. You get changes first, and things may break more often.")
            clickable: true
            radio: true
            checkmark: page.choice === "testing"
            Accessible.role: Accessible.RadioButton
            Accessible.checked: page.choice === "testing"
            onClicked: page.choice = "testing"
        }
    }

    RowLayout {
        Layout.fillWidth: true
        spacing: AtlasStyle.spacingLarge
        Item {
            Layout.fillWidth: true
        }
        // After a switch the restart is the one thing left to do.
        readonly property bool switched: page.backend.restartNeeded && page.choice !== "" && page.choice === page.backend.channel

        SecondaryButton {
            text: page.backend.rollbackQueued ? qsTr("Restart Now") : qsTr("Restart to Update")
            visible: page.backend.restartNeeded && !parent.switched
            enabled: !page.backend.busy
            onClicked: page.backend.restartNow()
        }
        PrimaryButton {
            text: page.backend.rollbackQueued ? qsTr("Restart Now") : qsTr("Restart to Update")
            visible: parent.switched
            enabled: !page.backend.busy
            onClicked: page.backend.restartNow()
        }
        PrimaryButton {
            text: qsTr("Switch Channel")
            visible: !parent.switched
            enabled: !page.backend.busy && page.choice !== "" && page.choice !== page.backend.channel
            onClicked: confirm.open()
        }
    }
}
