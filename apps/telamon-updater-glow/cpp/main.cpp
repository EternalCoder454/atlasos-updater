// The screen-edge glow, as a program of its own. Telamon Updater's tray
// (plain Rust, a few MB, no Qt) starts it when the OS image is being changed:
// an update being staged, a channel switch or a go back (not app updates,
// firmware or checks), whether Settings' window is open or not, and ends it
// (SIGTERM) when that is over. It draws the glow around the edges of
// every screen (qml/ScreenGlow.qml) and does nothing else: no window of its
// own, no D-Bus name, no input.
//
//   telamon-updater-glow [--seconds <n>]
//
// `--seconds` ends it after n seconds: for looking at the glow by hand and
// for tests. Without it, it still ends itself after 6 hours, the longest the
// helper lets an operation run (60 minutes) with room to spare.
#include <QCommandLineParser>
#include <QGuiApplication>
#include <QQmlApplicationEngine>
#include <QTimer>

#include <chrono>
#include <cstdio>
#include <cstring>

using namespace std::chrono_literals;

int main(int argc, char *argv[])
{
    // --version and --help need no screen: answered before Qt looks for one
    // (the package's build checks run without a display).
    for (int i = 1; i < argc; ++i) {
        if (!std::strcmp(argv[i], "--version") || !std::strcmp(argv[i], "-v")) {
            std::printf("telamon-updater-glow %s\n", TELAMON_UPDATER_GLOW_VERSION);
            return 0;
        }
        if (!std::strcmp(argv[i], "--help") || !std::strcmp(argv[i], "-h")) {
            std::printf("Usage: telamon-updater-glow [--seconds <n>]\n\n"
                        "Draws the glow around the screens' edges while Telamon OS is being changed.\n"
                        "Started and ended by telamon-updater-tray.\n\n"
                        "Options:\n"
                        "  --seconds <n>  End after <n> seconds.\n"
                        "  -v, --version  Print the version.\n"
                        "  -h, --help     Print this help.\n");
            return 0;
        }
    }
    QGuiApplication app(argc, argv);
    // The layer-shell scope of the surfaces is "<name>-glow".
    QGuiApplication::setApplicationName(QStringLiteral("telamon-updater"));
    QGuiApplication::setApplicationDisplayName(QStringLiteral("Telamon Updater"));
    QGuiApplication::setApplicationVersion(QStringLiteral(TELAMON_UPDATER_GLOW_VERSION));

    QCommandLineParser parser;
    parser.setApplicationDescription(QStringLiteral("Draws the glow around the screens' edges while Telamon OS is being changed."));
    parser.addHelpOption();
    parser.addVersionOption();
    const QCommandLineOption seconds(QStringLiteral("seconds"), QStringLiteral("End after <n> seconds."), QStringLiteral("n"));
    parser.addOption(seconds);
    parser.process(app);

    QQmlApplicationEngine engine;
    QObject::connect(&engine, &QQmlApplicationEngine::objectCreationFailed, &app, [] { QCoreApplication::exit(1); }, Qt::QueuedConnection);
    engine.loadFromModule("net.eterneon.telamon.updater.glow", "Main");
    if (engine.rootObjects().isEmpty()) {
        return 1;
    }

    std::chrono::milliseconds limit = 6h;
    if (parser.isSet(seconds)) {
        bool ok = false;
        const int n = parser.value(seconds).toInt(&ok);
        if (ok && n > 0) {
            limit = std::chrono::seconds(n);
        }
    }
    QTimer::singleShot(limit, &app, &QCoreApplication::quit);
    return app.exec();
}
