#include "shell.h"

#include <KNotification>
#include <KStatusNotifierItem>

#include <QAction>
#include <QCoreApplication>
#include <QDateTime>
#include <QDir>
#include <QFile>
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
const QString kAtlasStateDir = QStringLiteral("/var/lib/atlas-core");
const QString kEventsFile = QStringLiteral("/var/lib/atlas-core/events.jsonl");
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
    m_tray->setIconByName(kAppIcon);
    m_tray->setAttentionIconByName(QStringLiteral("software-update-available"));
    m_tray->setStatus(KStatusNotifierItem::Passive);

    auto *menu = m_tray->contextMenu();
    menu->addAction(QIcon::fromTheme(kAppIcon), tr("Open Atlas Updater"), this, [this] { openWindow(); });
    menu->addAction(QIcon::fromTheme(QStringLiteral("view-refresh")), tr("Check for updates"), this, [this] {
        QMetaObject::invokeMethod(m_backend, "checkForUpdate");
        openWindow();
    });
    m_restartAction = menu->addAction(QIcon::fromTheme(QStringLiteral("system-reboot")), tr("Restart to update"), this, [this] { restartNow(); });
    m_cancelAction = menu->addAction(QIcon::fromTheme(QStringLiteral("dialog-cancel")), tr("Cancel scheduled restart"), this, [this] { cancelRestart(); });
    connect(m_tray, &KStatusNotifierItem::activateRequested, this, [this] { openWindow(); });

    connect(m_backend, SIGNAL(hasStagedChanged()), this, SLOT(updateTray()));
    connect(m_backend, SIGNAL(stagedVersionChanged()), this, SLOT(updateTray()));
    connect(m_backend, SIGNAL(scheduledAtChanged()), this, SLOT(updateTray()));
    connect(m_backend, SIGNAL(updateStaged(QString)), this, SLOT(onUpdateStaged(QString)));
    connect(m_backend, SIGNAL(restartSoon()), this, SLOT(onRestartSoon()));
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
    const auto changed = [this] { m_crashDebounce.start(); };
    connect(m_crashWatcher, &QFileSystemWatcher::directoryChanged, this, changed);
    connect(m_crashWatcher, &QFileSystemWatcher::fileChanged, this, [this](const QString &path) {
        // The events file may be replaced; keep watching the new one.
        if (!m_crashWatcher->files().contains(path) && QFile::exists(path)) {
            m_crashWatcher->addPath(path);
        }
        m_crashDebounce.start();
    });
    for (const QString &dir : {kCoredumpDir, kAtlasStateDir}) {
        if (QDir(dir).exists()) {
            m_crashWatcher->addPath(dir);
        }
    }
    if (QFile::exists(kEventsFile)) {
        m_crashWatcher->addPath(kEventsFile);
    }
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
    n->setText(crash ? tr("%1 closed unexpectedly. Review the crash report?").arg(appName) : tr("Something went wrong with a system update. Review the report?"));
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
    n->setText(tr("AtlasOS %1 is downloaded. Restart to finish installing it.").arg(version));
    n->setIconName(kAppIcon);
    auto *restart = n->addAction(tr("Restart to update"));
    connect(restart, &KNotificationAction::activated, this, [this] { restartNow(); });
    auto *open = n->addDefaultAction(tr("Open Atlas Updater"));
    connect(open, &KNotificationAction::activated, this, [this] { openWindow(); });
    n->sendEvent();
}

void Shell::onRestartSoon()
{
    auto *n = new KNotification(QStringLiteral("restartSoon"));
    n->setComponentName(kComponent);
    n->setTitle(tr("Restarting in 5 minutes"));
    n->setText(tr("Your computer will restart soon to finish updating. Save your work."));
    n->setIconName(kAppIcon);
    auto *now = n->addAction(tr("Restart now"));
    connect(now, &KNotificationAction::activated, this, [this] { restartNow(); });
    auto *cancel = n->addAction(tr("Cancel restart"));
    connect(cancel, &KNotificationAction::activated, this, [this] { cancelRestart(); });
    n->sendEvent();
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
        m_window->show();
        m_window->raise();
        m_window->requestActivate();
        if (!page.isEmpty()) {
            QMetaObject::invokeMethod(m_window, "showPage", Q_ARG(QVariant, page));
        }
        return;
    }
    // The QML engine only exists while the window does.
    m_engine = new QQmlApplicationEngine;
    m_engine->setInitialProperties({
        {QStringLiteral("backend"), QVariant::fromValue(m_backend)},
        {QStringLiteral("startPage"), page.isEmpty() ? QStringLiteral("updates") : page},
    });
    connect(m_engine, &QQmlApplicationEngine::objectCreationFailed, this, [] { QCoreApplication::exit(1); }, Qt::QueuedConnection);
    m_engine->loadFromModule(QStringLiteral("net.eterneon.atlas.updater"), QStringLiteral("Main"));
    m_window = qobject_cast<QQuickWindow *>(m_engine->rootObjects().value(0));
    if (!m_window) {
        return;
    }
    connect(m_window, &QQuickWindow::closing, this, [this] { QTimer::singleShot(0, this, &Shell::destroyEngine); });
    QMetaObject::invokeMethod(m_backend, "refreshStatus");
    QMetaObject::invokeMethod(m_backend, "loadReports");
}

void Shell::destroyEngine()
{
    if (!m_engine) {
        return;
    }
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
