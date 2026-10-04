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
    // The progress line (pages without their own busy display).
    property bool showBusy: showError

    spacing: Kirigami.Units.smallSpacing
    visible: backend.fixturesActive || (showError && backend.errorText.length > 0) || backend.infoText.length > 0 || (backend.busy && showBusy)

    InfoBanner {
        Layout.fillWidth: true
        type: "warning"
        text: qsTr("Developer test data: this is not your real system.")
        shown: root.backend.fixturesActive
    }
    InfoBanner {
        id: errorBanner
        Layout.fillWidth: true
        type: "error"
        text: root.showError ? root.backend.errorText : ""
        closable: true
        onClosed: root.backend.dismissMessages()
        // Closing breaks a plain binding; this one re-applies on the next message.
        Binding {
            target: errorBanner
            property: "shown"
            value: errorBanner.text.length > 0
        }
    }
    InfoBanner {
        id: infoBanner
        Layout.fillWidth: true
        type: "info"
        text: root.backend.infoText
        closable: true
        onClosed: root.backend.dismissMessages()
        Binding {
            target: infoBanner
            property: "shown"
            value: infoBanner.text.length > 0
        }
    }
    RowLayout {
        Layout.fillWidth: true
        visible: root.backend.busy && root.showBusy
        spacing: Kirigami.Units.largeSpacing
        AtlasSpinner {
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
