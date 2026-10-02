// Thin C++ glue: start Qt, hand the Rust backend to QML, load the window.
#include <QApplication>
#include <QQmlApplicationEngine>
#include <QQuickStyle>
#include <QVariant>

// Defined in src/lib.rs.
extern "C" void *atlas_backend_new();

int main(int argc, char *argv[])
{
    QApplication app(argc, argv);
    QApplication::setApplicationName(QStringLiteral("atlas-app-template"));
    QApplication::setDesktopFileName(QStringLiteral("net.eterneon.atlas.apptemplate"));

    if (qEnvironmentVariableIsEmpty("QT_QUICK_CONTROLS_STYLE")) {
        QQuickStyle::setStyle(QStringLiteral("org.kde.desktop"));
    }

    auto *backend = static_cast<QObject *>(atlas_backend_new());

    QQmlApplicationEngine engine;
    engine.setInitialProperties({{QStringLiteral("backend"), QVariant::fromValue(backend)}});
    QObject::connect(&engine, &QQmlApplicationEngine::objectCreationFailed, &app, [] { QCoreApplication::exit(1); }, Qt::QueuedConnection);
    engine.loadFromModule(QStringLiteral("net.eterneon.atlas.apptemplate"), QStringLiteral("Main"));

    const int rc = app.exec();
    delete backend;
    return rc;
}
