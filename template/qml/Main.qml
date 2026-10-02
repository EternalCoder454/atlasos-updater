import QtQuick
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami

QQC2.ApplicationWindow {
    id: root

    // Set from main.cpp through setInitialProperties().
    required property var backend

    title: qsTr("Atlas App")
    width: Kirigami.Units.gridUnit * 40
    height: Kirigami.Units.gridUnit * 30
    visible: true
    color: Kirigami.Theme.backgroundColor
    LayoutMirroring.enabled: Qt.application.layoutDirection === Qt.RightToLeft
    LayoutMirroring.childrenInherit: true

    MainPage {
        anchors.fill: parent
        backend: root.backend
    }
}
