#include "accessibilitystate.h"

AccessibilityState::AccessibilityState(QObject *parent)
    : QObject(parent)
{
    QAccessible::installActivationObserver(this);
}

AccessibilityState::~AccessibilityState()
{
    QAccessible::removeActivationObserver(this);
}
