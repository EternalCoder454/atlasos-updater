import QtQuick
import QtQuick.Templates as T
import org.kde.kirigami as Kirigami

// A right-click menu in the Atlas look: a rounded raised card with inset
// rows. Fill it with ContextMenuItem and ContextMenuSeparator, and open it
// with popup() at the pointer, or popup(item, x, y) from the keyboard.
//
//   ContextMenu {
//       id: menu
//       ContextMenuItem { text: qsTr("Details"); icon.name: "documentinfo" }
//       ContextMenuSeparator {}
//       ContextMenuItem { text: qsTr("End Task"); destructive: true }
//   }
T.Menu {
    id: control

    implicitWidth: Math.max(Kirigami.Units.gridUnit * 11, contentItem.implicitWidth + leftPadding + rightPadding)
    implicitHeight: contentItem.implicitHeight + topPadding + bottomPadding
    padding: Kirigami.Units.smallSpacing
    margins: Kirigami.Units.smallSpacing
    overlap: 1
    modal: false
    focus: true

    delegate: ContextMenuItem {}

    contentItem: ListView {
        implicitWidth: {
            let w = 0;
            for (let i = 0; i < count; ++i) {
                const item = itemAtIndex(i);
                if (item) {
                    w = Math.max(w, item.implicitWidth);
                }
            }
            return w;
        }
        implicitHeight: contentHeight
        model: control.contentModel
        interactive: Window.window ? contentHeight + control.topPadding + control.bottomPadding > Window.window.height : false
        clip: interactive
        currentIndex: control.currentIndex
        keyNavigationEnabled: true
        keyNavigationWraps: true
    }

    background: Rectangle {
        radius: 10
        color: Kirigami.Theme.backgroundColor.hslLightness > 0.5 ? Qt.lighter(Kirigami.Theme.backgroundColor, 1.5) : Qt.tint(Kirigami.Theme.backgroundColor, Qt.rgba(1, 1, 1, 0.08))
        border.width: 1
        border.color: Qt.alpha(Kirigami.Theme.textColor, 0.16)
    }

    enter: Transition {
        NumberAnimation {
            property: "opacity"
            from: 0
            to: 1
            duration: Kirigami.Units.shortDuration
        }
    }
    exit: Transition {
        NumberAnimation {
            property: "opacity"
            from: 1
            to: 0
            duration: Kirigami.Units.shortDuration
        }
    }
}
