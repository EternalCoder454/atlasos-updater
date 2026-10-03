#include "livechart.h"

#include <QFontMetricsF>
#include <QPainter>

#include <algorithm>
#include <cmath>

// Why it draws the way it does (measured on the software backend, Qt 6.11,
// atlasos-monitor bench/chart/README.md):
// - The line is one small anti-aliased quad per segment, not a stroked
//   polyline: Qt's AA rasterizer is slow on long thin nearly flat shapes, and
//   the quads are pixel-equivalent to a round-joined 1.5 px stroke. They
//   overlap at the joins, so line colours must be opaque.
// - The fill is drawn without anti-aliasing; the line covers its sloped edge.
// - Grid and border are filled 1-device-pixel rectangles, not translucent
//   lines, which take Qt's per-pixel line path.
// - Captions are QStaticText, laid out again only when their text changes.

namespace
{
constexpr double LineHalfWidth = 0.75;

// The highest of the last Capacity samples of v, or 0.
double peakOf(const QList<qreal> &v)
{
    const qsizetype n = std::min<qsizetype>(v.size(), LiveChartItem::Capacity);
    double m = 0;
    for (qsizetype i = v.size() - n; i < v.size(); ++i) {
        if (std::isfinite(v[i])) {
            m = std::max(m, v[i]);
        }
    }
    return m;
}
}

LiveChartItem::LiveChartItem(QQuickItem *parent)
    : QQuickPaintedItem(parent)
{
    setAntialiasing(true);
    // A change of look repaints now, not at the next tick. Captions are laid
    // out again only when their text changes (drawText sees that), or for a
    // new font.
    connect(this, &LiveChartItem::styleChanged, this, [this] { update(); });
    connect(this, &LiveChartItem::fontChanged, this, [this] {
        forgetCaptions();
        update();
    });
}

void LiveChartItem::forgetCaptions()
{
    m_shownLabel = m_shownValue = m_shownTop = m_shownSpan = m_shownZero = QString();
}

// Captions are prepared for the screen's scale; on another screen, again.
void LiveChartItem::itemChange(ItemChange change, const ItemChangeData &data)
{
    if (change == ItemDevicePixelRatioHasChanged) {
        forgetCaptions();
    }
    QQuickPaintedItem::itemChange(change, data);
}

void LiveChartItem::setValues(const QList<qreal> &v)
{
    m_values = v;
    updateTop();
    update();
    Q_EMIT valuesChanged();
}

void LiveChartItem::setValues2(const QList<qreal> &v)
{
    m_values2 = v;
    updateTop();
    update();
    Q_EMIT values2Changed();
}

void LiveChartItem::setMaximum(qreal m)
{
    if (m == m_maximum) {
        return;
    }
    m_maximum = m;
    updateTop();
    update();
    Q_EMIT maximumChanged();
}

void LiveChartItem::setMinimumScale(qreal m)
{
    if (m == m_minimumScale) {
        return;
    }
    m_minimumScale = m;
    updateTop();
    update();
    Q_EMIT minimumScaleChanged();
}

void LiveChartItem::updateTop()
{
    double top = m_maximum;
    if (!(top > 0)) {
        top = std::max({m_minimumScale, peakOf(m_values) * 1.25, peakOf(m_values2) * 1.25});
    }
    if (!(top > 0) || !std::isfinite(top)) {
        top = 1;
    }
    if (top != m_top) {
        m_top = top;
        Q_EMIT scaleTopChanged();
    }
}

// Sets the static text only when it changed, so a steady caption is not laid
// out again, then draws it.
void LiveChartItem::drawText(QPainter *p, QStaticText &t, QString &shown, const QString &text, QPointF at, double alpha)
{
    if (text.isEmpty()) {
        return;
    }
    if (text != shown) {
        shown = text;
        t.setText(text);
        t.setTextFormat(Qt::PlainText);
        t.prepare(p->transform(), p->font());
    }
    QColor c = m_textColor;
    c.setAlphaF(alpha);
    p->setPen(c);
    p->drawStaticText(at, t);
}

void LiveChartItem::drawSeries(QPainter *p, const QList<qreal> &values, const QColor &color, double fillAlpha, double w, double top, double plotH)
{
    const int n = int(std::min<qsizetype>(values.size(), Capacity));
    const double dx = w / (Capacity - 1);
    const qsizetype first = values.size() - n;
    // A sample that isn't a number (a reading that failed) breaks the line
    // rather than dropping it to the floor: each unbroken run is drawn alone.
    for (int i = 0; i < n;) {
        if (!std::isfinite(values[first + i])) {
            ++i;
            continue;
        }
        int end = i;
        while (end < n && std::isfinite(values[first + end])) {
            ++end;
        }
        drawRun(p, values.constData() + first, i, end, color, fillAlpha, w - dx * (n - 1), dx, top + plotH, plotH);
        i = end;
    }
}

// Samples [from, to), sample k at x0 + dx * k.
void LiveChartItem::drawRun(QPainter *p, const qreal *values, int from, int to, const QColor &color, double fillAlpha, double x0, double dx, double bottom, double plotH)
{
    const int m = to - from;
    if (m < 2) {
        return;
    }
    QPointF pts[Capacity + 2];
    for (int i = 0; i < m; ++i) {
        const double r = std::clamp(values[from + i] / m_top, 0.0, 1.0);
        pts[i + 1] = QPointF(x0 + dx * (from + i), bottom - r * plotH);
    }
    pts[0] = QPointF(pts[1].x(), bottom);
    pts[m + 1] = QPointF(pts[m].x(), bottom);

    QColor fill = color;
    fill.setAlphaF(fillAlpha);
    p->setPen(Qt::NoPen);
    p->setBrush(fill);
    p->setRenderHint(QPainter::Antialiasing, false);
    p->drawPolygon(pts, m + 2);

    // Each segment its own quad, extended half the width at both ends so the
    // joins have no gaps.
    p->setRenderHint(QPainter::Antialiasing, true);
    p->setBrush(color);
    const QPointF *v = pts + 1;
    for (int i = 0; i + 1 < m; ++i) {
        const QPointF d = v[i + 1] - v[i];
        const double l = std::hypot(d.x(), d.y());
        if (l <= 0) {
            continue;
        }
        const QPointF u = d / l * LineHalfWidth;
        const QPointF nn(-u.y(), u.x());
        const QPointF a = v[i] - u, b = v[i + 1] + u;
        const QPointF quad[4] = {a + nn, b + nn, b - nn, a - nn};
        p->drawConvexPolygon(quad, 4);
    }
}

void LiveChartItem::paint(QPainter *p)
{
    const double dpr = p->device()->devicePixelRatioF();
    const double w = width();
    const double h = height();
    if (w <= 0 || h <= 0) {
        return;
    }
    p->setFont(m_font);
    const QFontMetricsF fm(m_font);
    const double band = m_captions ? std::ceil(fm.height()) + 4 : 0;
    const double top = band;
    const double plotH = std::max(0.0, h - 2 * band);
    const double px = 1 / dpr;
    auto snap = [dpr](double v) { return std::floor(v * dpr) / dpr; };

    if (plotH >= 2) {
        // Grid: rows divide the plot evenly, columns repeat the row height
        // leftwards from the newest sample, so every cell is square.
        QColor grid = m_textColor;
        grid.setAlphaF(0.08);
        p->setRenderHint(QPainter::Antialiasing, false);
        const int rows = std::clamp(int(std::round(plotH / 36)), 2, 10);
        const double cell = plotH / rows;
        for (int i = 1; i < rows; ++i) {
            p->fillRect(QRectF(0, snap(top + cell * i), w, px), grid);
        }
        for (int k = 1;; ++k) {
            const double x = w - cell * k;
            if (x <= 1) {
                break;
            }
            p->fillRect(QRectF(snap(x), top, px, plotH), grid);
        }

        p->save();
        p->setClipRect(QRectF(0, top, w, plotH));
        drawSeries(p, m_values, m_color, 0.22, w, top, plotH);
        drawSeries(p, m_values2, m_color2, 0.14, w, top, plotH);
        p->restore();

        grid.setAlphaF(0.30);
        p->setRenderHint(QPainter::Antialiasing, false);
        const double t = snap(top), b = snap(top + plotH) - px, r = snap(w) - px;
        p->fillRect(QRectF(0, t, w, px), grid);
        p->fillRect(QRectF(0, b, w, px), grid);
        p->fillRect(QRectF(0, t + px, px, b - t - px), grid);
        p->fillRect(QRectF(r, t + px, px, b - t - px), grid);
    }

    if (m_captions) {
        p->setRenderHint(QPainter::TextAntialiasing, true);
        const QString label = m_label.isEmpty() ? QString() : m_label + QStringLiteral("  ");
        drawText(p, m_labelStatic, m_shownLabel, label, QPointF(1, 2), 0.66);
        const double lw = label.isEmpty() ? 0 : m_labelStatic.size().width();
        drawText(p, m_valueStatic, m_shownValue, m_valueText, QPointF(1 + lw, 2), 0.92);
        const double vw = m_valueText.isEmpty() ? 0 : m_valueStatic.size().width();
        const double tw = fm.horizontalAdvance(m_topText);
        // On a narrow chart the scale gives way to the reading.
        if (1 + lw + vw + fm.averageCharWidth() * 2 <= w - tw - 1) {
            drawText(p, m_topStatic, m_shownTop, m_topText, QPointF(w - tw - 1, 2), 0.5);
        }
        const double by = h - band + 2;
        drawText(p, m_spanStatic, m_shownSpan, m_spanText, QPointF(1, by), 0.5);
        const QString zero = QStringLiteral("0");
        drawText(p, m_zeroStatic, m_shownZero, zero, QPointF(w - fm.horizontalAdvance(zero) - 1, by), 0.5);
    }
}
