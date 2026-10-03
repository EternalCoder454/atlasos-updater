import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami

// A table in the Section style: a rounded card with a header that sorts and
// inset rows below it, scrolling on its own and making rows only for what is
// on screen, so a list of a thousand processes costs what twenty do.
//
// The table doesn't sort: a header click sets sortRole and sortOrder, and the
// model follows them (a Rust QAbstractItemModel that moves rows, not one that
// resets). While the pointer is over the rows `pointerInside` is true; a live
// model holds its order still then, so the row under the pointer stays put.
//
//   DataTable {
//       model: apps
//       columns: [
//           { title: qsTr("Name"), role: "name", fill: true, iconRole: "icon" },
//           { title: qsTr("CPU"), role: "cpu", width: 5, align: Qt.AlignRight,
//             heat: 100, text: v => v.toFixed(1) + "%" },
//       ]
//       onActivated: row => ...
//       onContextMenuRequested: (row, x, y) => menu.popup(...)
//   }
//
// A column is an object with:
//   title, role       the header and the model role it shows
//   width             in grid units; or `fill: true` for the column that
//                     takes what is left (the first one, if none says so)
//   align             Qt.AlignLeft (default) or Qt.AlignRight for figures
//   text(value, row)  formats the value; `row` is the delegate's model object
//   heat              shade the cell by value / heat (load columns, as in the
//                     Windows Task Manager); 0 or absent for none
//   iconRole          a role holding an icon name, drawn before the text
//   cell              a Component for anything else (a status dot, a switch);
//                     it gets `value`, `row` and `column` set
//   sortable          false to keep its header from sorting (default true)
//
// A tree (an app and its processes) is the model's to flatten: give
// `depthRole`, `expandableRole` and `expandedRole`, and the first column
// indents and shows a chevron that emits toggleRequested(row).
FocusScope {
    id: root

    property var model
    property var columns: []
    property string sortRole
    property int sortOrder: Qt.DescendingOrder
    property alias currentIndex: list.currentIndex
    property string depthRole
    property string expandableRole
    property string expandedRole
    // Shown in the middle when there are no rows ("No Apps Match").
    property string placeholderText
    readonly property bool pointerInside: hover.hovered
    readonly property real rowHeight: Math.round(Kirigami.Units.gridUnit * 1.9)
    readonly property alias count: list.count

    signal activated(int row)
    signal contextMenuRequested(int row, real x, real y)
    signal deleteRequested(int row)
    signal toggleRequested(int row)

    implicitWidth: Kirigami.Units.gridUnit * 30
    implicitHeight: Kirigami.Units.gridUnit * 20
    Layout.fillWidth: true
    activeFocusOnTab: true

    Accessible.role: Accessible.Table
    Accessible.name: placeholderText

    readonly property real padding: Kirigami.Units.smallSpacing
    readonly property real cellPadding: Kirigami.Units.largeSpacing
    // Pixel widths, the fill column taking what the others leave.
    readonly property var widths: {
        const avail = Math.max(0, width - 2 * padding);
        let fixed = 0, fill = -1;
        for (let i = 0; i < columns.length; ++i) {
            if (columns[i].fill === true && fill < 0) {
                fill = i;
            } else if (columns[i].width !== undefined) {
                fixed += Math.round(columns[i].width * Kirigami.Units.gridUnit);
            }
        }
        if (fill < 0) {
            fill = 0;
        }
        const w = [];
        for (let i = 0; i < columns.length; ++i) {
            w.push(i === fill ? Math.max(Kirigami.Units.gridUnit * 4, avail - fixed + (columns[i].width === undefined || columns[i].fill ? 0 : Math.round(columns[i].width * Kirigami.Units.gridUnit))) : Math.round(columns[i].width * Kirigami.Units.gridUnit));
        }
        return w;
    }

    function sortBy(i) {
        const c = columns[i];
        if (c.sortable === false) {
            return;
        }
        if (sortRole === c.role) {
            sortOrder = sortOrder === Qt.AscendingOrder ? Qt.DescendingOrder : Qt.AscendingOrder;
        } else {
            sortRole = c.role;
            // Figures start biggest first, names A to Z.
            sortOrder = c.align === Qt.AlignRight ? Qt.DescendingOrder : Qt.AscendingOrder;
        }
    }

    Keys.onPressed: event => {
        const page = Math.max(1, Math.floor(list.height / root.rowHeight) - 1);
        let to = list.currentIndex;
        switch (event.key) {
        case Qt.Key_Up:
            to = Math.max(0, to - 1);
            break;
        case Qt.Key_Down:
            to = Math.min(list.count - 1, to + 1);
            break;
        case Qt.Key_PageUp:
            to = Math.max(0, to - page);
            break;
        case Qt.Key_PageDown:
            to = Math.min(list.count - 1, to + page);
            break;
        case Qt.Key_Home:
            to = 0;
            break;
        case Qt.Key_End:
            to = list.count - 1;
            break;
        case Qt.Key_Return:
        case Qt.Key_Enter:
            if (to >= 0 && !event.isAutoRepeat) {
                root.activated(to);
            }
            event.accepted = true;
            return;
        case Qt.Key_Delete:
            if (to >= 0 && !event.isAutoRepeat) {
                root.deleteRequested(to);
            }
            event.accepted = true;
            return;
        case Qt.Key_Menu:
            root.openMenuAtCurrent();
            event.accepted = true;
            return;
        case Qt.Key_F10:
            if (event.modifiers & Qt.ShiftModifier) {
                root.openMenuAtCurrent();
                event.accepted = true;
            }
            return;
        case Qt.Key_Left:
        case Qt.Key_Right:
            // Fold or unfold a tree row, the way a file manager does.
            if (to >= 0 && root.expandableRole && list.currentItem && list.currentItem.expandable) {
                const open = (event.key === Qt.Key_Right) !== root.mirrored;
                if (open !== list.currentItem.expanded) {
                    root.toggleRequested(to);
                }
            }
            event.accepted = true;
            return;
        default:
            return;
        }
        if (to !== list.currentIndex && to >= 0) {
            list.currentIndex = to;
            list.positionViewAtIndex(to, ListView.Contain);
        }
        event.accepted = true;
    }
    readonly property bool mirrored: LayoutMirroring.enabled

    function openMenuAtCurrent() {
        const item = list.currentItem;
        if (!item) {
            return;
        }
        const p = item.mapToItem(root, Kirigami.Units.gridUnit * 2, item.height / 2);
        root.contextMenuRequested(list.currentIndex, p.x, p.y);
    }

    // The card, drawn like Section's.
    Rectangle {
        anchors.fill: parent
        radius: 10
        color: Kirigami.Theme.backgroundColor.hslLightness > 0.5 ? Qt.lighter(Kirigami.Theme.backgroundColor, 1.5) : Qt.tint(Kirigami.Theme.backgroundColor, Qt.rgba(1, 1, 1, 0.06))
        border.width: 1
        border.color: root.activeFocus ? Qt.alpha(Kirigami.Theme.highlightColor, 0.6) : Qt.alpha(Kirigami.Theme.textColor, 0.12)
    }

    Row {
        id: header
        x: root.padding
        y: root.padding
        height: Math.round(Kirigami.Units.gridUnit * 1.8)
        LayoutMirroring.enabled: root.mirrored

        Repeater {
            model: root.columns.length

            Item {
                id: head
                required property int index
                readonly property var column: root.columns[index]
                readonly property bool sorted: column.role === root.sortRole
                readonly property bool alignRight: column.align === Qt.AlignRight

                width: root.widths[index] ?? 0
                height: header.height
                Accessible.role: Accessible.ColumnHeader
                Accessible.name: column.title

                Rectangle {
                    anchors.fill: parent
                    anchors.margins: 1
                    radius: 6
                    color: Qt.alpha(Kirigami.Theme.textColor, headMouse.pressed ? 0.1 : headMouse.containsMouse ? 0.05 : 0)
                }

                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: root.cellPadding
                    anchors.rightMargin: root.cellPadding
                    spacing: Kirigami.Units.smallSpacing
                    layoutDirection: head.alignRight !== root.mirrored ? Qt.RightToLeft : Qt.LeftToRight

                    QQC2.Label {
                        text: head.column.title
                        font.weight: head.sorted ? Font.DemiBold : Font.Normal
                        opacity: head.sorted ? 0.9 : 0.6
                        elide: Text.ElideRight
                        textFormat: Text.PlainText
                        Layout.maximumWidth: head.width - root.cellPadding * 2 - Kirigami.Units.iconSizes.small
                    }
                    Kirigami.Icon {
                        visible: head.sorted
                        Layout.preferredWidth: Kirigami.Units.iconSizes.small
                        Layout.preferredHeight: Kirigami.Units.iconSizes.small
                        source: root.sortOrder === Qt.AscendingOrder ? "arrow-up" : "arrow-down"
                        isMask: true
                        color: Kirigami.Theme.textColor
                        opacity: 0.6
                    }
                    Item {
                        Layout.fillWidth: true
                    }
                }

                MouseArea {
                    id: headMouse
                    anchors.fill: parent
                    hoverEnabled: true
                    enabled: head.column.sortable !== false
                    cursorShape: enabled ? Qt.PointingHandCursor : Qt.ArrowCursor
                    onClicked: root.sortBy(head.index)
                }
            }
        }
    }

    Rectangle {
        id: rule
        x: root.padding + root.cellPadding / 2
        y: header.y + header.height
        width: root.width - 2 * x
        height: 1
        color: Qt.alpha(Kirigami.Theme.textColor, 0.1)
    }

    Item {
        id: rows
        x: root.padding
        y: rule.y + rule.height + root.padding / 2
        width: root.width - 2 * root.padding
        // Stop short of the bottom corners' curve.
        height: root.height - y - root.padding * 2
    }

    ListView {
        id: list
        anchors.fill: rows
        clip: true
        model: root.model
        reuseItems: true
        boundsBehavior: Flickable.StopAtBounds
        currentIndex: -1
        highlightMoveDuration: 0
        keyNavigationEnabled: false
        activeFocusOnTab: false

        // Over the rows only: pointing at the header doesn't hold the order.
        // An ancestor of the rows, since their MouseAreas take the hover.
        HoverHandler {
            id: hover
        }
        QQC2.ScrollBar.vertical: QQC2.ScrollBar {
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

        delegate: Item {
            id: row
            required property int index
            required property var model

            readonly property bool selected: ListView.isCurrentItem
            readonly property int depth: root.depthRole ? (model[root.depthRole] ?? 0) : 0
            readonly property bool expandable: root.expandableRole ? model[root.expandableRole] === true : false
            readonly property bool expanded: root.expandedRole ? model[root.expandedRole] === true : false

            width: ListView.view.width
            height: root.rowHeight
            Accessible.role: Accessible.Row
            Accessible.selected: selected
            Accessible.name: {
                const parts = [];
                for (let i = 0; i < root.columns.length; ++i) {
                    const c = root.columns[i];
                    const v = row.model[c.role];
                    parts.push(c.title + " " + (c.text ? c.text(v, row.model) : v));
                }
                return parts.join(", ");
            }

            Rectangle {
                anchors.fill: parent
                anchors.topMargin: 1
                anchors.bottomMargin: 1
                radius: 6
                color: row.selected ? Qt.alpha(Kirigami.Theme.highlightColor, root.activeFocus ? 0.22 : 0.14) : Qt.alpha(Kirigami.Theme.textColor, rowMouse.pressed ? 0.08 : rowMouse.containsMouse ? 0.045 : 0)
            }

            Row {
                anchors.fill: parent
                LayoutMirroring.enabled: root.mirrored

                Repeater {
                    model: root.columns.length

                    Item {
                        id: cell
                        required property int index
                        readonly property var column: root.columns[index]
                        readonly property var value: row.model[column.role]
                        readonly property real heat: column.heat > 0 ? Math.max(0, Math.min(1, Number(value) / column.heat)) : 0
                        readonly property real indent: index === 0 ? row.depth * Kirigami.Units.gridUnit * 1.2 : 0

                        width: root.widths[index] ?? 0
                        height: row.height

                        // Heat: the busier, the warmer. Faint at idle so a
                        // quiet list stays quiet.
                        Rectangle {
                            visible: cell.heat > 0.02
                            anchors.fill: parent
                            anchors.topMargin: 1
                            anchors.bottomMargin: 1
                            color: Qt.alpha(Kirigami.Theme.neutralTextColor, 0.06 + 0.32 * cell.heat)
                        }

                        Kirigami.Icon {
                            id: chevron
                            visible: cell.index === 0 && row.expandable
                            x: root.mirrored ? parent.width - width - root.cellPadding / 2 - cell.indent : root.cellPadding / 2 + cell.indent
                            anchors.verticalCenter: parent.verticalCenter
                            width: Kirigami.Units.iconSizes.small
                            height: width
                            source: root.mirrored ? "arrow-left" : "arrow-right"
                            isMask: true
                            color: Kirigami.Theme.textColor
                            opacity: 0.45
                            rotation: row.expanded ? (root.mirrored ? -90 : 90) : 0

                            TapHandler {
                                onTapped: root.toggleRequested(row.index)
                            }
                        }

                        Loader {
                            id: custom
                            active: cell.column.cell !== undefined
                            anchors.fill: parent
                            anchors.leftMargin: root.cellPadding
                            anchors.rightMargin: root.cellPadding
                            sourceComponent: cell.column.cell
                            onLoaded: {
                                item.value = Qt.binding(() => cell.value);
                                item.row = Qt.binding(() => row.model);
                                item.column = cell.column;
                            }
                        }

                        RowLayout {
                            visible: !custom.active
                            anchors.fill: parent
                            anchors.leftMargin: root.cellPadding + (cell.index === 0 && (root.expandableRole || row.depth > 0) ? Kirigami.Units.iconSizes.small + cell.indent : 0)
                            anchors.rightMargin: root.cellPadding
                            spacing: Kirigami.Units.smallSpacing

                            Kirigami.Icon {
                                visible: cell.column.iconRole !== undefined
                                Layout.preferredWidth: Kirigami.Units.iconSizes.small
                                Layout.preferredHeight: Kirigami.Units.iconSizes.small
                                source: cell.column.iconRole ? (row.model[cell.column.iconRole] || "application-x-executable") : ""
                            }
                            QQC2.Label {
                                Layout.fillWidth: true
                                text: cell.column.text ? cell.column.text(cell.value, row.model) : (cell.value ?? "")
                                horizontalAlignment: cell.column.align === Qt.AlignRight ? Text.AlignRight : Text.AlignLeft
                                elide: Text.ElideRight
                                textFormat: Text.PlainText
                                font.features: cell.column.align === Qt.AlignRight ? { "tnum": 1 } : {}
                            }
                        }
                    }
                }
            }

            MouseArea {
                id: rowMouse
                anchors.fill: parent
                // Under the cells' own controls (a switch) and the chevron.
                z: -1
                hoverEnabled: true
                acceptedButtons: Qt.LeftButton | Qt.RightButton
                onPressed: mouse => {
                    list.currentIndex = row.index;
                    root.forceActiveFocus(Qt.MouseFocusReason);
                }
                onClicked: mouse => {
                    if (mouse.button === Qt.RightButton) {
                        const p = mapToItem(root, mouse.x, mouse.y);
                        root.contextMenuRequested(row.index, p.x, p.y);
                    }
                }
                onDoubleClicked: mouse => {
                    if (mouse.button === Qt.LeftButton) {
                        root.activated(row.index);
                    }
                }
            }
        }
    }

    QQC2.Label {
        visible: list.count === 0 && root.placeholderText.length > 0
        anchors.centerIn: rows
        width: list.width - Kirigami.Units.gridUnit * 2
        horizontalAlignment: Text.AlignHCenter
        wrapMode: Text.Wrap
        text: root.placeholderText
        opacity: 0.6
        textFormat: Text.PlainText
    }
}
