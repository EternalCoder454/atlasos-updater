// Starts Qt, makes the app single-instance, and either opens the window or
// (--tray) only sets up the tray icon. QML is loaded only when a window opens.
#include "shell.h"

#include <KDBusService>

#include <QApplication>
#include <QIcon>
#include <QQmlEngine>
#include <QQuickStyle>

#include <cstdio>
#include <cstring>

// Rust, see src/lib.rs and src/crash.rs.
extern "C" void *atlas_backend_new();
extern "C" void atlas_crash_install();
extern "C" void atlas_crash_fatal(const char *msg);

static QtMessageHandler s_previousHandler = nullptr;

static void messageHandler(QtMsgType type, const QMessageLogContext &context, const QString &msg)
{
    if (type == QtFatalMsg) {
        // Let the Rust side save a crash report (if the user enabled them).
        atlas_crash_fatal(msg.toUtf8().constData());
    }
    if (s_previousHandler) {
        s_previousHandler(type, context, msg);
    } else {
        // Qt's built-in handler is not returned by qInstallMessageHandler:
        // print the message ourselves, so warnings and fatal errors are not lost.
        fprintf(stderr, "%s\n", qPrintable(qFormatLogMessage(type, context, msg)));
        fflush(stderr);
    }
}

int main(int argc, char *argv[])
{
    atlas_crash_install(); // Rust panic hook, first thing.

    bool trayMode = false;
    for (int i = 1; i < argc; ++i) {
        if (std::strcmp(argv[i], "--tray") == 0) {
            trayMode = true;
        }
    }

    QApplication app(argc, argv);
    s_previousHandler = qInstallMessageHandler(messageHandler);
    // Together these give the D-Bus name net.eterneon.atlas.updater.
    QApplication::setOrganizationDomain(QStringLiteral("atlas.eterneon.net"));
    QApplication::setApplicationName(QStringLiteral("updater"));
    QApplication::setApplicationDisplayName(QStringLiteral("Atlas Updater"));
    QApplication::setApplicationVersion(QStringLiteral(ATLAS_UPDATER_VERSION));
    QApplication::setDesktopFileName(QStringLiteral("net.eterneon.atlas.updater"));
    QApplication::setWindowIcon(QIcon::fromTheme(QStringLiteral("net.eterneon.atlas.updater")));
    QApplication::setQuitOnLastWindowClosed(false); // Shell decides when to quit.

    if (qEnvironmentVariableIsEmpty("QT_QUICK_CONTROLS_STYLE")) {
        QQuickStyle::setStyle(QStringLiteral("org.kde.desktop"));
    }

    // One instance per session; a second launch raises the first one.
    KDBusService service(KDBusService::Unique);

    auto *backend = static_cast<QObject *>(atlas_backend_new());
    // main() owns it: a destroyed QML engine must never delete it.
    QQmlEngine::setObjectOwnership(backend, QQmlEngine::CppOwnership);
    int rc = 0;
    {
        Shell shell(backend, trayMode);

        QObject::connect(&service, &KDBusService::activateRequested, &shell, [&shell](const QStringList &arguments, const QString &) {
            // The autostart entry (--tray) must not pop a window up, but it
            // does mean this instance is the session's tray: stay alive.
            if (arguments.contains(QStringLiteral("--tray"))) {
                shell.enableTrayMode();
            } else {
                shell.openWindow();
            }
        });

        if (!trayMode) {
            // Developer option, only with ATLAS_UPDATER_FIXTURES: open on a given page.
            shell.openWindow(qEnvironmentVariableIsSet("ATLAS_UPDATER_FIXTURES") ? qEnvironmentVariable("ATLAS_UPDATER_PAGE") : QString());
        }
        rc = app.exec();
    }
    delete backend;
    return rc;
}
