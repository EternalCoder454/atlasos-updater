import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami
import Atlas.Ui

// Messages every page shows at the top: errors in plain language, results.
ColumnLayout {
    id: root

    required property var backend
    // The Updates page shows the error in its hero instead.
    property bool showError: true

    spacing: Kirigami.Units.smallSpacing
    visible: backend.fixturesActive || (showError && backend.errorText.length > 0) || backend.infoText.length > 0 || (backend.busy && showError)

    component Banner: Rectangle {
        id: banner
        property string message
        property color tint
        property string iconName
        property bool dismissable: true
        property string dismissName: qsTr("Dismiss")
        Layout.fillWidth: true
        visible: message.length > 0
        Accessible.role: Accessible.AlertMessage
        Accessible.name: message
        implicitHeight: bannerRow.implicitHeight + Kirigami.Units.largeSpacing * 2
        radius: 10
        color: Qt.alpha(tint, 0.14)
        border.width: 1
        border.color: Qt.alpha(tint, 0.35)
        RowLayout {
            id: bannerRow
            anchors.fill: parent
            anchors.margins: Kirigami.Units.largeSpacing
            spacing: Kirigami.Units.largeSpacing
            Kirigami.Icon {
                source: banner.iconName
                Layout.preferredWidth: Kirigami.Units.iconSizes.smallMedium
                Layout.preferredHeight: Kirigami.Units.iconSizes.smallMedium
            }
            QQC2.Label {
                Layout.fillWidth: true
                text: banner.message
                wrapMode: Text.Wrap
                textFormat: Text.PlainText
            }
            SecondaryButton {
                visible: banner.dismissable
                text: qsTr("Dismiss")
                Accessible.name: banner.dismissName
                onClicked: root.backend.dismissMessages()
            }
        }
    }

    Banner {
        message: root.backend.fixturesActive ? qsTr("Developer test data: this is not your real system.") : ""
        tint: Kirigami.Theme.neutralTextColor
        iconName: "dialog-information"
        dismissable: false
    }
    Banner {
        message: root.showError ? root.backend.errorText : ""
        tint: Kirigami.Theme.negativeTextColor
        iconName: "dialog-error"
        dismissName: qsTr("Dismiss error")
    }
    Banner {
        message: root.backend.infoText
        tint: Kirigami.Theme.highlightColor
        iconName: "dialog-information"
        dismissName: qsTr("Dismiss message")
    }
    RowLayout {
        Layout.fillWidth: true
        visible: root.backend.busy && root.showError
        spacing: Kirigami.Units.largeSpacing
        QQC2.BusyIndicator {
            running: root.backend.busy
        }
        QQC2.Label {
            Layout.fillWidth: true
            text: root.backend.busyText
            wrapMode: Text.Wrap
            textFormat: Text.PlainText
        }
    }
}
