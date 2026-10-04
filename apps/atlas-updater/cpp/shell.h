// Thin Qt glue: tray icon, notifications, inotify watcher and the QML engine's
// lifetime. All app logic is in the Rust `Backend` QObject, which this class
// talks to through the Qt meta-object system (signals, slots, properties).
#pragma once

#include <QFileSystemWatcher>
#include <QObject>
#include <QPointer>
#include <QTimer>

class KNotification;
class KStatusNotifierItem;
class QAction;
class QQmlApplicationEngine;
class QQuickWindow;

class Shell : public QObject
{
    Q_OBJECT
public:
    Shell(QObject *backend, bool trayMode, QObject *parent = nullptr);
    ~Shell() override;

    /// Opens the window (creating the QML engine on first use) or raises it.
    /// `page` is "updates" (default), "settings", "reports" or "sent".
    void openWindow(const QString &page = QString());

    /// A tray launch arrived after the window: keep running when it closes.
    void enableTrayMode();

private Q_SLOTS:
    void updateTray();
    void onUpdateStaged(const QString &version);
    void onAppUpdatesReady(const QString &text, bool canUpdate);
    void onRestartSoon();
    void onRestartProblem(const QString &text);
    void onScheduleChanged();
    void onWatchedChanged(const QString &path);
    void updateCollectors();
    void onReportFound(const QString &appName, const QString &reportType);
    void destroyEngine();

private:
    bool windowIsActive() const;
    void watchOstree();
    void watchCollectors(bool on);
    /// Watch each collector source, or the nearest existing parent until it
    /// appears. Returns true if a source was newly watched.
    bool syncCollectorPaths();
    void restartNow();
    void cancelRestart();

    QObject *m_backend;
    bool m_trayMode;
    KStatusNotifierItem *m_tray = nullptr;
    QAction *m_restartAction = nullptr;
    QAction *m_cancelAction = nullptr;
    QQmlApplicationEngine *m_engine = nullptr;
    bool m_closing = false; // window closed, engine destruction queued
    QPointer<KNotification> m_restartSoon;
    // The last restart failed: the tray shows the urgent icon until the next try.
    bool m_restartFailed = false;
    // Tray attention icons: an update is ready, and urgent.
    QString m_readyIcon;
    QString m_urgentIcon;
    QPointer<QQuickWindow> m_window;
    QFileSystemWatcher m_watcher;
    QTimer m_debounce;
    // Crash-report collectors: only exist while the user has reports switched on.
    QFileSystemWatcher *m_crashWatcher = nullptr;
    QTimer m_crashDebounce;
};
