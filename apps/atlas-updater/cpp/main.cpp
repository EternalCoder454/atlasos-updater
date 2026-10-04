// Starts Qt, makes the app single-instance and opens the window. The panel
// icon is atlas-updater-tray (--tray hands over to it, for old autostart
// entries); --worker runs an app job for the tray without Qt.
#include "shell.h"

#include <KDBusService>

#include <QApplication>
#include <QIcon>
#include <QQmlEngine>
#include <QQuickStyle>

#include <cerrno>
#include <climits>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <libgen.h>
#include <unistd.h>

// Rust, see src/lib.rs and src/crash.rs.
extern "C" void *atlas_backend_new();
extern "C" void atlas_crash_install();
extern "C" void atlas_crash_fatal(const char *msg);
extern "C" int atlas_worker(const char *job);
extern "C" void atlas_tray_flush();

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

// The tray program next to this one (/usr/bin/atlas-updater-tray).
static int execTray()
{
    char self[PATH_MAX];
    const ssize_t n = readlink("/proc/self/exe", self, sizeof self - 1);
    if (n <= 0) {
        fprintf(stderr, "atlas-updater: cannot find the tray program: %s\n", std::strerror(errno));
        return 1;
    }
    self[n] = '\0';
    char path[PATH_MAX];
    if (snprintf(path, sizeof path, "%s/atlas-updater-tray", dirname(self)) >= int(sizeof path)) {
        return 1;
    }
    char *const args[] = {path, nullptr};
    execv(path, args);
    fprintf(stderr, "atlas-updater: cannot start %s: %s\n", path, std::strerror(errno));
    return 1;
}

// `--page <name>` and `--check`, from the command line or a second launch.
struct Request {
    QString page;
    bool check = false;
};

static Request parseRequest(const QStringList &args)
{
    Request r;
    for (int i = 1; i < args.size(); ++i) {
        if (args[i] == QLatin1String("--check")) {
            r.check = true;
        } else if (args[i] == QLatin1String("--page") && i + 1 < args.size()) {
            const QString page = args[++i];
            static const QStringList pages = {QStringLiteral("updates"), QStringLiteral("settings"), QStringLiteral("reports"), QStringLiteral("sent")};
            if (pages.contains(page)) {
                r.page = page;
            }
        }
    }
    return r;
}

static void handle(Shell &shell, const Request &r)
{
    if (r.check) {
        shell.checkForUpdate();
    } else {
        shell.openWindow(r.page);
    }
}

int main(int argc, char *argv[])
{
    atlas_crash_install(); // Rust panic hook, first thing.

    for (int i = 1; i < argc; ++i) {
        if (std::strcmp(argv[i], "--worker") == 0) {
            // An app job for the tray: no Qt at all.
            return atlas_worker(i + 1 < argc ? argv[i + 1] : "");
        }
        if (std::strcmp(argv[i], "--tray") == 0) {
            return execTray();
        }
    }

    QApplication app(argc, argv);
    s_previousHandler = qInstallMessageHandler(messageHandler);
    // Together these give the single-instance D-Bus name net.eterneon.atlas.updater
    // (the app ID); do not change either without changing that name.
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
        Shell shell(backend);

        QObject::connect(&service, &KDBusService::activateRequested, &shell, [&shell](const QStringList &arguments, const QString &) {
            handle(shell, parseRequest(arguments));
        });

        Request first = parseRequest(QCoreApplication::arguments());
        // Developer option, only with ATLAS_UPDATER_FIXTURES: open on a given page.
        if (first.page.isEmpty() && qEnvironmentVariableIsSet("ATLAS_UPDATER_FIXTURES")) {
            first.page = qEnvironmentVariable("ATLAS_UPDATER_PAGE");
        }
        handle(shell, first);
        // Idle: give up the name first, so a launch from now on starts a new
        // instance instead of asking this one, which is about to go.
        QObject::connect(&shell, &Shell::idle, &app, [&service] {
            service.unregister();
            QCoreApplication::quit();
        });
        do {
            rc = app.exec();
            // A launch that reached us just before the name went opened a
            // window: keep it.
        } while (rc == 0 && shell.hasWindow());
    }
    // A setting change the tray has not heard about yet.
    atlas_tray_flush();
    delete backend;
    return rc;
}
