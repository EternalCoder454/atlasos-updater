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
        property list<real> cores: []

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
                charts.cores = Array.from({ length: 16 }, (_, i) => Math.max(0, Math.min(100, 50 + 45 * Math.sin(t / 6 + i) + Math.random() * 10 - 5)));
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

    Section {
        title: qsTr("Usage Bars")

        ColumnLayout {
            Layout.fillWidth: true
            Layout.margins: Kirigami.Units.largeSpacing
            spacing: Kirigami.Units.largeSpacing

            // Sizes in GiB here; a real app passes bytes and formats them.
            UsageBar {
                total: 32
                values: [17.2, 9.4]
                labels: [qsTr("Used"), qsTr("Cached"), qsTr("Free")]
                texts: ["17.2 GiB", "9.4 GiB", "5.4 GiB"]
            }

            MiniBars {
                values: charts.cores
            }
        }
    }

    Section {
        title: qsTr("Sidebar")

        // A sidebar column as an app would lay it out, here inside a Section.
        ColumnLayout {
            id: sidebar
            property string current: "nvme0n1"

            Layout.margins: Kirigami.Units.largeSpacing
            Layout.preferredWidth: Kirigami.Units.gridUnit * 13
            spacing: 2

            SidebarItem {
                Layout.fillWidth: true
                text: qsTr("Processor")
                icon.name: "cpu"
                tintIcon: false
                value: charts.load.length ? Math.round(charts.load[charts.load.length - 1]) + "%" : ""
                selected: sidebar.current === "cpu"
                onClicked: sidebar.current = "cpu"
            }
            SidebarGroup {
                text: qsTr("Disk")
                iconName: "drive-harddisk-symbolic"

                Repeater {
                    model: [
                        { name: "nvme0n1", label: "Samsung 990 Pro", rate: "12 MB/s" },
                        { name: "sda", label: "Backup", rate: "0 B/s" }
                    ]

                    SidebarItem {
                        required property var modelData
                        Layout.fillWidth: true
                        sub: true
                        text: modelData.label
                        icon.name: "drive-harddisk-symbolic"
                        value: modelData.rate
                        selected: sidebar.current === modelData.name
                        onClicked: sidebar.current = modelData.name
                    }
                }
            }
            SidebarGroup {
                text: qsTr("Network")
                iconName: "network-wired-symbolic"
                expanded: false

                SidebarItem {
                    Layout.fillWidth: true
                    sub: true
                    text: "enp5s0"
                    icon.name: "network-wired-symbolic"
                    value: "1.4 MB/s"
                    selected: sidebar.current === "enp5s0"
                    onClicked: sidebar.current = "enp5s0"
                }
            }
        }
    }
}
