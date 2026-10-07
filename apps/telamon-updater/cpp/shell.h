// Thin Qt glue: the window's lifetime and the watchers that keep it current.
// All app logic is in the Rust `Backend` QObject, which this class talks to
// through the Qt meta-object system (signals, slots, properties). The panel
// icon, the schedule and the notifications are telamon-updater-tray's: this
// process runs only while the window is open.
#pragma once

#include <QByteArray>
#include <QDateTime>
#include <QFileSystemWatcher>
#include <QObject>
#include <QPointer>
#include <QString>
#include <QTimer>

class QQmlApplicationEngine;
class QQuickWindow;

// Developer variables: TELAMON_UPDATER_<name>, and when that is not set the
// name they had until 0.3.0, ATLAS_UPDATER_<name>. The new name wins.
inline bool updaterEnvIsSet(const char *name)
{
    return qEnvironmentVariableIsSet((QByteArray("TELAMON_UPDATER_") + name).constData())
        || qEnvironmentVariableIsSet((QByteArray("ATLAS_UPDATER_") + name).constData());
}

inline QString updaterEnv(const char *name)
{
    const QByteArray current = QByteArray("TELAMON_UPDATER_") + name;
    return qEnvironmentVariableIsSet(current.constData())
        ? qEnvironmentVariable(current.constData())
        : qEnvironmentVariable((QByteArray("ATLAS_UPDATER_") + name).constData());
}

class Shell : public QObject
{
    Q_OBJECT
public:
    explicit Shell(QObject *backend, QObject *parent = nullptr);
    ~Shell() override;

    /// Opens the window (creating the QML engine on first use) or raises it.
    /// `page` is "updates" (default), "settings", "reports" or "sent".
    void openWindow(const QString &page = QString());
    /// The tray's "Check for Updates".
    void checkForUpdate();
    /// The window is open (or reopened after `idle`).
    bool hasWindow() const
    {
        return m_engine != nullptr;
    }

Q_SIGNALS:
    /// The window is closed and nothing runs: time to quit.
    void idle();

private Q_SLOTS:
    void onWatchedChanged(const QString &path);
    void destroyEngine();
    void quitWhenIdle();

private:
    void watchOstree();
    void watchRenderer(QQuickWindow *window);

    QObject *m_backend;
    QQmlApplicationEngine *m_engine = nullptr;
    bool m_closing = false; // window closed, engine destruction queued
    QPointer<QQuickWindow> m_window;
    QFileSystemWatcher m_watcher;
    QTimer m_debounce;
    // The settings file: the tray clears a restart, a round runs.
    QFileSystemWatcher m_configWatcher;
    QTimer m_configDebounce;
    QString m_rcPath;
    QDateTime m_rcModified; // other files in the folder change too
};
