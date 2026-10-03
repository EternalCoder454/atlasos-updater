// LiveChartItem: a line-and-area chart of the last 60 samples, drawn with
// QPainter so it renders on Qt Quick's software backend as well as on the
// GPU. Use it through LiveChart.qml, which gives it the theme's colours and
// font. Measured and tuned in atlasos-monitor's bench/chart (README there).
#pragma once

#include <QColor>
#include <QFont>
#include <QQuickPaintedItem>
#include <QStaticText>
#include <QtQml/qqmlregistration.h>

#include <array>

class LiveChartItem : public QQuickPaintedItem
{
    Q_OBJECT
    QML_NAMED_ELEMENT(LiveChartItem)

    // The samples, oldest first. Only the last 60 are drawn; the newest sits
    // at the right edge. values2 is an optional second series (upload beside
    // download), drawn over the first.
    Q_PROPERTY(QList<qreal> values READ values WRITE setValues NOTIFY valuesChanged)
    Q_PROPERTY(QList<qreal> values2 READ values2 WRITE setValues2 NOTIFY values2Changed)
    // The value at the top of the plot. 0 (the default) scales to the
    // samples: 1.25 times the highest one shown, and at least minimumScale.
    Q_PROPERTY(qreal maximum READ maximum WRITE setMaximum NOTIFY maximumChanged)
    Q_PROPERTY(qreal minimumScale READ minimumScale WRITE setMinimumScale NOTIFY maximumChanged)
    // The value the top of the plot stands for now, for a caption. (Not
    // `top`: QQuickItem has a final member of that name.)
    Q_PROPERTY(qreal scaleTop READ scaleTop NOTIFY scaleTopChanged)

    Q_PROPERTY(QColor color MEMBER m_color NOTIFY styleChanged)
    Q_PROPERTY(QColor color2 MEMBER m_color2 NOTIFY styleChanged)
    Q_PROPERTY(QColor textColor MEMBER m_textColor NOTIFY styleChanged)
    Q_PROPERTY(QFont font MEMBER m_font NOTIFY styleChanged)
    // Captions, already formatted: label and valueText top left, topText
    // top right, spanText bottom left ("60 seconds"), and 0 bottom right.
    Q_PROPERTY(bool captions MEMBER m_captions NOTIFY styleChanged)
    Q_PROPERTY(QString label MEMBER m_label NOTIFY styleChanged)
    Q_PROPERTY(QString valueText MEMBER m_valueText NOTIFY styleChanged)
    Q_PROPERTY(QString topText MEMBER m_topText NOTIFY styleChanged)
    Q_PROPERTY(QString spanText MEMBER m_spanText NOTIFY styleChanged)

public:
    static constexpr int Capacity = 60;

    explicit LiveChartItem(QQuickItem *parent = nullptr);

    QList<qreal> values() const { return m_values; }
    void setValues(const QList<qreal> &v);
    QList<qreal> values2() const { return m_values2; }
    void setValues2(const QList<qreal> &v);
    qreal maximum() const { return m_maximum; }
    void setMaximum(qreal m);
    qreal minimumScale() const { return m_minimumScale; }
    void setMinimumScale(qreal m);
    qreal scaleTop() const { return m_top; }

    void paint(QPainter *p) override;

Q_SIGNALS:
    void valuesChanged();
    void values2Changed();
    void maximumChanged();
    void scaleTopChanged();
    void styleChanged();

private:
    void updateTop();
    void drawSeries(QPainter *p, const QList<qreal> &values, const QColor &color, double fillAlpha, double w, double top, double plotH);
    void drawText(QPainter *p, QStaticText &t, QString &shown, const QString &text, QPointF at, double alpha);

    QList<qreal> m_values, m_values2;
    qreal m_maximum = 0;
    qreal m_minimumScale = 1;
    qreal m_top = 1;

    QColor m_color{0x3d, 0xae, 0xe9};
    QColor m_color2{0xf6, 0x74, 0x00};
    QColor m_textColor{0x23, 0x26, 0x29};
    QFont m_font;
    bool m_captions = true;
    QString m_label, m_valueText, m_topText, m_spanText;

    QStaticText m_labelStatic, m_valueStatic, m_topStatic, m_spanStatic, m_zeroStatic;
    QString m_shownLabel, m_shownValue, m_shownTop, m_shownSpan, m_shownZero;
};
