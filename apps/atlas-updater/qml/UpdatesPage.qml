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
    signal openChangelog
    signal appsChecked

    // Set by Main: when the app list was last checked (ms since the epoch).
    property double lastAppsCheck: 0

    title: qsTr("Updates")

    // The OS logo for the up-to-date state, if the icon theme has it.
    readonly property string logoIcon: osLogoProbe.valid ? osLogoProbe.source : (distroLogoProbe.valid ? "distributor-logo" : "checkmark")
    Kirigami.Icon {
        id: osLogoProbe
        visible: false
        width: 0
        height: 0
        source: page.backend.osLogo
    }
    Kirigami.Icon {
        id: distroLogoProbe
        visible: false
        width: 0
        height: 0
        source: "distributor-logo"
    }
    readonly property var apps: page.backend.appsJson.length > 0 ? JSON.parse(page.backend.appsJson) : []
    // Errors from these operations belong to the hero; the others (apps, crash
    // reports) stay in the banner at the top.
    readonly property bool heroError: ["check", "download", "restart", "rollback", "cancelRollback", "switch", "status", "timer"].indexOf(page.backend.errorOp) >= 0
    readonly property bool retryable: ["check", "status", "download"].indexOf(page.backend.errorOp) >= 0
    // A failed download is not retried while a restart waits (it could replace a queued rollback).
    readonly property bool canRetry: page.hasError && page.retryable && !(page.backend.errorOp === "download" && page.restartReady)
    readonly property bool hasError: page.backend.errorText.length > 0 && page.heroError && page.backend.restarting !== true && (!page.backend.loaded || !page.backend.busy)
    // busyOp is only meaningful together with busy.
    readonly property string busyOp: page.backend.busy ? page.backend.busyOp : ""
    readonly property bool downloading: page.busyOp === "download"
    // A change of state is running: show its own text, not "Checking".
    readonly property bool working: page.backend.restarting === true || ["rollback", "cancelRollback", "switch"].indexOf(page.busyOp) >= 0
    readonly property bool rollbackQueued: page.backend.rollbackQueued === true
    readonly property bool availableIsRollback: page.backend.availableIsRollback === true
    // The update failed its startup checks here and was undone (bad-image-digests).
    readonly property bool availableIsBad: page.backend.availableIsBad === true
    // Something is waiting for a restart (an update, a switch or a go back).
    readonly property bool restartReady: page.backend.hasStaged || page.backend.restartNeeded || page.rollbackQueued
    readonly property bool checking: (!page.backend.loaded && !page.hasError) || page.busyOp === "check"
    readonly property bool restarting: page.backend.restarting === true

    // The helper's progress while a download (or a channel switch) runs.
    readonly property string stage: page.downloading || page.busyOp === "switch" ? page.backend.progressStage : ""
    // Installing is counted in steps, not bytes: one step (bootc importing
    // the image) can take a minute, and a percentage would sit still that
    // long. So the bar moves on its own and the text names the step.
    readonly property real fraction: page.stage === "downloading" && page.backend.progressTotal > 0 ? Math.min(1, page.backend.progressDone / page.backend.progressTotal) : -1
    readonly property string progressText: {
        if (page.stage === "downloading") {
            if (page.fraction >= 0) {
                return qsTr("%1 of %2 · %3%").arg(page.size(page.backend.progressDone)).arg(page.size(page.backend.progressTotal)).arg(Math.floor(page.fraction * 100));
            }
            return page.backend.progressDone > 0 ? qsTr("%1 downloaded").arg(page.size(page.backend.progressDone)) : "";
        }
        if (page.stage === "installing") {
            var total = page.backend.progressTotal;
            var step = total > 0 ? qsTr("Step %1 of %2").arg(Math.min(total, page.backend.progressDone + 1)).arg(total) : "";
            var d = page.sentenceCase(page.backend.progressDetail);
            return d.length > 0 && step.length > 0 ? step + " · " + d : step + d;
        }
        return "";
    }
    // bootc writes "Importing Image", rpm-ostree "Writing OSTree commit":
    // capitalised words after the first are lowered, names such as OSTree kept.
    function sentenceCase(text) {
        return text.split(" ").map(function (w, i) {
            return i > 0 && /^[A-Z][a-z]+$/.test(w) ? w.toLowerCase() : w;
        }).join(" ");
    }

    // The clock for "Restart Tonight" (23:00 today, offered until 22:30) and
    // "Last checked: Today at …". Ticks while shown, and catches up when
    // the page shows again after the window sat in the tray.
    property double now: Date.now()
    onVisibleChanged: page.now = Date.now()
    onRestartReadyChanged: page.now = Date.now()
    Timer {
        interval: 60 * 1000
        repeat: true
        triggeredOnStart: true
        running: page.visible
        onTriggered: page.now = Date.now()
    }
    function tonightAt(nowMs) {
        var d = new Date(nowMs);
        d.setHours(23, 0, 0, 0);
        return nowMs < d.getTime() - 30 * 60 * 1000 ? Math.floor(d.getTime() / 1000) : 0;
    }
    readonly property double tonight: page.tonightAt(page.now)

    function size(bytes) {
        if (bytes >= 1e9) {
            return qsTr("%1 GB").arg((bytes / 1e9).toLocaleString(Qt.locale(), "f", 1));
        }
        if (bytes >= 1e6) {
            return qsTr("%1 MB").arg(Math.round(bytes / 1e6));
        }
        return qsTr("%1 kB").arg(Math.round(bytes / 1e3));
    }

    function copyDetails() {
        copier.text = page.backend.errorText;
        copier.selectAll();
        copier.copy();
        copier.text = "";
        copied.restart();
    }

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
        id: badDialog
        title: qsTr("Download %1 Anyway?").arg(page.backend.availableVersion)
        text: qsTr("This version didn't pass its startup checks on this computer, and AtlasOS went back to the version before it. It will probably fail again.")
        acceptText: qsTr("Download Anyway")
        focusReject: true
        // The state can change under an open dialog (a background read).
        onAccepted: {
            if (page.availableIsBad) {
                page.backend.downloadUpdate();
            }
        }
    }
    onAvailableIsBadChanged: {
        if (!page.availableIsBad) {
            badDialog.close();
        }
    }

    // Copy Details goes through this: QML has no clipboard of its own.
    TextEdit {
        id: copier
        visible: false
    }
    Timer {
        id: copied
        interval: 2000
    }

    ConfirmDialog {
        id: scheduleDialog
        title: qsTr("Pick a Restart Time")
        text: qsTr("Atlas Updater restarts your computer at this time. You get a notification 5 minutes before, and apps get to save their work first.")
        acceptText: qsTr("Schedule Restart")
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
        id: hero
        badgeUnits: 7
        ringWidth: 5
        Layout.topMargin: Kirigami.Units.gridUnit
        Layout.bottomMargin: Kirigami.Units.largeSpacing
        busy: page.checking || page.downloading || page.working
        progress: page.fraction
        showBar: page.downloading || page.stage.length > 0
        barText: page.progressText
        tint: {
            if (page.hasError) {
                return Kirigami.Theme.negativeTextColor;
            }
            if (page.restartReady) {
                return Kirigami.Theme.highlightColor;
            }
            if (page.backend.updateAvailable && page.availableIsBad) {
                return Kirigami.Theme.neutralTextColor;
            }
            return page.backend.updateAvailable ? Kirigami.Theme.highlightColor : Kirigami.Theme.positiveTextColor;
        }
        // Up to date: the OS logo in its own colours with a check badge,
        // not a tinted circle. The logo is `LOGO=` from os-release, then
        // `distributor-logo`, then a plain check.
        readonly property bool upToDate: iconName === page.logoIcon && page.logoIcon !== "checkmark"
        showTintCircle: !upToDate
        iconIsMask: !upToDate
        cornerBadgeIcon: upToDate ? "checkmark" : ""
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
                // An update goes up a version: an up arrow, not a download's.
                return Qt.resolvedUrl("icons/update-arrow.svg");
            }
            if (page.rollbackQueued) {
                return "edit-undo";
            }
            if (page.restartReady) {
                return "system-reboot";
            }
            if (page.backend.updateAvailable && page.availableIsBad) {
                return "dialog-warning";
            }
            if (page.backend.updateAvailable) {
                return "update-medium";
            }
            return page.logoIcon;
        }
        headline: {
            if (page.hasError) {
                return page.backend.loaded ? qsTr("Something went wrong") : qsTr("Could not read the system state");
            }
            if (!page.backend.loaded) {
                return qsTr("Reading the system state…");
            }
            if (page.working) {
                return page.backend.restarting === true ? qsTr("Restarting System…") : qsTr("Applying your change…");
            }
            if (page.checking) {
                return qsTr("Checking for updates…");
            }
            if (page.downloading) {
                return page.stage === "installing" ? qsTr("Installing %1…").arg(page.backend.availableVersion) : qsTr("Downloading %1…").arg(page.backend.availableVersion);
            }
            if (page.rollbackQueued) {
                return qsTr("Restart to go back to %1").arg(page.backend.rollbackTarget);
            }
            if (page.restartReady) {
                return qsTr("Restart to finish updating");
            }
            if (page.backend.updateAvailable && page.availableIsBad) {
                return qsTr("Version %1 didn't start properly").arg(page.backend.availableVersion);
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
            if (page.downloading) {
                return qsTr("Keep using your computer. The update is set up on the side and starts when you restart.");
            }
            if (page.checking || page.working) {
                // Never repeat the headline.
                return page.backend.busyText === hero.headline ? qsTr("This takes a moment.") : page.backend.busyText;
            }
            var when = page.backend.scheduledAt > 0 ? " " + qsTr("Restart scheduled for %1.").arg(Dates.atTime(page.backend.scheduledAt)) : "";
            if (page.rollbackQueued) {
                return qsTr("The previous version starts after the restart.") + when;
            }
            if (page.backend.hasStaged) {
                return qsTr("Version %1 is downloaded and waits for a restart.").arg(page.backend.stagedVersion) + when;
            }
            if (page.restartReady) {
                return qsTr("A restart finishes the change you made.") + when;
            }
            if (page.backend.updateAvailable && page.availableIsBad) {
                return qsTr("It failed its startup checks on this computer and was undone. It won't download on its own; a newer version will.");
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
            text: page.restarting ? qsTr("Restarting System…") : (page.rollbackQueued ? qsTr("Restart Now") : qsTr("Restart to Update"))
            visible: page.restartReady
            enabled: !page.backend.busy && !page.working
            onClicked: page.backend.restartNow()
        }
        SecondaryButton {
            text: qsTr("Restart Tonight")
            visible: page.restartReady && !page.working && page.backend.scheduledAt === 0 && page.tonight > 0
            QQC2.ToolTip.visible: hovered
            QQC2.ToolTip.text: qsTr("Restarts at %1. You get a notification 5 minutes before.").arg(new Date(page.tonight * 1000).toLocaleTimeString(Qt.locale(), Qt.locale().timeFormat(1)))
            QQC2.ToolTip.delay: Kirigami.Units.toolTipDelay
            onClicked: {
                var at = page.tonightAt(Date.now());
                page.now = Date.now();
                if (at > 0) {
                    page.backend.scheduleRestart(at);
                }
            }
        }
        SecondaryButton {
            text: qsTr("Pick a Time…")
            visible: page.restartReady && !page.working && page.backend.scheduledAt === 0
            onClicked: scheduleDialog.open()
        }

        SecondaryButton {
            text: qsTr("Cancel Scheduled Restart")
            visible: page.backend.scheduledAt > 0 && !page.working
            onClicked: page.backend.cancelRestart()
        }
        PrimaryButton {
            text: qsTr("Download Update")
            visible: page.backend.updateAvailable && !page.backend.hasStaged && !page.restartReady && !page.availableIsRollback && !page.availableIsBad && !page.hasError && !page.checking && !page.downloading
            enabled: !page.backend.busy
            onClicked: page.backend.downloadUpdate()
        }
        SecondaryButton {
            text: qsTr("Download Anyway")
            visible: page.backend.updateAvailable && !page.backend.hasStaged && !page.restartReady && (page.availableIsRollback || page.availableIsBad) && !page.hasError && !page.checking && !page.downloading
            enabled: !page.backend.busy
            onClicked: page.availableIsBad ? badDialog.open() : page.backend.downloadUpdate()
        }
        // One primary pill at most: with a restart waiting, Try again is secondary.
        PrimaryButton {
            text: qsTr("Try Again")
            visible: page.canRetry && !page.restartReady
            onClicked: page.retry()
        }
        SecondaryButton {
            text: qsTr("Try Again")
            visible: page.canRetry && page.restartReady
            onClicked: page.retry()
        }
        SecondaryButton {
            text: copied.running ? qsTr("Copied") : qsTr("Copy Details")
            Accessible.name: qsTr("Copy the error details")
            visible: page.hasError
            onClicked: page.copyDetails()
        }
        SecondaryButton {
            text: qsTr("Dismiss")
            Accessible.name: qsTr("Dismiss error")
            visible: page.hasError && !page.canRetry
            onClicked: page.backend.dismissMessages()
        }
        SecondaryButton {
            text: qsTr("Check for Updates")
            visible: !page.hasError && !page.restartReady && !page.checking && !page.downloading && !page.working
            enabled: !page.backend.busy
            onClicked: page.backend.checkForUpdate()
        }
    }

    QQC2.Label {
        Layout.fillWidth: true
        Layout.topMargin: -Kirigami.Units.smallSpacing
        Layout.bottomMargin: Kirigami.Units.largeSpacing
        visible: page.backend.lastChecked > 0 && page.backend.loaded && !page.hasError && !page.checking && !page.downloading && !page.working
        horizontalAlignment: Text.AlignHCenter
        font: Kirigami.Theme.smallFont
        opacity: 0.6
        text: qsTr("Last checked: %1").arg(Dates.relative(page.backend.lastChecked, page.now))
        textFormat: Text.PlainText
    }

    // ---- release notes ----
    Section {
        title: qsTr("What's New in %1").arg(page.backend.notesVersion)
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
                text: qsTr("Try Again")
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
            // "Ready to install" only once it is downloaded.
            title: !page.backend.hasStaged && page.backend.updateAvailable ? qsTr("Available") : qsTr("Ready to install")
            value: page.backend.hasStaged ? page.version(page.backend.stagedVersion, page.backend.stagedDate) : (page.backend.updateAvailable ? qsTr("%1, not downloaded yet").arg(page.backend.availableVersion) : qsTr("Nothing waiting"))
        }
        SectionRow {
            title: qsTr("Previous")
            value: page.backend.hasRollback ? page.version(page.backend.rollbackVersion, page.backend.rollbackDate) : qsTr("None")
        }
        SectionRow {
            title: qsTr("Changelog")
            subtitle: qsTr("What changed in each version this computer went through")
            chevron: true
            onClicked: page.openChangelog()
        }
    }

    // ---- what happens on its own ----
    Section {
        title: qsTr("Automatic Updates")
        footer: qsTr("Atlas Updater never restarts your computer without asking. A version you went back from isn't downloaded again on its own.")
        SectionRow {
            iconName: "update-none"
            title: qsTr("Updates download on their own")
            subtitle: qsTr("In the background, when you're online and not on a metered connection. Restart when it suits you.")
        }
        SectionRow {
            title: qsTr("Download app updates in the background")
            subtitle: page.backend.appsAuto ? qsTr("Apps update by themselves, but not on a metered connection or a low battery. An app that asks for new permissions waits for you.") : qsTr("Off: you get a notification when app updates are ready, and Update Apps installs them.")
            showSwitch: true
            switchChecked: page.backend.appsAuto
            onSwitchToggled: checked => page.backend.enableBackgroundApps(checked)
        }
    }

    // ---- flatpak apps ----
    Section {
        title: qsTr("App Updates")
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
                // Held back by a background round: say what it wants before
                // the user presses Update Apps.
                subtitle: appRow.modelData.asks
                    ? qsTr("Asks for new permissions: %1").arg(appRow.modelData.asks)
                    : (appRow.modelData.runtime ? qsTr("Runtime") : qsTr("App")) + " · " + appRow.modelData.branch + " · " + (appRow.modelData.system ? qsTr("System") : qsTr("User"))
                value: appRow.modelData.size_text
            }
        }
        SectionRow {
            title: qsTr("Check for App Updates")
            Accessible.name: qsTr("Check for App Updates")
            clickable: !page.backend.appsBusy
            chevron: true
            onClicked: page.backend.checkApps()
        }
        SectionRow {
            visible: page.apps.length > 0
            title: page.apps.length === 1 ? qsTr("1 app can be updated") : qsTr("%n apps can be updated", "", page.apps.length)
            SecondaryButton {
                text: qsTr("Update Apps")
                enabled: !page.backend.appsBusy
                onClicked: page.backend.updateApps()
            }
        }
    }
}
