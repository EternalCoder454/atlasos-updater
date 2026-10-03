// RepaintArea: an invisible item that has its whole area repainted, as one
// rectangle, whenever `content` changes. For Qt Quick's software backend,
// which repaints only what changed: a table row whose figures change dirties
// one rectangle per label, each as tall as its glyphs, and the renderer then
// carries a region of hundreds of slivers through every node of the window,
// which cost a busy table more than drawing the rows. Laid over a row and
// given the row's values, it turns that into one rectangle per row, and rows
// next to each other into one. It draws nothing (a transparent fill, which
// the raster engine skips) and on the GPU backends it has no node at all.
// Measured in atlasos-monitor's bench/pages.
#pragma once

#include <QQuickItem>
#include <QVariant>
#include <QtQml/qqmlregistration.h>

class RepaintArea : public QQuickItem
{
    Q_OBJECT
    QML_NAMED_ELEMENT(RepaintArea)

    // What the area shows: the row's values, say.
    Q_PROPERTY(QVariantList content READ content WRITE setContent NOTIFY contentChanged)

public:
    explicit RepaintArea(QQuickItem *parent = nullptr);

    QVariantList content() const { return m_content; }
    void setContent(const QVariantList &content);

Q_SIGNALS:
    void contentChanged();

protected:
    QSGNode *updatePaintNode(QSGNode *old, UpdatePaintNodeData *) override;
    void itemChange(ItemChange change, const ItemChangeData &value) override;
    void geometryChange(const QRectF &newGeometry, const QRectF &oldGeometry) override;

private:
    QVariantList m_content;
    bool m_software = false;
};
