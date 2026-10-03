import QtQuick
import QtQuick.Layouts

// A sidebar entry that folds out into sub-entries: Disk ▸ its drives,
// Network ▸ its interfaces. Put SidebarItems with `sub: true` inside. The
// header toggles the group; while folded it shows as selected if one of its
// entries is, so the sidebar still says where you are.
//
// A compact sidebar (icons only) has no room for sub-entries: the group is
// its header's icon alone, and a click on it emits `activated` for the app to
// open the group's first entry.
//
//   SidebarGroup {
//       text: qsTr("Disk")
//       iconName: "drive-harddisk-symbolic"
//       Repeater {
//           model: disks
//           SidebarItem { sub: true; text: model.label; value: model.rate; ... }
//       }
//   }
ColumnLayout {
    id: root

    property alias text: header.text
    property string iconName
    property alias value: header.value
    property alias compact: header.compact
    property alias tintIcon: header.tintIcon
    property bool expanded: true
    default property alias items: entries.data
    // True when one of the entries is selected.
    readonly property bool holdsSelection: {
        const c = entries.children;
        for (let i = 0; i < c.length; ++i) {
            if (c[i].selected === true) {
                return true;
            }
        }
        return false;
    }

    signal toggled
    signal activated

    Layout.fillWidth: true
    // The gap between entries; match the sidebar column it sits in.
    spacing: 2

    SidebarItem {
        id: header
        Layout.fillWidth: true
        icon.name: root.iconName
        disclosure: true
        expanded: root.expanded
        selected: (!root.expanded || root.compact) && root.holdsSelection
        onClicked: {
            if (root.compact) {
                root.activated();
                return;
            }
            root.expanded = !root.expanded;
            root.toggled();
        }
    }

    ColumnLayout {
        id: entries
        Layout.fillWidth: true
        visible: root.expanded && !root.compact
        spacing: root.spacing
    }
}
