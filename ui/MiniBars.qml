import QtQuick
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

// A row of small upright bars, one per value, wrapping onto more rows when
// there are many (one per processor core). Each bar fills from the bottom to
// value / maximum and is numbered underneath when `numbered`.
//
//   MiniBars {
//       values: cpu.coreUsage
//       maximum: 100
//   }
Flow {
    id: root

    property list<real> values
    property real maximum: 100
    property bool numbered: true
    property color color: Kirigami.Theme.highlightColor
    property real barWidth: Math.round(Kirigami.Units.gridUnit * 0.9)
    property real barHeight: Kirigami.Units.gridUnit * 2.5
    // What a screen reader says for bar i, e.g. "Core 3".
    property var nameOf: i => qsTr("Core %1").arg(i)
    property var textOf: v => Math.round(v) + "%"

    Layout.fillWidth: true
    spacing: Kirigami.Units.smallSpacing

    Repeater {
        model: root.values.length

        Column {
            id: cell
            required property int index
            readonly property real share: root.maximum > 0 ? Math.max(0, Math.min(1, root.values[index] / root.maximum)) : 0

            spacing: 2
            Accessible.role: Accessible.Indicator
            Accessible.name: root.nameOf(index)
            Accessible.description: root.textOf(root.values[index])

            Item {
                width: root.barWidth
                height: root.barHeight

                Rectangle {
                    anchors.fill: parent
                    radius: 3
                    color: Qt.alpha(Kirigami.Theme.textColor, 0.1)
                }
                Rectangle {
                    anchors.bottom: parent.bottom
                    width: parent.width
                    // At least the radius, so a busy-but-small value still shows.
                    height: cell.share > 0 ? Math.max(radius * 2, Math.round(parent.height * cell.share)) : 0
                    radius: 3
                    color: root.color
                }
            }
            Text {
                visible: root.numbered
                width: root.barWidth
                horizontalAlignment: Text.AlignHCenter
                text: cell.index
                color: Qt.alpha(Kirigami.Theme.textColor, 0.55)
                font.family: Kirigami.Theme.smallFont.family
                font.pointSize: Kirigami.Theme.smallFont.pointSize * 0.85
                textFormat: Text.PlainText
            }
        }
    }
}
