import QtQuick
import QtQuick.Layouts
import QtQuick.Templates as T
import org.kde.kirigami as Kirigami

// Sidebar entry: accent icon plus label, with a rounded selection pill.
T.AbstractButton {
    id: control

    property bool selected: false
    // Icon only (narrow windows); the text becomes the tooltip and accessible name.
    property bool compact: false

    implicitHeight: Math.round(Kirigami.Units.gridUnit * 2.1)
    implicitWidth: compact ? implicitHeight + Kirigami.Units.smallSpacing : Kirigami.Units.gridUnit * 10
    hoverEnabled: true
    focusPolicy: Qt.StrongFocus
    Accessible.name: control.text

    background: Rectangle {
        radius: 8
        color: control.selected ? Qt.alpha(Kirigami.Theme.highlightColor, 0.18) : Qt.alpha(Kirigami.Theme.textColor, control.down ? 0.1 : control.hovered ? 0.06 : 0)
        border.width: control.visualFocus ? 2 : 0
        border.color: Qt.alpha(Kirigami.Theme.highlightColor, 0.6)
        Behavior on color {
            ColorAnimation {
                duration: Kirigami.Units.shortDuration
            }
        }
    }

    contentItem: RowLayout {
        spacing: Kirigami.Units.largeSpacing
        Item {
            Layout.fillWidth: control.compact
            visible: control.compact
        }
        Kirigami.Icon {
            Layout.leftMargin: control.compact ? 0 : Kirigami.Units.largeSpacing
            Layout.preferredWidth: Kirigami.Units.iconSizes.smallMedium
            Layout.preferredHeight: Kirigami.Units.iconSizes.smallMedium
            source: control.icon.name
            isMask: true
            color: Kirigami.Theme.highlightColor
        }
        Text {
            visible: !control.compact
            Layout.fillWidth: true
            text: control.text
            font: Kirigami.Theme.defaultFont
            textFormat: Text.PlainText
            elide: Text.ElideRight
            color: Kirigami.Theme.textColor
            Component.onCompleted: font.weight = Font.Medium
        }
        Item {
            Layout.fillWidth: control.compact
            visible: control.compact
        }
    }
}
