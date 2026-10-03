import QtQuick
import QtQuick.Templates as T
import org.kde.kirigami as Kirigami

// A thin rule between groups of ContextMenuItems.
T.MenuSeparator {
    implicitWidth: Kirigami.Units.gridUnit * 8
    implicitHeight: Kirigami.Units.smallSpacing * 2 + 1
    topPadding: Kirigami.Units.smallSpacing
    bottomPadding: Kirigami.Units.smallSpacing
    leftPadding: Kirigami.Units.largeSpacing
    rightPadding: Kirigami.Units.largeSpacing

    contentItem: Rectangle {
        implicitHeight: 1
        color: Qt.alpha(Kirigami.Theme.textColor, 0.12)
    }
}
