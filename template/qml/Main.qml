import QtQuick
import org.kde.kirigami as Kirigami

Kirigami.ApplicationWindow {
    id: root

    // Set from main.cpp through setInitialProperties().
    required property var backend

    title: qsTr("Atlas App")
    width: Kirigami.Units.gridUnit * 40
    height: Kirigami.Units.gridUnit * 30

    pageStack.initialPage: MainPage {
        backend: root.backend
    }
}
