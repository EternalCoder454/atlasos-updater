#include "shell.h"

#include <KNotification>
#include <KStatusNotifierItem>

#include <QAction>
#include <QCoreApplication>
#include <QDateTime>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QMenu>
#include <QQmlApplicationEngine>
#include <QQuickWindow>

#include <malloc.h>

namespace
{
const QString kAppIcon = QStringLiteral("net.eterneon.atlas.updater");
const QString kComponent = QStringLiteral("atlas-updater");
const QString kOstreeRunDir = QStringLiteral("/run/ostree");
const QString kCoredumpDir = QStringLiteral("/var/lib/systemd/coredump");
const QString kEventsFile = QStringLiteral("/var/lib/atlas-core/events.jsonl");
// Watched in place of a source that does not exist yet.
const QStringList kCoredumpParents = {QStringLiteral("/var/lib/systemd"), QStringLiteral("/var/lib")};
const QStringList kEventsParents = {QStringLiteral("/var/lib/atlas-core"), QStringLiteral("/var/lib")};
}

Shell::Shell(QObject *backend, bool trayMode, QObject *parent)
    : QObject(parent)
    , m_backend(backend)
    , m_trayMode(trayMode)
{
    // Tray icon: Passive normally, NeedsAttention while an update is staged.
    m_tray = new KStatusNotifierItem(QStringLiteral("net.eterneon.atlas.updater"), this);
    m_tray->setCategory(KStatusNotifierItem::SystemServices);
    m_tray->setTitle(tr("Atlas Updater"));
    // Monochrome like the tray's other icons, so they follow light and dark
    m_tray->setIconByName(QStringLiteral("net.eterneon.atlas.updater-symbolic"));
    m_tray->setAttentionIconByName(QStringLiteral("net.eterneon.atlas.updater-ready-symbolic"));
    m_tray->setStatus(KStatusNotifierItem::Passive);

    auto *menu = m_tray->contextMenu();
    menu->addAction(QIcon::fromTheme(kAppIcon), tr("Open Atlas Updater"), this, [this] { openWindow(); });
    menu->addAction(QIcon::fromTheme(QStringLiteral("view-refresh")), tr("Check for Updates"), this, [this] {
        QMetaObject::invokeMethod(m_backend, "checkForUpdate");
        openWindow();
    });
    m_restartAction = menu->addAction(QIcon::fromTheme(QStringLiteral("system-reboot")), tr("Restart to Update"), this, [this] { restartNow(); });
    m_cancelAction = menu->addAction(QIcon::fromTheme(QStringLiteral("dialog-cancel")), tr("Cancel Scheduled Restart"), this, [this] { cancelRestart(); });
    connect(m_tray, &KStatusNotifierItem::activateRequested, this, [this] { openWindow(); });

    connect(m_backend, SIGNAL(hasStagedChanged()), this, SLOT(updateTray()));
    connect(m_backend, SIGNAL(stagedVersionChanged()), this, SLOT(updateTray()));
    connect(m_backend, SIGNAL(scheduledAtChanged()), this, SLOT(updateTray()));
    connect(m_backend, SIGNAL(updateStaged(QString)), this, SLOT(onUpdateStaged(QString)));
    connect(m_backend, SIGNAL(restartSoon()), this, SLOT(onRestartSoon()));
    connect(m_backend, SIGNAL(restartProblem(QString)), this, SLOT(onRestartProblem(QString)));
    connect(m_backend, SIGNAL(scheduledAtChanged()), this, SLOT(onScheduleChanged()));
    connect(m_backend, SIGNAL(reportFound(QString,QString)), this, SLOT(onReportFound(QString,QString)));
    connect(m_backend, SIGNAL(crashEnabledChanged()), this, SLOT(updateCollectors()));
    updateTray();

    // Staged-update detection: ostree creates /run/ostree/staged-deployment.
    m_debounce.setSingleShot(true);
    m_debounce.setInterval(1500);
    connect(&m_debounce, &QTimer::timeout, this, [this] { QMetaObject::invokeMethod(m_backend, "refreshStatus"); });
    connect(&m_watcher, &QFileSystemWatcher::directoryChanged, this, &Shell::onWatchedChanged);
    watchOstree();

    m_crashDebounce.setSingleShot(true);
    m_crashDebounce.setInterval(3000);
    connect(&m_crashDebounce, &QTimer::timeout, this, [this] { QMetaObject::invokeMethod(m_backend, "collectReports"); });

    QMetaObject::invokeMethod(m_backend, "start");
}

Shell::~Shell()
{
    delete m_crashWatcher;
    QMetaObject::invokeMethod(m_backend, "shutdown");
    delete m_engine;
}

void Shell::watchOstree()
{
    if (QDir(kOstreeRunDir).exists()) {
        m_watcher.addPath(kOstreeRunDir);
    } else {
        // Not there yet: wait for it to appear (only in unusual setups).
        m_watcher.addPath(QStringLiteral("/run"));
    }
}

void Shell::onWatchedChanged(const QString &path)
{
    if (path == QLatin1String("/run")) {
        if (QDir(kOstreeRunDir).exists()) {
            m_watcher.removePath(path);
            m_watcher.addPath(kOstreeRunDir);
        } else {
            return;
        }
    }
    m_debounce.start();
}

void Shell::updateCollectors()
{
    watchCollectors(m_backend->property("crashEnabled").toBool());
}

// Crash reports are opt-in. Switched off: no watcher exists, nothing is read.
void Shell::watchCollectors(bool on)
{
    if (!on) {
        delete m_crashWatcher;
        m_crashWatcher = nullptr;
        return;
    }
    if (m_crashWatcher) {
        return;
    }
    m_crashWatcher = new QFileSystemWatcher(this);
    connect(m_crashWatcher, &QFileSystemWatcher::directoryChanged, this, [this](const QString &path) {
        // A parent only matters for noticing that a source appeared.
        const bool newSource = syncCollectorPaths();
        if (path == kCoredumpDir || newSource) {
            m_crashDebounce.start();
        }
    });
    connect(m_crashWatcher, &QFileSystemWatcher::fileChanged, this, [this](const QString &) {
        // The events file may be replaced (rotation): keep watching the new one.
        syncCollectorPaths();
        m_crashDebounce.start();
    });
    syncCollectorPaths();
}

bool Shell::syncCollectorPaths()
{
    if (!m_crashWatcher) {
        return false;
    }
    bool added = false;
    QStringList neededParents;
    const auto watch = [&](const QString &source, const QStringList &parents) {
        const QStringList watched = m_crashWatcher->files() + m_crashWatcher->directories();
        if (QFileInfo::exists(source)) {
            if (!watched.contains(source)) {
                m_crashWatcher->addPath(source);
                added = true;
            }
            return;
        }
        for (const QString &p : parents) {
            if (QDir(p).exists()) {
                neededParents << p;
                if (!watched.contains(p)) {
                    m_crashWatcher->addPath(p);
                }
                return;
            }
        }
    };
    watch(kCoredumpDir, kCoredumpParents);
    watch(kEventsFile, kEventsParents);
    // Parents only stand in for a missing source: drop them once not needed.
    for (const QString &p : kCoredumpParents + kEventsParents) {
        if (!neededParents.contains(p) && p != kCoredumpDir && m_crashWatcher->directories().contains(p)) {
            m_crashWatcher->removePath(p);
        }
    }
    return added;
}

void Shell::onReportFound(const QString &appName, const QString &reportType)
{
    if (windowIsActive()) {
        return;
    }
    auto *n = new KNotification(QStringLiteral("crashReport"));
    n->setComponentName(kComponent);
    n->setTitle(tr("Crash report ready"));
    const bool crash = reportType == QLatin1String("panic") || reportType == QLatin1String("fatal") || reportType == QLatin1String("coredump");
    n->setText(crash ? tr("%1 closed unexpectedly. Review the crash report?").arg(appName.toHtmlEscaped()) : tr("Something went wrong with a system update. Review the report?"));
    n->setIconName(QStringLiteral("tools-report-bug"));
    auto *review = n->addAction(tr("Review"));
    connect(review, &KNotificationAction::activated, this, [this] { openWindow(QStringLiteral("reports")); });
    n->sendEvent();
}

bool Shell::windowIsActive() const
{
    return m_window && m_window->isVisible() && m_window->isActive();
}

void Shell::updateTray()
{
    const bool staged = m_backend->property("hasStaged").toBool();
    const qint64 at = m_backend->property("scheduledAt").toLongLong();
    const QString version = m_backend->property("stagedVersion").toString();
    m_tray->setStatus(staged ? KStatusNotifierItem::NeedsAttention : KStatusNotifierItem::Passive);
    QString sub = staged ? tr("Update %1 is ready. Restart to install it.").arg(version) : tr("Your system is up to date.");
    if (at > 0) {
        sub += QLatin1Char('\n') + tr("Restart scheduled for %1").arg(QLocale().toString(QDateTime::fromSecsSinceEpoch(at), QLocale::ShortFormat));
    }
    m_tray->setToolTip(kAppIcon, tr("Atlas Updater"), sub);
    m_restartAction->setVisible(staged);
    m_cancelAction->setVisible(at > 0);
}

void Shell::onUpdateStaged(const QString &version)
{
    if (windowIsActive()) {
        return; // The user is looking at it.
    }
    auto *n = new KNotification(QStringLiteral("updateStaged"));
    n->setComponentName(kComponent);
    n->setTitle(tr("Update ready"));
    n->setText(tr("AtlasOS %1 is downloaded. Restart to finish installing it.").arg(version.toHtmlEscaped()));
    n->setIconName(kAppIcon);
    auto *restart = n->addAction(tr("Restart to Update"));
    connect(restart, &KNotificationAction::activated, this, [this] { restartNow(); });
    auto *open = n->addDefaultAction(tr("Open Atlas Updater"));
    connect(open, &KNotificationAction::activated, this, [this] { openWindow(); });
    n->sendEvent();
}

void Shell::onRestartSoon()
{
    auto *n = new KNotification(QStringLiteral("restartSoon"));
    n->setComponentName(kComponent);
    // The real time left: a saved time found at login can be much closer.
    const qint64 left = m_backend->property("scheduledAt").toLongLong() - QDateTime::currentSecsSinceEpoch();
    const int minutes = qMax<qint64>(1, (left + 59) / 60);
    n->setTitle(minutes == 1 ? tr("Restarting in 1 minute") : tr("Restarting in %n minutes", "", minutes));
    n->setText(tr("Your computer will restart soon to finish updating. Save your work."));
    n->setIconName(kAppIcon);
    // The only warning before an automatic restart: it stays until the user
    // acts, and closes itself when the restart is cancelled.
    n->setFlags(KNotification::Persistent);
    n->setUrgency(KNotification::CriticalUrgency);
    m_restartSoon = n;
    auto *now = n->addAction(tr("Restart Now"));
    connect(now, &KNotificationAction::activated, this, [this] { restartNow(); });
    auto *cancel = n->addAction(tr("Cancel Restart"));
    connect(cancel, &KNotificationAction::activated, this, [this] { cancelRestart(); });
    n->sendEvent();
}

void Shell::onRestartProblem(const QString &text)
{
    if (windowIsActive()) {
        return; // The window shows it on the Updates page.
    }
    auto *n = new KNotification(QStringLiteral("restartFailed"));
    n->setComponentName(kComponent);
    n->setTitle(tr("Restart did not happen"));
    n->setText(text.toHtmlEscaped());
    n->setIconName(kAppIcon);
    n->setUrgency(KNotification::HighUrgency);
    auto *open = n->addDefaultAction(tr("Open Atlas Updater"));
    connect(open, &KNotificationAction::activated, this, [this] { openWindow(QStringLiteral("updates")); });
    n->sendEvent();
}

void Shell::onScheduleChanged()
{
    const bool scheduled = m_backend->property("scheduledAt").toLongLong() > 0;
    if (!scheduled && m_restartSoon) {
        m_restartSoon->close();
    }
    // Non-tray mode stays alive for a scheduled restart only. A restart that is
    // running keeps scheduledAt set until it fails, so this is the end of a
    // cancel, a missed time or a failure: leave a moment for its notification
    // to be sent, then quit if nothing has been scheduled or opened since.
    if (!scheduled && !m_trayMode && !m_engine) {
        QTimer::singleShot(10000, this, [this] {
            if (m_backend->property("scheduledAt").toLongLong() <= 0 && !m_trayMode && !m_engine) {
                QCoreApplication::quit();
            }
        });
    }
}

void Shell::enableTrayMode()
{
    m_trayMode = true;
}

void Shell::restartNow()
{
    QMetaObject::invokeMethod(m_backend, "restartNow");
}

void Shell::cancelRestart()
{
    QMetaObject::invokeMethod(m_backend, "cancelRestart");
}

void Shell::openWindow(const QString &page)
{
    if (m_engine && m_window) {
        // A close may have queued the engine's destruction: cancel it.
        m_closing = false;
        m_window->show();
        m_window->raise();
        m_window->requestActivate();
        if (!page.isEmpty()) {
            QMetaObject::invokeMethod(m_window, "showPage", Q_ARG(QVariant, page));
        }
        return;
    }
    // The QML engine only exists while the window does.
    auto *engine = new QQmlApplicationEngine;
    m_engine = engine;
    m_closing = false;
    engine->setInitialProperties({
        {QStringLiteral("backend"), QVariant::fromValue(m_backend)},
        {QStringLiteral("startPage"), page.isEmpty() ? QStringLiteral("updates") : page},
    });
    connect(engine, &QQmlApplicationEngine::objectCreationFailed, this, [this] {
        // A broken UI must not take the tray (and a scheduled restart) down.
        if (m_trayMode) {
            m_closing = true;
            QTimer::singleShot(0, this, &Shell::destroyEngine);
        } else {
            QCoreApplication::exit(1);
        }
    }, Qt::QueuedConnection);
    engine->loadFromModule(QStringLiteral("net.eterneon.atlas.updater"), QStringLiteral("Main"));
    m_window = qobject_cast<QQuickWindow *>(engine->rootObjects().value(0));
    if (!m_window) {
        // Nothing to show: do not keep a half-built engine around.
        m_engine = nullptr;
        delete engine;
        if (m_trayMode) {
            return;
        }
        QCoreApplication::exit(1);
        return;
    }
    connect(m_window, &QQuickWindow::closing, this, [this] {
        m_closing = true;
        QTimer::singleShot(0, this, &Shell::destroyEngine);
    });
    QMetaObject::invokeMethod(m_backend, "windowOpened");
    QMetaObject::invokeMethod(m_backend, "refreshStatus");
    QMetaObject::invokeMethod(m_backend, "loadReports");
}

void Shell::destroyEngine()
{
    // A second launch may have reopened the window since the close.
    if (!m_engine || !m_closing) {
        return;
    }
    m_closing = false;
    QMetaObject::invokeMethod(m_backend, "windowClosed");
    QQmlApplicationEngine *engine = m_engine;
    m_engine = nullptr;
    m_window = nullptr;
    // Free everything QML held, then hand the heap back to the OS.
    connect(engine, &QObject::destroyed, qApp, [] { malloc_trim(0); });
    delete engine;
    malloc_trim(0);
    const bool restartScheduled = m_backend->property("scheduledAt").toLongLong() > 0;
    if (!m_trayMode && !restartScheduled) {
        QCoreApplication::quit();
    }
}
