#include "repaintarea.h"

#include <QQuickWindow>
#include <QSGRectangleNode>
#include <QSGRendererInterface>

#include <cmath>

// QVariant's ==, except that a NaN (a figure the machine doesn't report)
// equals itself, in a list as well: otherwise a row with one would repaint
// on every change of the model.
static bool same(const QVariant &a, const QVariant &b)
{
    if (a.typeId() == QMetaType::Double && b.typeId() == QMetaType::Double) {
        const double x = a.toDouble(), y = b.toDouble();
        return x == y || (std::isnan(x) && std::isnan(y));
    }
    if (a.typeId() == QMetaType::QVariantList && b.typeId() == QMetaType::QVariantList) {
        const QVariantList l = a.toList(), m = b.toList();
        if (l.size() != m.size()) {
            return false;
        }
        for (qsizetype i = 0; i < l.size(); ++i) {
            if (!same(l[i], m[i])) {
                return false;
            }
        }
        return true;
    }
    return a == b;
}

RepaintArea::RepaintArea(QQuickItem *parent)
    : QQuickItem(parent)
{
}

void RepaintArea::setContent(const QVariantList &content)
{
    if (same(QVariant(content), QVariant(m_content))) {
        return;
    }
    m_content = content;
    if (m_software) {
        update();
    }
    Q_EMIT contentChanged();
}

void RepaintArea::itemChange(ItemChange change, const ItemChangeData &value)
{
    if (change == ItemSceneChange) {
        // Only the software renderer repaints by region; the GPU backends
        // draw every frame whole, so there a node would only cost.
        QSGRendererInterface *renderer = value.window ? value.window->rendererInterface() : nullptr;
        m_software = renderer && renderer->graphicsApi() == QSGRendererInterface::Software;
        setFlag(ItemHasContents, m_software);
        if (m_software) {
            update();
        }
    }
    QQuickItem::itemChange(change, value);
}

void RepaintArea::geometryChange(const QRectF &newGeometry, const QRectF &oldGeometry)
{
    QQuickItem::geometryChange(newGeometry, oldGeometry);
    if (m_software && newGeometry.size() != oldGeometry.size()) {
        update();
    }
}

QSGNode *RepaintArea::updatePaintNode(QSGNode *old, UpdatePaintNodeData *)
{
    if (!m_software || width() <= 0 || height() <= 0) {
        delete old;
        return nullptr;
    }
    auto *node = static_cast<QSGRectangleNode *>(old);
    if (!node) {
        node = window()->createRectangleNode();
        node->setColor(Qt::transparent);
    }
    node->setRect(boundingRect());
    // The software renderer repaints a node's whole rectangle when its
    // material is dirty, and everything under it there.
    node->markDirty(QSGNode::DirtyMaterial);
    return node;
}
