import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as QQC2
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

    SearchField {
        id: search
        Layout.alignment: Qt.AlignRight
        placeholderText: qsTr("Search Apps")
        onQueryChanged: table.rebuild()
    }

    // A table with made-up processes. A real app gives it a Rust
    // QAbstractItemModel that sorts itself and moves rows; this one sorts a
    // ListModel in JavaScript.
    DataTable {
        id: table
        Layout.preferredHeight: Kirigami.Units.gridUnit * 16
        sortRole: "cpu"
        depthRole: "depth"
        expandableRole: "expandable"
        expandedRole: "expanded"
        placeholderText: qsTr("No Apps Running")
        columns: [
            { title: qsTr("Name"), role: "name", fill: true, iconRole: "icon" },
            { title: qsTr("State"), role: "state", width: 6, cell: stateCell },
            { title: qsTr("CPU"), role: "cpu", width: 5, align: Qt.AlignRight, heat: 100, text: v => v.toFixed(1) + "%" },
            { title: qsTr("Memory"), role: "memory", width: 6, align: Qt.AlignRight, text: v => v.toFixed(0) + " MiB" }
        ]
        model: ListModel {
            id: apps
        }

        property var groupOpen: true
        readonly property var names: [["Firefox", "firefox"], ["Konsole", "utilities-terminal"], ["Dolphin", "system-file-manager"], ["Kate", "kate"], ["Atlas Updater", "system-software-update"], ["KWin", "kwin"], ["Plasma Shell", "plasma"], ["PipeWire", "audio-card"], ["Discover", "plasmadiscover"], ["Spectacle", "spectacle"], ["Okular", "okular"], ["Gwenview", "gwenview"]]
        property var load: names.map((_, i) => ({ cpu: (i * 7) % 30, memory: 80 + i * 37 }))

        function rebuild() {
            const order = table.sortOrder === Qt.AscendingOrder ? 1 : -1;
            const rows = table.names.map((n, i) => ({ name: n[0], icon: n[1], state: i === 5 ? "stopped" : "running", cpu: table.load[i].cpu, memory: table.load[i].memory, depth: 0, expandable: i === 0, expanded: i === 0 && table.groupOpen }));
            const q = search.query.toLowerCase();
            if (q) {
                rows.splice(0, rows.length, ...rows.filter(r => r.name.toLowerCase().includes(q)));
            }
            rows.sort((a, b) => (a[table.sortRole] < b[table.sortRole] ? -1 : a[table.sortRole] > b[table.sortRole] ? 1 : 0) * order);
            // Firefox's processes, under it while it is open.
            const at = rows.findIndex(r => r.expandable);
            if (table.groupOpen && at >= 0) {
                rows.splice(at + 1, 0, { name: "Web Content", icon: "", state: "running", cpu: 4.2, memory: 310, depth: 1, expandable: false, expanded: false }, { name: "GPU Process", icon: "", state: "running", cpu: 1.1, memory: 95, depth: 1, expandable: false, expanded: false });
            }
            const current = table.currentIndex >= 0 && table.currentIndex < apps.count ? apps.get(table.currentIndex).name : "";
            apps.clear();
            for (const r of rows) {
                apps.append(r);
            }
            table.currentIndex = rows.findIndex(r => r.name === current);
        }

        onSortRoleChanged: rebuild()
        onSortOrderChanged: rebuild()
        onContextMenuRequested: (row, x, y) => rowMenu.popup(table, x, y)
        onToggleRequested: row => {
            groupOpen = !groupOpen;
            rebuild();
        }
        Component.onCompleted: rebuild()

        Timer {
            interval: 1000
            running: true
            repeat: true
            onTriggered: {
                table.load = table.load.map((l, i) => ({ cpu: Math.max(0, Math.min(100, l.cpu + (Math.random() - 0.5) * 12 * (i % 3 + 1))), memory: l.memory }));
                // Hold the order still under the pointer, and while the
                // menu is open on a row.
                if (!table.pointerInside && !rowMenu.opened) {
                    table.rebuild();
                }
            }
        }

        ContextMenu {
            id: rowMenu
            ContextMenuItem {
                text: qsTr("Details")
                icon.name: "documentinfo"
            }
            ContextMenuItem {
                text: qsTr("Open File Location")
                icon.name: "folder-open"
            }
            ContextMenuSeparator {}
            ContextMenuItem {
                text: qsTr("Stop")
                icon.name: "media-playback-pause"
            }
            ContextMenuItem {
                text: qsTr("End Task")
                icon.name: "process-stop"
                shortcutText: qsTr("Del")
                destructive: true
            }
        }

        Component {
            id: stateCell
            Row {
                property var value
                property var row
                property var column
                spacing: Kirigami.Units.smallSpacing
                Rectangle {
                    anchors.verticalCenter: parent.verticalCenter
                    width: Kirigami.Units.gridUnit * 0.5
                    height: width
                    radius: width / 2
                    color: parent.value === "running" ? Kirigami.Theme.positiveTextColor : Kirigami.Theme.neutralTextColor
                }
                QQC2.Label {
                    anchors.verticalCenter: parent.verticalCenter
                    text: parent.value === "running" ? qsTr("Running") : qsTr("Stopped")
                    opacity: 0.8
                }
            }
        }
    }
}
