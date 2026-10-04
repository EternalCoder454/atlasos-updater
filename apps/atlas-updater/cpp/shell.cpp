#include "shell.h"

#include <QCoreApplication>
#include <QDir>
#include <QFileInfo>
#include <QQmlApplicationEngine>
#include <QQuickWindow>
#include <QStandardPaths>

#include <malloc.h>

namespace
{
const QString kOstreeRunDir = QStringLiteral("/run/ostree");
}

Shell::Shell(QObject *backend, QObject *parent)
    : QObject(parent)
    , m_backend(backend)
{
    // Staged-update detection: ostree creates /run/ostree/staged-deployment.
    m_debounce.setSingleShot(true);
    m_debounce.setInterval(1500);
    connect(&m_debounce, &QTimer::timeout, this, [this] { QMetaObject::invokeMethod(m_backend, "refreshStatus"); });
    connect(&m_watcher, &QFileSystemWatcher::directoryChanged, this, &Shell::onWatchedChanged);
    watchOstree();

    // The settings file is replaced on every write (temp file, then rename):
    // watch its directory.
    m_configDebounce.setSingleShot(true);
    m_configDebounce.setInterval(500);
    const QString configDir = QStandardPaths::writableLocation(QStandardPaths::GenericConfigLocation);
    m_rcPath = configDir + QStringLiteral("/atlas-updaterrc");
    m_rcModified = QFileInfo(m_rcPath).lastModified();
    connect(&m_configDebounce, &QTimer::timeout, this, [this] {
        const QDateTime modified = QFileInfo(m_rcPath).lastModified();
        if (modified == m_rcModified) {
            return;
        }
        m_rcModified = modified;
        QMetaObject::invokeMethod(m_backend, "reloadSettings");
    });
    if (!configDir.isEmpty() && QDir(configDir).exists()) {
        m_configWatcher.addPath(configDir);
    }
    connect(&m_configWatcher, &QFileSystemWatcher::directoryChanged, this, [this] { m_configDebounce.start(); });

    // Quit once the window is gone and nothing runs (see quitWhenIdle).
    connect(m_backend, SIGNAL(busyChanged()), this, SLOT(quitWhenIdle()));
    connect(m_backend, SIGNAL(appsBusyChanged()), this, SLOT(quitWhenIdle()));
    connect(m_backend, SIGNAL(restartingChanged()), this, SLOT(quitWhenIdle()));

    QMetaObject::invokeMethod(m_backend, "start");
}

Shell::~Shell()
{
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

void Shell::checkForUpdate()
{
    openWindow(QStringLiteral("updates"));
    QMetaObject::invokeMethod(m_backend, "checkForUpdate");
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
    auto *engine = new QQmlApplicationEngine;
    m_engine = engine;
    m_closing = false;
    engine->setInitialProperties({
        {QStringLiteral("backend"), QVariant::fromValue(m_backend)},
        {QStringLiteral("startPage"), page.isEmpty() ? QStringLiteral("updates") : page},
    });
    connect(engine, &QQmlApplicationEngine::objectCreationFailed, this, [] { QCoreApplication::exit(1); }, Qt::QueuedConnection);
    engine->loadFromModule(QStringLiteral("net.eterneon.atlas.updater"), QStringLiteral("Main"));
    m_window = qobject_cast<QQuickWindow *>(engine->rootObjects().value(0));
    if (!m_window) {
        // Nothing to show: do not keep a half-built engine around.
        m_engine = nullptr;
        delete engine;
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
    delete engine;
    malloc_trim(0);
    quitWhenIdle();
}

// The window is closed: quit, but not in the middle of an operation (a
// download, an app update or a restart request finishes first).
void Shell::quitWhenIdle()
{
    if (m_engine) {
        return;
    }
    const bool running = m_backend->property("busy").toBool() || m_backend->property("appsBusy").toBool() || m_backend->property("restarting").toBool();
    if (!running) {
        Q_EMIT idle();
    }
}
