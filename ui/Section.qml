import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami

// A rounded card that groups SectionRows (or any items) on a raised background.
//
// A `foldable` section's title is a button that folds the card away. The
// section doesn't keep the state: it asks with foldRequested, and the page
// sets `folded` (from a saved setting, say). Folded, the card is not drawn,
// and a binding that reads `folded` first can stop feeding what is inside:
//
//   Section {
//       id: cores
//       title: qsTr("Each Processor")
//       foldable: true
//       folded: settings.folded.includes("cores")
//       onFoldRequested: fold => settings.setFolded("cores", fold)
//       MiniBars { values: cores.folded ? [] : cpu.coreUsage }
//   }
ColumnLayout {
    id: root

    property string title
    property string footer
    property bool foldable: false
    property bool folded: false
    default property alias content: col.data

    // The title was clicked: `fold` is the state asked for.
    signal foldRequested(bool fold)

    Layout.fillWidth: true
    spacing: Kirigami.Units.smallSpacing

    QQC2.Label {
        visible: root.title.length > 0 && !root.foldable
        Layout.leftMargin: Kirigami.Units.largeSpacing
        text: root.title
        font.bold: true
        opacity: 0.65
        textFormat: Text.PlainText
        Accessible.role: Accessible.Heading
    }

    QQC2.AbstractButton {
        id: fold
        visible: root.title.length > 0 && root.foldable
        // Its width from the layout, never from the label: sized by its
        // content, a section that starts folded (no card to widen it)
        // settles at no room for the title at all.
        Layout.fillWidth: true
        leftPadding: Kirigami.Units.largeSpacing
        rightPadding: Kirigami.Units.smallSpacing
        topPadding: 2
        bottomPadding: 2
        hoverEnabled: true
        focusPolicy: Qt.StrongFocus
        text: root.title
        onClicked: root.foldRequested(!root.folded)
        Accessible.role: Accessible.Button
        Accessible.name: root.title
        Accessible.description: root.folded ? qsTr("Folded. Press to show.") : qsTr("Press to fold away.")

        // Around the title and chevron only, not the whole row.
        background: Rectangle {
            x: fold.mirrored ? fold.width - width : 0
            width: Math.min(fold.width, label.implicitWidth + chevron.width + Kirigami.Units.smallSpacing + fold.leftPadding + fold.rightPadding)
            height: fold.height
            radius: 6
            color: Qt.alpha(Kirigami.Theme.textColor, fold.pressed ? 0.1 : fold.hovered ? 0.05 : 0)
            border.width: fold.visualFocus ? 2 : 0
            border.color: Kirigami.Theme.focusColor
        }

        contentItem: RowLayout {
            spacing: Kirigami.Units.smallSpacing

            QQC2.Label {
                id: label
                Layout.maximumWidth: Math.max(0, fold.availableWidth - Kirigami.Units.iconSizes.small - Kirigami.Units.smallSpacing)
                text: fold.text
                font.bold: true
                opacity: 0.65
                elide: Text.ElideRight
                textFormat: Text.PlainText
            }
            Kirigami.Icon {
                id: chevron
                Layout.preferredWidth: Kirigami.Units.iconSizes.small
                Layout.preferredHeight: Kirigami.Units.iconSizes.small
                source: root.folded ? (fold.mirrored ? "go-previous" : "go-next") : "go-down"
                isMask: true
                color: Kirigami.Theme.textColor
                opacity: 0.6
            }
            Item {
                Layout.fillWidth: true
            }
        }
    }

    Rectangle {
        visible: !root.folded
        Layout.fillWidth: true
        implicitHeight: col.implicitHeight + 2
        radius: 10
        // Slightly raised over the page in both light and dark.
        color: Kirigami.Theme.backgroundColor.hslLightness > 0.5 ? Qt.lighter(Kirigami.Theme.backgroundColor, 1.5) : Qt.tint(Kirigami.Theme.backgroundColor, Qt.rgba(1, 1, 1, 0.06))
        border.width: 1
        border.color: Qt.alpha(Kirigami.Theme.textColor, 0.12)

        ColumnLayout {
            id: col
            anchors.fill: parent
            anchors.margins: 1
            spacing: 0
        }
    }

    Text {
        visible: root.footer.length > 0 && !root.folded
        Layout.fillWidth: true
        Layout.leftMargin: Kirigami.Units.largeSpacing
        Layout.rightMargin: Kirigami.Units.largeSpacing
        text: root.footer
        wrapMode: Text.Wrap
        font.family: Kirigami.Theme.defaultFont.family
        font.pointSize: Kirigami.Theme.defaultFont.pointSize * 0.92
        color: Qt.alpha(Kirigami.Theme.textColor, 0.65)
        textFormat: Text.PlainText
    }
}
