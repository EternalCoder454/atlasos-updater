import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami

// Messages every page shows at the top: errors in plain language, results.
ColumnLayout {
    id: root

    required property var backend

    spacing: Kirigami.Units.smallSpacing

    Kirigami.InlineMessage {
        Layout.fillWidth: true
        type: Kirigami.MessageType.Error
        text: root.backend.errorText
        visible: text.length > 0
        actions: [
            Kirigami.Action {
                text: qsTr("Dismiss")
                onTriggered: root.backend.dismissMessages()
            }
        ]
    }
    Kirigami.InlineMessage {
        Layout.fillWidth: true
        type: Kirigami.MessageType.Positive
        text: root.backend.infoText
        visible: text.length > 0
        actions: [
            Kirigami.Action {
                text: qsTr("Dismiss")
                onTriggered: root.backend.dismissMessages()
            }
        ]
    }
    RowLayout {
        Layout.fillWidth: true
        visible: root.backend.busy
        spacing: Kirigami.Units.largeSpacing
        QQC2.BusyIndicator {
            running: root.backend.busy
        }
        QQC2.Label {
            Layout.fillWidth: true
            text: root.backend.busyText
            wrapMode: Text.Wrap
        }
    }
}
