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
    signal appsChecked

    // Set by Main: when the app list was last checked (ms since the epoch).
    property double lastAppsCheck: 0

    title: qsTr("Updates")

    readonly property var apps: page.backend.appsJson.length > 0 ? JSON.parse(page.backend.appsJson) : []
    // Errors from these operations belong to the hero; the others (apps, crash
    // reports) stay in the banner at the top.
    readonly property bool heroError: ["check", "download", "restart", "rollback", "cancelRollback", "switch", "status", "timer"].indexOf(page.backend.errorOp) >= 0
    readonly property bool retryable: ["check", "status", "download"].indexOf(page.backend.errorOp) >= 0
    // A failed download is not retried over a queued rollback.
    readonly property bool canRetry: page.hasError && page.retryable && !(page.backend.errorOp === "download" && page.rollbackQueued)
    readonly property bool hasError: page.backend.errorText.length > 0 && page.heroError && (!page.backend.loaded || !page.backend.busy)
    // busyOp is only meaningful together with busy.
    readonly property string busyOp: page.backend.busy ? page.backend.busyOp : ""
    readonly property bool downloading: page.busyOp === "download"
    // A change of state is running: show its own text, not "Checking".
    readonly property bool working: page.backend.restarting === true || ["rollback", "cancelRollback", "switch"].indexOf(page.busyOp) >= 0
    readonly property bool rollbackQueued: page.backend.rollbackQueued === true
    readonly property bool availableIsRollback: page.backend.availableIsRollback === true
    // Something is waiting for a restart (an update, a switch or a go back).
    readonly property bool restartReady: page.backend.hasStaged || page.backend.restartNeeded || page.rollbackQueued
    readonly property bool checking: (!page.backend.loaded && !page.hasError) || page.busyOp === "check"

    function retry() {
        if (page.backend.errorOp === "download") {
            page.backend.downloadUpdate();
        } else if (!page.backend.loaded) {
            // refreshStatus is silent: clear the old error first.
            page.backend.dismissMessages();
            page.backend.refreshStatus();
        } else {
            page.backend.checkForUpdate();
        }
    }

    function version(v, date) {
        return date.length > 0 ? qsTr("%1  (%2)").arg(v).arg(Dates.longDate(date)) : v;
    }

    Component.onCompleted: {
        backend.loadNotes();
        if (Date.now() - page.lastAppsCheck > 10 * 60 * 1000) {
            page.appsChecked();
            backend.checkApps();
        }
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
        closeOnAccept: false
        property string problem: ""
        onAccepted: {
            var d = new Date();
            if (dayBox.currentIndex === 1) {
                d.setDate(d.getDate() + 1);
            }
            d.setHours(hourSpin.value, minuteSpin.value, 0, 0);
            if (d.getTime() <= Date.now()) {
                scheduleDialog.problem = qsTr("That time has already passed. Pick a later time.");
                return;
            }
            page.backend.scheduleRestart(Math.floor(d.getTime() / 1000));
            scheduleDialog.close();
        }
        onAboutToShow: {
            scheduleDialog.problem = "";
            var d = new Date(Date.now() + 60 * 60 * 1000);
            dayBox.currentIndex = d.getDate() !== new Date().getDate() ? 1 : 0;
            hourSpin.value = d.getHours();
            minuteSpin.value = 0;
        }

        QQC2.Label {
            Layout.fillWidth: true
            visible: scheduleDialog.problem.length > 0
            text: scheduleDialog.problem
            color: Kirigami.Theme.negativeTextColor
            wrapMode: Text.Wrap
            Accessible.role: Accessible.AlertMessage
        }
        RowLayout {
            spacing: Kirigami.Units.largeSpacing
            QQC2.ComboBox {
                id: dayBox
                onActivated: scheduleDialog.problem = ""
                model: [qsTr("Today"), qsTr("Tomorrow")]
                Accessible.name: qsTr("Day")
            }
            QQC2.SpinBox {
                id: hourSpin
                from: 0
                to: 23
                editable: true
                onValueChanged: scheduleDialog.problem = ""
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
                onValueChanged: scheduleDialog.problem = ""
                Accessible.name: qsTr("Minute")
                textFromValue: v => (v < 10 ? "0" : "") + v
            }
        }
    }

    Cards {
        Layout.fillWidth: true
        backend: page.backend
        showError: !page.hasError
        showBusy: false
    }

    // A crash report is waiting.
    Section {
        visible: page.backend.reportsCount > 0
        SectionRow {
            iconName: "data-warning"
            title: page.backend.reportsCount === 1 ? qsTr("1 crash report waiting") : qsTr("%n crash reports waiting", "", page.backend.reportsCount)
            subtitle: qsTr("Review them. Nothing is sent unless you say so.")
            chevron: true
            onClicked: page.openReports()
        }
    }

    // ---- the system ----
    StatusHero {
        Layout.topMargin: Kirigami.Units.gridUnit
        Layout.bottomMargin: Kirigami.Units.largeSpacing
        busy: page.checking || page.downloading || page.working
        tint: page.hasError ? Kirigami.Theme.negativeTextColor : (page.restartReady || page.backend.updateAvailable ? Kirigami.Theme.highlightColor : Kirigami.Theme.positiveTextColor)
        iconName: {
            if (page.hasError) {
                return "dialog-error";
            }
            if (page.working) {
                return "view-refresh";
            }
            if (page.checking) {
                return "view-refresh";
            }
            if (page.downloading) {
                return "download";
            }
            if (page.rollbackQueued) {
                return "edit-undo";
            }
            if (page.restartReady) {
                return "system-reboot";
            }
            if (page.backend.updateAvailable) {
                return "update-medium";
            }
            return "checkmark";
        }
        headline: {
            if (page.hasError) {
                return page.backend.loaded ? qsTr("Something went wrong") : qsTr("Could not read the system state");
            }
            if (!page.backend.loaded) {
                return qsTr("Reading the system state…");
            }
            if (page.working) {
                return page.backend.restarting === true ? qsTr("Restarting…") : qsTr("Applying your change…");
            }
            if (page.checking) {
                return qsTr("Checking for updates…");
            }
            if (page.downloading) {
                return qsTr("Downloading %1…").arg(page.backend.availableVersion);
            }
            if (page.rollbackQueued) {
                return qsTr("Restart to go back to %1").arg(page.backend.rollbackTarget);
            }
            if (page.restartReady) {
                return qsTr("Restart to finish updating");
            }
            if (page.backend.updateAvailable && page.availableIsRollback) {
                return qsTr("You went back from version %1").arg(page.backend.availableVersion);
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
            if (page.backend.restarting === true) {
                return qsTr("Saving your session…");
            }
            if (page.checking || page.downloading || page.working) {
                return page.backend.busyText;
            }
            var when = page.backend.scheduledAt > 0 ? " " + qsTr("Restart scheduled for %1.").arg(Dates.shortDateTime(page.backend.scheduledAt)) : "";
            if (page.rollbackQueued) {
                return qsTr("The previous version starts after the restart.") + when;
            }
            if (page.backend.hasStaged) {
                return qsTr("Version %1 is downloaded and waits for a restart.").arg(page.backend.stagedVersion) + when;
            }
            if (page.restartReady) {
                return qsTr("A restart finishes the change you made.") + when;
            }
            if (page.backend.updateAvailable && page.availableIsRollback) {
                return qsTr("It won't download on its own; download it again if you like.");
            }
            if (page.backend.updateAvailable) {
                return qsTr("It can be downloaded now. You are on %1.").arg(page.backend.currentVersion);
            }
            return qsTr("Version %1. Updates download in the background.").arg(page.backend.currentVersion);
        }

        PrimaryButton {
            text: page.rollbackQueued ? qsTr("Restart now") : qsTr("Restart to update")
            visible: page.restartReady
            enabled: !page.backend.busy && !page.working
            onClicked: page.backend.restartNow()
        }
        SecondaryButton {
            text: qsTr("Restart later…")
            visible: page.restartReady && page.backend.scheduledAt === 0
            onClicked: scheduleDialog.open()
        }
        SecondaryButton {
            text: qsTr("Cancel scheduled restart")
            visible: page.backend.scheduledAt > 0
            onClicked: page.backend.cancelRestart()
        }
        PrimaryButton {
            text: qsTr("Download update")
            visible: page.backend.updateAvailable && !page.backend.hasStaged && !page.restartReady && !page.availableIsRollback && !page.hasError && !page.checking && !page.downloading
            enabled: !page.backend.busy
            onClicked: page.backend.downloadUpdate()
        }
        SecondaryButton {
            text: qsTr("Download anyway")
            visible: page.backend.updateAvailable && !page.backend.hasStaged && !page.restartReady && page.availableIsRollback && !page.hasError && !page.checking && !page.downloading
            enabled: !page.backend.busy
            onClicked: page.backend.downloadUpdate()
        }
        // One primary pill at most: with a restart waiting, Try again is secondary.
        PrimaryButton {
            text: qsTr("Try again")
            visible: page.canRetry && !page.restartReady
            onClicked: page.retry()
        }
        SecondaryButton {
            text: qsTr("Try again")
            visible: page.canRetry && page.restartReady
            onClicked: page.retry()
        }
        SecondaryButton {
            text: qsTr("Dismiss")
            Accessible.name: qsTr("Dismiss error")
            visible: page.hasError && !page.canRetry
            onClicked: page.backend.dismissMessages()
        }
        SecondaryButton {
            text: qsTr("Check for updates")
            visible: !page.hasError && !page.restartReady
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
                Accessible.name: qsTr("Try again, release notes")
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
                html: page.backend.notesHtml
                plain: page.backend.notesPlain
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
            title: page.apps.length === 1 ? qsTr("1 app can be updated") : qsTr("%n apps can be updated", "", page.apps.length)
            SecondaryButton {
                text: qsTr("Update apps")
                enabled: !page.backend.appsBusy
                onClicked: page.backend.updateApps()
            }
        }
    }
}
