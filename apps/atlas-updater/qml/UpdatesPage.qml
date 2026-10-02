pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Atlas.Ui
import "dates.js" as Dates

AtlasPage {
    id: page

    required property var backend

    signal openReports

    title: qsTr("Updates")

    readonly property var apps: page.backend.appsJson.length > 0 ? JSON.parse(page.backend.appsJson) : []
    readonly property bool hasError: page.backend.errorText.length > 0 && (!page.backend.loaded || !page.backend.busy)
    readonly property bool downloading: page.backend.busy && page.backend.updateAvailable && !page.backend.hasStaged
    readonly property bool checking: (!page.backend.loaded && !page.hasError) || (page.backend.busy && !page.downloading)

    function version(v, date) {
        return date.length > 0 ? qsTr("%1  (%2)").arg(v).arg(Dates.longDate(date)) : v;
    }

    Component.onCompleted: {
        backend.loadNotes();
        backend.checkApps();
    }

    // The staged/available version can change under us (inotify, a check).
    Connections {
        target: page.backend
        function onStagedVersionChanged() {
            page.backend.loadNotes();
        }
        function onAvailableVersionChanged() {
            page.backend.loadNotes();
        }
    }

    ConfirmDialog {
        id: scheduleDialog
        title: qsTr("Restart later")
        text: qsTr("Atlas Updater restarts your computer at this time. You get a notification 5 minutes before, and apps get to save their work first.")
        acceptText: qsTr("Schedule restart")
        onAccepted: {
            var d = new Date();
            if (dayBox.currentIndex === 1) {
                d.setDate(d.getDate() + 1);
            }
            d.setHours(hourSpin.value, minuteSpin.value, 0, 0);
            page.backend.scheduleRestart(Math.floor(d.getTime() / 1000));
        }
        onAboutToShow: {
            var d = new Date(Date.now() + 60 * 60 * 1000);
            dayBox.currentIndex = d.getDate() !== new Date().getDate() ? 1 : 0;
            hourSpin.value = d.getHours();
            minuteSpin.value = 0;
        }

        RowLayout {
            spacing: Kirigami.Units.largeSpacing
            QQC2.ComboBox {
                id: dayBox
                model: [qsTr("Today"), qsTr("Tomorrow")]
                Accessible.name: qsTr("Day")
            }
            QQC2.SpinBox {
                id: hourSpin
                from: 0
                to: 23
                editable: true
                Accessible.name: qsTr("Hour")
                textFromValue: v => (v < 10 ? "0" : "") + v
            }
            QQC2.Label {
                text: ":"
            }
            QQC2.SpinBox {
                id: minuteSpin
                from: 0
                to: 59
                stepSize: 5
                editable: true
                Accessible.name: qsTr("Minute")
                textFromValue: v => (v < 10 ? "0" : "") + v
            }
        }
    }

    Cards {
        Layout.fillWidth: true
        backend: page.backend
        showError: false
    }

    // A crash report is waiting.
    SecondaryButton {
        Layout.alignment: Qt.AlignLeft
        visible: page.backend.reportsCount > 0
        text: qsTr("A crash report is waiting. Review it")
        icon.name: "emblem-important"
        onClicked: page.openReports()
    }

    // ---- the system ----
    StatusHero {
        Layout.topMargin: Kirigami.Units.gridUnit
        Layout.bottomMargin: Kirigami.Units.largeSpacing
        busy: page.checking || page.downloading
        tint: page.hasError ? Kirigami.Theme.negativeTextColor : (page.backend.hasStaged ? Kirigami.Theme.highlightColor : (page.backend.updateAvailable ? Kirigami.Theme.highlightColor : Kirigami.Theme.positiveTextColor))
        iconName: {
            if (page.hasError) {
                return "dialog-error";
            }
            if (page.checking) {
                return "view-refresh";
            }
            if (page.downloading) {
                return "download";
            }
            if (page.backend.hasStaged) {
                return "system-reboot";
            }
            if (page.backend.updateAvailable) {
                return "update-medium";
            }
            return "checkmark";
        }
        headline: {
            if (page.hasError) {
                return page.backend.loaded ? qsTr("Could not check for updates") : qsTr("Could not read the system state");
            }
            if (!page.backend.loaded) {
                return qsTr("Reading the system state…");
            }
            if (page.checking) {
                return qsTr("Checking for updates…");
            }
            if (page.downloading) {
                return qsTr("Downloading %1…").arg(page.backend.availableVersion);
            }
            if (page.backend.hasStaged) {
                return qsTr("Restart to finish updating");
            }
            if (page.backend.updateAvailable) {
                return qsTr("AtlasOS %1 is available").arg(page.backend.availableVersion);
            }
            return qsTr("AtlasOS is up to date");
        }
        subtitle: {
            if (page.hasError) {
                return page.backend.errorText;
            }
            if (page.checking || page.downloading) {
                return page.backend.busyText;
            }
            if (page.backend.hasStaged) {
                return page.backend.scheduledAt > 0 ? qsTr("Version %1 is ready. Restart scheduled for %2.").arg(page.backend.stagedVersion).arg(Dates.shortDateTime(page.backend.scheduledAt)) : qsTr("Version %1 is downloaded and waits for a restart.").arg(page.backend.stagedVersion);
            }
            if (page.backend.updateAvailable) {
                return qsTr("It can be downloaded now. You are on %1.").arg(page.backend.currentVersion);
            }
            return qsTr("Version %1. Updates download in the background.").arg(page.backend.currentVersion);
        }

        PrimaryButton {
            text: qsTr("Restart to update")
            visible: page.backend.hasStaged || page.backend.restartNeeded
            enabled: !page.backend.busy
            onClicked: page.backend.restartNow()
        }
        MenuButton {
            text: qsTr("Restart later…")
            visible: page.backend.restartNeeded && page.backend.scheduledAt === 0
            QQC2.MenuItem {
                text: qsTr("Choose a time…")
                onTriggered: scheduleDialog.open()
            }
        }
        SecondaryButton {
            text: qsTr("Cancel scheduled restart")
            visible: page.backend.scheduledAt > 0
            onClicked: page.backend.cancelRestart()
        }
        PrimaryButton {
            text: qsTr("Download update")
            visible: page.backend.updateAvailable && !page.backend.hasStaged
            enabled: !page.backend.busy
            onClicked: page.backend.downloadUpdate()
        }
        PrimaryButton {
            text: qsTr("Try again")
            visible: page.hasError
            onClicked: page.backend.checkForUpdate()
        }
        SecondaryButton {
            text: qsTr("Check for updates")
            visible: !page.hasError && !page.backend.hasStaged && !page.backend.restartNeeded
            enabled: !page.backend.busy
            onClicked: page.backend.checkForUpdate()
        }
    }

    // ---- release notes ----
    Section {
        title: qsTr("What's new in %1").arg(page.backend.notesVersion)
        visible: page.backend.notesState !== "none" && page.backend.notesState !== ""

        SectionRow {
            visible: page.backend.notesState === "loading"
            title: qsTr("Loading release notes…")
        }
        SectionRow {
            visible: page.backend.notesState === "missing"
            title: qsTr("No release notes for this version")
        }
        SectionRow {
            visible: page.backend.notesState === "error"
            title: qsTr("Could not load the release notes")
            subtitle: page.backend.notesError.length > 0 ? page.backend.notesError : qsTr("Check your internet connection.")
            SecondaryButton {
                text: qsTr("Try again")
                onClicked: page.backend.loadNotes()
            }
        }
        Item {
            visible: page.backend.notesState === "ready"
            Layout.fillWidth: true
            implicitHeight: notes.implicitHeight + Kirigami.Units.largeSpacing * 2
            NotesText {
                id: notes
                anchors.fill: parent
                anchors.margins: Kirigami.Units.largeSpacing
                markdown: page.backend.notesText
                onLinkClicked: link => {
                    if (page.backend.isSafeLink(link)) {
                        Qt.openUrlExternally(link);
                    }
                }
            }
        }
    }

    // ---- versions ----
    Section {
        title: qsTr("Versions")
        SectionRow {
            title: qsTr("Current")
            value: page.backend.loaded ? page.version(page.backend.currentVersion, page.backend.currentDate) : "…"
        }
        SectionRow {
            title: qsTr("Ready to install")
            value: page.backend.hasStaged ? page.version(page.backend.stagedVersion, page.backend.stagedDate) : (page.backend.updateAvailable ? qsTr("%1, not downloaded yet").arg(page.backend.availableVersion) : qsTr("Nothing waiting"))
        }
        SectionRow {
            title: qsTr("Previous")
            value: page.backend.hasRollback ? page.version(page.backend.rollbackVersion, page.backend.rollbackDate) : qsTr("None")
        }
    }

    // ---- flatpak apps ----
    Section {
        title: qsTr("App updates")
        Layout.bottomMargin: Kirigami.Units.largeSpacing

        SectionRow {
            visible: page.backend.appsBusy
            title: page.backend.appsStatus
            QQC2.BusyIndicator {
                running: page.backend.appsBusy
                implicitWidth: Kirigami.Units.iconSizes.smallMedium
                implicitHeight: implicitWidth
            }
        }
        SectionRow {
            visible: page.backend.appsError.length > 0
            iconName: "dialog-error"
            title: page.backend.appsError
        }
        SectionRow {
            visible: !page.backend.appsBusy && page.apps.length === 0 && page.backend.appsError.length === 0
            iconName: "checkmark"
            title: qsTr("All apps are up to date.")
        }
        Repeater {
            model: page.apps
            delegate: SectionRow {
                id: appRow
                required property var modelData
                iconName: appRow.modelData.icon ? appRow.modelData.icon : (appRow.modelData.runtime ? "preferences-system-plugin" : "applications-all")
                title: appRow.modelData.name
                subtitle: (appRow.modelData.runtime ? qsTr("Runtime") : qsTr("App")) + " · " + appRow.modelData.branch + " · " + (appRow.modelData.system ? qsTr("System") : qsTr("User"))
                value: appRow.modelData.size_text
            }
        }
        SectionRow {
            title: qsTr("Check for app updates")
            Accessible.name: qsTr("Check for app updates")
            clickable: !page.backend.appsBusy
            chevron: true
            onClicked: page.backend.checkApps()
        }
        SectionRow {
            visible: page.apps.length > 0
            title: qsTr("%n app(s) can be updated", "", page.apps.length)
            SecondaryButton {
                text: qsTr("Update apps")
                enabled: !page.backend.appsBusy
                onClicked: page.backend.updateApps()
            }
        }
    }
}
