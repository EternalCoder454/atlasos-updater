// The screen-edge glow, as a program of its own. Telamon Updater's tray
// (plain Rust, a few MB, no Qt) starts it when the system is being changed:
// an update, a channel switch or a go back being staged, apps being updated
// or firmware being installed, whether Settings' window is open or not, and
// ends it (SIGTERM) when that is over. It draws the glow around the edges of
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

using namespace std::chrono_literals;

int main(int argc, char *argv[])
{
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
