import QtQuick
import QtQuick.Templates as T
import org.kde.kirigami as Kirigami

// A rounded search field: a magnifier, the text, and a clear button once
// there is text. `query` follows the text after a short pause, so a live
// list filters once per word rather than per key; bind to it, not to text.
// Escape clears the field (and, when it is already empty, lets the key go
// on to close whatever it is in).
T.TextField {
    id: control

    // The text after `delay` ms without typing; at once when cleared.
    property string query
    property int delay: 150
    // A TextField is no Control, so it has no `mirrored` of its own.
    readonly property bool rtl: LayoutMirroring.enabled

    implicitWidth: Kirigami.Units.gridUnit * 14
    implicitHeight: Math.round(Kirigami.Units.gridUnit * 1.9)
    leftPadding: (rtl ? clearButton.width : icon.width) + Kirigami.Units.largeSpacing + Kirigami.Units.smallSpacing
    rightPadding: (rtl ? icon.width : clearButton.width) + Kirigami.Units.largeSpacing + Kirigami.Units.smallSpacing
    verticalAlignment: TextInput.AlignVCenter
    placeholderText: qsTr("Search")
    placeholderTextColor: Qt.alpha(Kirigami.Theme.textColor, 0.5)
    color: Kirigami.Theme.textColor
    selectionColor: Kirigami.Theme.highlightColor
    selectedTextColor: Kirigami.Theme.highlightedTextColor
    font: Kirigami.Theme.defaultFont
    selectByMouse: true
    inputMethodHints: Qt.ImhNoPredictiveText

    Accessible.role: Accessible.EditableText
    Accessible.name: placeholderText
    Accessible.searchEdit: true

    onTextChanged: {
        if (text.length === 0) {
            pause.stop();
            query = "";
        } else {
            pause.restart();
        }
    }
    Keys.onEscapePressed: event => {
        if (text.length > 0) {
            clear();
        } else {
            event.accepted = false;
        }
    }

    Timer {
        id: pause
        interval: control.delay
        onTriggered: control.query = control.text
    }

    background: Rectangle {
        radius: height / 2
        color: Qt.alpha(Kirigami.Theme.textColor, control.hovered && !control.activeFocus ? 0.09 : 0.06)
        border.width: control.activeFocus ? 2 : 1
        border.color: control.activeFocus ? Qt.alpha(Kirigami.Theme.highlightColor, 0.7) : Qt.alpha(Kirigami.Theme.textColor, 0.1)
    }

    // A template field keeps placeholderText but draws nothing for it.
    Text {
        x: control.leftPadding
        anchors.verticalCenter: parent.verticalCenter
        width: control.availableWidth
        visible: control.length === 0 && control.preeditText.length === 0
        text: control.placeholderText
        font: control.font
        color: control.placeholderTextColor
        verticalAlignment: control.verticalAlignment
        elide: Text.ElideRight
        renderType: control.renderType
        Accessible.ignored: true
    }

    Kirigami.Icon {
        id: icon
        x: control.rtl ? control.width - width - Kirigami.Units.largeSpacing : Kirigami.Units.largeSpacing
        anchors.verticalCenter: parent.verticalCenter
        width: Kirigami.Units.iconSizes.small
        height: width
        source: "search"
        isMask: true
        color: Kirigami.Theme.textColor
        opacity: 0.55
    }

    T.AbstractButton {
        id: clearButton
        x: control.rtl ? Kirigami.Units.smallSpacing : control.width - width - Kirigami.Units.smallSpacing
        anchors.verticalCenter: parent.verticalCenter
        width: Kirigami.Units.iconSizes.small + Kirigami.Units.smallSpacing * 2
        height: width
        visible: control.text.length > 0
        focusPolicy: Qt.NoFocus
        hoverEnabled: true
        Accessible.name: qsTr("Clear Search")
        onClicked: {
            control.clear();
            control.forceActiveFocus();
        }

        background: Rectangle {
            radius: width / 2
            color: Qt.alpha(Kirigami.Theme.textColor, clearButton.down ? 0.15 : clearButton.hovered ? 0.08 : 0)
        }
        contentItem: Kirigami.Icon {
            source: "edit-clear"
            isMask: true
            color: Kirigami.Theme.textColor
            opacity: 0.6
        }
    }
}
