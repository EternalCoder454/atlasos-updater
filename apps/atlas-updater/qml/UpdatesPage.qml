pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import "dates.js" as Dates

Kirigami.ScrollablePage {
    id: page

    required property var backend

    signal openReports

    title: qsTr("Updates")

    readonly property var apps: page.backend.appsJson.length > 0 ? JSON.parse(page.backend.appsJson) : []

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

    Kirigami.PromptDialog {
        id: scheduleDialog
        title: qsTr("Restart later")
        standardButtons: Kirigami.Dialog.NoButton
        customFooterActions: [
            Kirigami.Action {
                text: qsTr("Schedule restart")
                icon.name: "appointment-new"
                onTriggered: {
                    var d = new Date();
                    if (dayBox.currentIndex === 1) {
                        d.setDate(d.getDate() + 1);
                    }
                    d.setHours(hourSpin.value, minuteSpin.value, 0, 0);
                    page.backend.scheduleRestart(Math.floor(d.getTime() / 1000));
                    scheduleDialog.close();
                }
            },
            Kirigami.Action {
                text: qsTr("Cancel")
                icon.name: "dialog-cancel"
                onTriggered: scheduleDialog.close()
            }
        ]
        onOpened: {
            var d = new Date(Date.now() + 60 * 60 * 1000);
            dayBox.currentIndex = d.getDate() !== new Date().getDate() ? 1 : 0;
            hourSpin.value = d.getHours();
            minuteSpin.value = 0;
        }

        ColumnLayout {
            spacing: Kirigami.Units.largeSpacing
            QQC2.Label {
                Layout.fillWidth: true
                wrapMode: Text.Wrap
                text: qsTr("Atlas Updater restarts your computer at this time. You get a notification 5 minutes before, and apps get to save their work first.")
            }
            Kirigami.FormLayout {
                Layout.fillWidth: true
                QQC2.ComboBox {
                    id: dayBox
                    Kirigami.FormData.label: qsTr("Day:")
                    model: [qsTr("Today"), qsTr("Tomorrow")]
                    Accessible.name: qsTr("Day")
                }
                RowLayout {
                    Kirigami.FormData.label: qsTr("Time:")
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
        }
    }

    ColumnLayout {
        spacing: Kirigami.Units.largeSpacing

        Kirigami.InlineMessage {
            Layout.fillWidth: true
            type: Kirigami.MessageType.Information
            visible: page.backend.reportsCount > 0
            text: qsTr("A crash report is waiting. You can review it, and nothing is sent unless you say so.")
            actions: [
                Kirigami.Action {
                    text: qsTr("Review report")
                    icon.name: "tools-report-bug"
                    onTriggered: page.openReports()
                }
            ]
        }

        Cards {
            Layout.fillWidth: true
            backend: page.backend
        }

        // ---- the system ----
        Kirigami.AbstractCard {
            Layout.fillWidth: true
            contentItem: ColumnLayout {
                spacing: Kirigami.Units.largeSpacing

                RowLayout {
                    spacing: Kirigami.Units.largeSpacing
                    Kirigami.Icon {
                        source: page.backend.hasStaged ? "update-high" : (page.backend.updateAvailable ? "update-medium" : "update-none")
                        Layout.preferredWidth: Kirigami.Units.iconSizes.huge
                        Layout.preferredHeight: Kirigami.Units.iconSizes.huge
                    }
                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 0
                        Kirigami.Heading {
                            Layout.fillWidth: true
                            level: 2
                            wrapMode: Text.Wrap
                            text: {
                                if (!page.backend.loaded) {
                                    return qsTr("Reading the system state…");
                                }
                                if (page.backend.hasStaged) {
                                    return qsTr("Restart to finish updating");
                                }
                                if (page.backend.updateAvailable) {
                                    return qsTr("An update is available");
                                }
                                return qsTr("Your system is up to date");
                            }
                        }
                        QQC2.Label {
                            Layout.fillWidth: true
                            wrapMode: Text.Wrap
                            opacity: 0.7
                            visible: page.backend.loaded
                            text: {
                                if (page.backend.hasStaged) {
                                    return qsTr("Version %1 is downloaded and waits for a restart.").arg(page.backend.stagedVersion);
                                }
                                if (page.backend.updateAvailable) {
                                    return qsTr("Version %1 can be downloaded now.").arg(page.backend.availableVersion);
                                }
                                return qsTr("Updates download in the background. Check now if you like.");
                            }
                        }
                    }
                }

                Kirigami.FormLayout {
                    Layout.fillWidth: true
                    wideMode: true
                    QQC2.Label {
                        Kirigami.FormData.label: qsTr("Current:")
                        text: page.backend.loaded ? page.version(page.backend.currentVersion, page.backend.currentDate) : "…"
                        wrapMode: Text.Wrap
                        Layout.fillWidth: true
                    }
                    QQC2.Label {
                        Kirigami.FormData.label: qsTr("Ready to install:")
                        text: page.backend.hasStaged ? page.version(page.backend.stagedVersion, page.backend.stagedDate) : (page.backend.updateAvailable ? qsTr("%1, not downloaded yet").arg(page.backend.availableVersion) : qsTr("Nothing waiting"))
                        wrapMode: Text.Wrap
                        Layout.fillWidth: true
                    }
                    QQC2.Label {
                        Kirigami.FormData.label: qsTr("Previous:")
                        text: page.backend.hasRollback ? page.version(page.backend.rollbackVersion, page.backend.rollbackDate) : qsTr("None")
                        wrapMode: Text.Wrap
                        Layout.fillWidth: true
                    }
                }

                Flow {
                    Layout.fillWidth: true
                    spacing: Kirigami.Units.smallSpacing
                    QQC2.Button {
                        text: qsTr("Restart to update")
                        icon.name: "system-reboot"
                        visible: page.backend.restartNeeded
                        highlighted: true
                        enabled: !page.backend.busy
                        onClicked: page.backend.restartNow()
                    }
                    QQC2.Button {
                        text: qsTr("Restart later…")
                        icon.name: "chronometer"
                        visible: page.backend.restartNeeded && page.backend.scheduledAt === 0
                        onClicked: scheduleDialog.open()
                    }
                    QQC2.Button {
                        text: qsTr("Download update")
                        icon.name: "download"
                        visible: page.backend.updateAvailable && !page.backend.hasStaged
                        highlighted: true
                        enabled: !page.backend.busy
                        onClicked: page.backend.downloadUpdate()
                    }
                    QQC2.Button {
                        text: qsTr("Check for updates")
                        icon.name: "view-refresh"
                        enabled: !page.backend.busy
                        onClicked: page.backend.checkForUpdate()
                    }
                }

                RowLayout {
                    Layout.fillWidth: true
                    visible: page.backend.scheduledAt > 0
                    spacing: Kirigami.Units.largeSpacing
                    Kirigami.Icon {
                        source: "chronometer"
                        Layout.preferredWidth: Kirigami.Units.iconSizes.smallMedium
                        Layout.preferredHeight: Kirigami.Units.iconSizes.smallMedium
                    }
                    QQC2.Label {
                        Layout.fillWidth: true
                        wrapMode: Text.Wrap
                        text: qsTr("Restart scheduled for %1").arg(Dates.shortDateTime(page.backend.scheduledAt))
                    }
                    QQC2.Button {
                        text: qsTr("Cancel scheduled restart")
                        icon.name: "dialog-cancel"
                        onClicked: page.backend.cancelRestart()
                    }
                }
            }
        }

        // ---- release notes ----
        Kirigami.AbstractCard {
            Layout.fillWidth: true
            visible: page.backend.notesState !== "none" && page.backend.notesState !== ""
            header: Kirigami.Heading {
                level: 3
                text: qsTr("What's new in %1").arg(page.backend.notesVersion)
                wrapMode: Text.Wrap
            }
            contentItem: ColumnLayout {
                spacing: Kirigami.Units.smallSpacing
                RowLayout {
                    visible: page.backend.notesState === "loading"
                    QQC2.BusyIndicator {
                        running: page.backend.notesState === "loading"
                    }
                    QQC2.Label {
                        text: qsTr("Loading release notes…")
                    }
                }
                QQC2.Label {
                    Layout.fillWidth: true
                    visible: page.backend.notesState === "missing"
                    wrapMode: Text.Wrap
                    text: qsTr("No release notes for this version")
                    opacity: 0.7
                }
                RowLayout {
                    visible: page.backend.notesState === "error"
                    QQC2.Label {
                        Layout.fillWidth: true
                        wrapMode: Text.Wrap
                        text: qsTr("Could not load the release notes. Check your internet connection.")
                    }
                    QQC2.Button {
                        text: qsTr("Try again")
                        icon.name: "view-refresh"
                        onClicked: {
                            page.backend.loadNotes();
                        }
                    }
                }
                QQC2.Label {
                    Layout.fillWidth: true
                    visible: page.backend.notesState === "ready"
                    text: page.backend.notesText
                    textFormat: Text.MarkdownText
                    wrapMode: Text.Wrap
                    onLinkActivated: link => Qt.openUrlExternally(link)
                }
            }
        }

        // ---- flatpak apps ----
        Kirigami.AbstractCard {
            Layout.fillWidth: true
            Layout.bottomMargin: Kirigami.Units.largeSpacing
            header: Kirigami.Heading {
                level: 3
                text: qsTr("App updates")
            }
            contentItem: ColumnLayout {
                spacing: Kirigami.Units.smallSpacing

                RowLayout {
                    visible: page.backend.appsBusy
                    spacing: Kirigami.Units.largeSpacing
                    QQC2.BusyIndicator {
                        running: page.backend.appsBusy
                    }
                    QQC2.Label {
                        Layout.fillWidth: true
                        wrapMode: Text.Wrap
                        text: page.backend.appsStatus
                    }
                }
                Kirigami.InlineMessage {
                    Layout.fillWidth: true
                    type: Kirigami.MessageType.Error
                    visible: page.backend.appsError.length > 0
                    text: page.backend.appsError
                }
                QQC2.Label {
                    Layout.fillWidth: true
                    visible: !page.backend.appsBusy && page.apps.length === 0 && page.backend.appsError.length === 0
                    text: qsTr("All apps are up to date.")
                    opacity: 0.7
                }
                Repeater {
                    model: page.apps
                    delegate: QQC2.ItemDelegate {
                        id: appRow
                        required property var modelData
                        Layout.fillWidth: true
                        hoverEnabled: false
                        down: false
                        text: appRow.modelData.name
                        icon.name: appRow.modelData.runtime ? "preferences-system-plugin" : "applications-all"
                        contentItem: RowLayout {
                            spacing: Kirigami.Units.largeSpacing
                            Kirigami.Icon {
                                source: appRow.icon.name
                                Layout.preferredWidth: Kirigami.Units.iconSizes.smallMedium
                                Layout.preferredHeight: Kirigami.Units.iconSizes.smallMedium
                            }
                            ColumnLayout {
                                Layout.fillWidth: true
                                spacing: 0
                                QQC2.Label {
                                    Layout.fillWidth: true
                                    text: appRow.modelData.name
                                    elide: Text.ElideRight
                                }
                                QQC2.Label {
                                    Layout.fillWidth: true
                                    opacity: 0.7
                                    font: Kirigami.Theme.smallFont
                                    elide: Text.ElideRight
                                    text: (appRow.modelData.runtime ? qsTr("Runtime") : qsTr("App")) + " · " + appRow.modelData.branch + " · " + (appRow.modelData.system ? qsTr("System") : qsTr("User"))
                                }
                            }
                            QQC2.Label {
                                text: appRow.modelData.size_text
                                opacity: 0.7
                            }
                        }
                    }
                }
                Flow {
                    Layout.fillWidth: true
                    spacing: Kirigami.Units.smallSpacing
                    QQC2.Button {
                        text: qsTr("Update apps")
                        icon.name: "update-high"
                        visible: page.apps.length > 0
                        enabled: !page.backend.appsBusy
                        onClicked: page.backend.updateApps()
                    }
                    QQC2.Button {
                        text: qsTr("Check for app updates")
                        icon.name: "view-refresh"
                        enabled: !page.backend.appsBusy
                        onClicked: page.backend.checkApps()
                    }
                }
            }
        }
    }
}
