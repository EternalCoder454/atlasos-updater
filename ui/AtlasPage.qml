import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami

// A scrolling page with a large bold title and generous, centred margins.
Item {
    id: root

    property string title
    default property alias content: col.data
    readonly property real maxContentWidth: Kirigami.Units.gridUnit * 38

    QQC2.ScrollView {
        id: scroll
        anchors.fill: parent
        contentWidth: availableWidth

        // A slim overlay scrollbar instead of the classic one with arrows.
        QQC2.ScrollBar.vertical: QQC2.ScrollBar {
            parent: scroll
            x: scroll.width - width
            height: scroll.height
            policy: QQC2.ScrollBar.AsNeeded
            implicitWidth: 10
            padding: 2
            contentItem: Rectangle {
                implicitWidth: 6
                radius: width / 2
                color: Qt.alpha(Kirigami.Theme.textColor, parent.pressed ? 0.45 : parent.hovered ? 0.35 : 0.22)
                opacity: parent.active ? 1 : 0
                Behavior on opacity {
                    NumberAnimation {
                        duration: Kirigami.Units.longDuration
                    }
                }
            }
            background: null
        }

        Item {
            width: scroll.availableWidth
            implicitHeight: col.implicitHeight + Kirigami.Units.gridUnit * 3

            ColumnLayout {
                id: col
                y: Kirigami.Units.gridUnit * 1.5
                width: Math.min(parent.width - Kirigami.Units.gridUnit * 3, root.maxContentWidth)
                x: Math.round((parent.width - width) / 2)
                spacing: Kirigami.Units.gridUnit * 1.2

                QQC2.Label {
                    Layout.fillWidth: true
                    text: root.title
                    font.pointSize: Kirigami.Theme.defaultFont.pointSize * 1.6
                    font.bold: true
                    textFormat: Text.PlainText
                    elide: Text.ElideRight
                    Accessible.role: Accessible.Heading
                }
            }
        }
    }
}
