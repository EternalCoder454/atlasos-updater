import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami

// One row in a Section: optional icon, title and subtitle on the left; value,
// extra items, a checkmark, a switch or a chevron on the right. A clickable row
// takes keyboard focus (Tab), shows a focus ring and activates with Enter or
// Space. A `radio` row also moves selection with Up and Down.
FocusScope {
    id: root

    property string title
    property string subtitle
    property string value
    property string iconName
    property bool chevron: false
    // Rotate the chevron a quarter turn when `expanded` (disclosure rows).
    property bool disclosure: false
    property bool expanded: false
    property bool checkmark: false
    property bool radio: false
    property bool showSwitch: false
    property bool switchChecked: false
    property bool clickable: chevron
    default property alias trailing: trailingRow.data

    signal clicked
    signal switchToggled(bool checked)

    readonly property bool atlasRow: true
    readonly property bool mirrored: LayoutMirroring.enabled
    // The first visible row in a Section draws no separator above itself.
    readonly property bool isFirst: {
        var v = root.parent ? root.parent.visibleChildren : [];
        for (var i = 0; i < v.length; ++i) {
            if (v[i].atlasRow === true) {
                return v[i] === root;
            }
        }
        return true;
    }

    Layout.fillWidth: true
    implicitHeight: Math.max(Math.round(Kirigami.Units.gridUnit * 2.5), content.implicitHeight + Kirigami.Units.largeSpacing * 1.6)
    activeFocusOnTab: root.clickable
    opacity: !root.clickable && root.chevron ? 0.5 : 1

    Accessible.role: root.radio ? Accessible.RadioButton : (root.clickable ? Accessible.Button : Accessible.ListItem)
    Accessible.name: root.title
    Accessible.description: root.subtitle.length > 0 && root.value.length > 0 ? root.subtitle + ", " + root.value : root.subtitle + root.value
    Accessible.checkable: root.radio
    Accessible.checked: root.radio && root.checkmark
    Accessible.focusable: root.clickable
    Accessible.onPressAction: if (root.clickable) root.clicked()

    Keys.onReturnPressed: if (root.clickable) root.clicked()
    Keys.onEnterPressed: if (root.clickable) root.clicked()
    Keys.onSpacePressed: if (root.clickable) root.clicked()
    Keys.onDownPressed: event => root.step(true, event)
    Keys.onUpPressed: event => root.step(false, event)

    function step(forward, event) {
        if (!root.radio) {
            event.accepted = false;
            return;
        }
        var n = root.nextItemInFocusChain(forward);
        if (n && n.radio === true) {
            n.forceActiveFocus();
            n.clicked();
        }
    }

    Rectangle {
        visible: !root.isFirst
        anchors.top: parent.top
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.leftMargin: Kirigami.Units.largeSpacing + (root.iconName.length > 0 ? Kirigami.Units.iconSizes.smallMedium + Kirigami.Units.largeSpacing : 0)
        height: 1
        color: Qt.alpha(Kirigami.Theme.textColor, 0.1)
    }

    Rectangle {
        anchors.fill: parent
        anchors.margins: 3
        radius: 7
        color: Qt.alpha(Kirigami.Theme.textColor, tap.pressed ? 0.1 : 0.05)
        opacity: root.clickable && hover.hovered ? 1 : 0
        Behavior on opacity {
            NumberAnimation {
                duration: Kirigami.Units.shortDuration
            }
        }
    }
    Rectangle {
        anchors.fill: parent
        anchors.margins: 3
        radius: 7
        color: "transparent"
        border.width: 2
        border.color: Qt.alpha(Kirigami.Theme.highlightColor, 0.6)
        visible: root.activeFocus && root.clickable
    }

    HoverHandler {
        id: hover
        enabled: root.clickable
        cursorShape: Qt.PointingHandCursor
    }
    TapHandler {
        id: tap
        enabled: root.clickable
        onTapped: {
            root.forceActiveFocus();
            root.clicked();
        }
    }

    RowLayout {
        id: content
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.verticalCenter: parent.verticalCenter
        anchors.leftMargin: Kirigami.Units.largeSpacing
        anchors.rightMargin: Kirigami.Units.largeSpacing
        spacing: Kirigami.Units.largeSpacing

        Kirigami.Icon {
            visible: root.iconName.length > 0
            source: root.iconName
            fallback: "applications-other"
            Layout.preferredWidth: Kirigami.Units.iconSizes.smallMedium
            Layout.preferredHeight: Kirigami.Units.iconSizes.smallMedium
        }
        ColumnLayout {
            Layout.fillWidth: true
            Layout.minimumWidth: Kirigami.Units.gridUnit * 6
            spacing: 0
            QQC2.Label {
                Layout.fillWidth: true
                text: root.title
                wrapMode: Text.Wrap
                textFormat: Text.PlainText
                Accessible.ignored: true
            }
            QQC2.Label {
                Layout.fillWidth: true
                visible: root.subtitle.length > 0
                text: root.subtitle
                wrapMode: Text.Wrap
                font: Kirigami.Theme.smallFont
                opacity: 0.65
                textFormat: Text.PlainText
                Accessible.ignored: true
            }
        }
        QQC2.Label {
            visible: root.value.length > 0
            text: root.value
            opacity: 0.65
            horizontalAlignment: Text.AlignRight
            elide: Text.ElideRight
            Layout.maximumWidth: Math.round(root.width * 0.55)
            textFormat: Text.PlainText
            Accessible.ignored: true
        }
        Row {
            id: trailingRow
            spacing: Kirigami.Units.smallSpacing
        }
        Kirigami.Icon {
            visible: root.checkmark
            source: "checkmark"
            isMask: true
            color: Kirigami.Theme.highlightColor
            Layout.preferredWidth: Kirigami.Units.iconSizes.smallMedium
            Layout.preferredHeight: Kirigami.Units.iconSizes.smallMedium
        }
        AtlasSwitch {
            visible: root.showSwitch
            checked: root.switchChecked
            Accessible.name: root.title
            onToggled: {
                root.switchToggled(checked);
                // The switch shows what the system says, not what was clicked:
                // if saving fails, the binding puts it back.
                checked = Qt.binding(() => root.switchChecked);
            }
        }
        Kirigami.Icon {
            visible: root.chevron
            source: root.mirrored ? "arrow-left" : "arrow-right"
            isMask: true
            color: Kirigami.Theme.textColor
            opacity: 0.45
            rotation: root.disclosure && root.expanded ? 90 : 0
            Layout.preferredWidth: Kirigami.Units.iconSizes.small
            Layout.preferredHeight: Kirigami.Units.iconSizes.small
            Behavior on rotation {
                NumberAnimation {
                    duration: Kirigami.Units.shortDuration
                }
            }
        }
    }
}
