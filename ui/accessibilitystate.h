// AccessibilityState.active: whether an assistive technology (a screen
// reader) is listening, so a component can leave out text only one would
// read. Qt keeps the answer and says when it changes.
#pragma once

#include <QAccessible>
#include <QObject>
#include <QtQml/qqmlregistration.h>

class AccessibilityState : public QObject, public QAccessible::ActivationObserver
{
    Q_OBJECT
    QML_NAMED_ELEMENT(AccessibilityState)
    QML_SINGLETON

    Q_PROPERTY(bool active READ active NOTIFY activeChanged)

public:
    explicit AccessibilityState(QObject *parent = nullptr);
    ~AccessibilityState() override;

    bool active() const { return QAccessible::isActive(); }
    void accessibilityActiveChanged(bool) override { Q_EMIT activeChanged(); }

Q_SIGNALS:
    void activeChanged();
};
