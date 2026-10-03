import QtQuick
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Atlas.Ui

AtlasPage {
    id: page

    required property var backend

    title: qsTr("Home")

    StatusHero {
        iconName: "checkmark"
        headline: qsTr("Hello from an Atlas app")
        subtitle: page.backend.status

        PrimaryButton {
            text: qsTr("Refresh")
            enabled: !page.backend.busy
            onClicked: page.backend.refresh()
        }
    }

    Section {
        title: qsTr("Example")
        SectionRow {
            title: qsTr("A row")
            subtitle: qsTr("Put your settings and lists in Sections.")
            value: qsTr("Value")
        }
    }

    Section {
        id: charts
        title: qsTr("Live Chart")

        // Made-up readings: a slow wave with some noise, and a second series
        // under it. A real app binds `values` to its backend's history.
        property list<real> load: []
        property list<real> other: []

        Timer {
            interval: 1000
            running: true
            repeat: true
            triggeredOnStart: true
            onTriggered: {
                const t = Date.now() / 1000;
                const next = (list, v) => list.concat([v]).slice(-60);
                charts.load = next(charts.load, 45 + 30 * Math.sin(t / 8) + Math.random() * 12);
                charts.other = next(charts.other, 20 + 10 * Math.sin(t / 5) + Math.random() * 6);
            }
        }

        LiveChart {
            Layout.fillWidth: true
            Layout.margins: Kirigami.Units.largeSpacing
            values: charts.load
            values2: charts.other
            maximum: 100
            label: qsTr("Load")
            valueText: charts.load.length ? Math.round(charts.load[charts.load.length - 1]) + "%" : ""
            topText: "100%"
            spanText: qsTr("60 seconds")
        }
    }
}
