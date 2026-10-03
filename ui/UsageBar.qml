import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami

// A stacked bar of how something is shared out (memory: Used, Cached, Free),
// with an optional legend under it. Give it the parts' sizes in `values`, in
// order; what is left of `total` is drawn as the empty track.
//
//   UsageBar {
//       total: memory.total
//       values: [memory.used, memory.cached]
//       labels: [qsTr("Used"), qsTr("Cached"), qsTr("Free")]
//       texts: [fmt(memory.used), fmt(memory.cached), fmt(memory.free)]
//   }
//
// `labels` and `texts` may hold one more entry than `values`: the rest of the
// track, shown in the legend with the track's colour.
ColumnLayout {
    id: root

    property list<real> values
    // The whole bar. 0 means the sum of `values`.
    property real total: 0
    // One colour per part; parts past the list get the last colour faded.
    property list<color> colors: [Kirigami.Theme.highlightColor, Qt.alpha(Kirigami.Theme.highlightColor, 0.45)]
    property list<string> labels
    // What the legend shows after each label, already formatted.
    property list<string> texts
    property bool legend: labels.length > 0
    property real barHeight: Math.round(Kirigami.Units.gridUnit * 0.6)

    readonly property real sum: {
        let s = 0;
        for (const v of values) {
            s += Math.max(0, v);
        }
        return s;
    }
    readonly property real whole: total > 0 ? total : sum
    readonly property color trackColor: Qt.alpha(Kirigami.Theme.textColor, 0.1)

    function colorAt(i) {
        if (i < colors.length) {
            return colors[i];
        }
        return Qt.alpha(colors[colors.length - 1], 0.25);
    }

    spacing: Kirigami.Units.smallSpacing
    Layout.fillWidth: true

    Accessible.role: Accessible.Graphic
    Accessible.name: {
        const parts = [];
        for (let i = 0; i < labels.length; ++i) {
            parts.push(labels[i] + (i < texts.length ? " " + texts[i] : ""));
        }
        return parts.join(", ");
    }

    Item {
        id: bar
        Layout.fillWidth: true
        implicitHeight: root.barHeight

        Rectangle {
            anchors.fill: parent
            radius: height / 2
            color: root.trackColor
        }

        // Each part is a plain rectangle; only the ends are rounded, so no
        // clip is needed (clips are the costly part on the software backend).
        Repeater {
            model: root.values.length

            Rectangle {
                id: part
                required property int index

                readonly property real start: {
                    let s = 0;
                    for (let i = 0; i < index; ++i) {
                        s += Math.max(0, root.values[i]);
                    }
                    return root.whole > 0 ? Math.min(1, s / root.whole) : 0;
                }
                readonly property real share: root.whole > 0 ? Math.min(1 - start, Math.max(0, root.values[index]) / root.whole) : 0
                readonly property real r: bar.height / 2

                x: Math.round(start * bar.width)
                width: Math.round((start + share) * bar.width) - x
                height: bar.height
                visible: width > 0
                color: root.colorAt(index)
                topLeftRadius: x < r ? r : 0
                bottomLeftRadius: topLeftRadius
                topRightRadius: x + width > bar.width - r ? r : 0
                bottomRightRadius: topRightRadius
            }
        }
    }

    Flow {
        visible: root.legend
        Layout.fillWidth: true
        spacing: Kirigami.Units.largeSpacing * 2

        Repeater {
            model: root.labels.length

            Row {
                id: key
                required property int index
                spacing: Kirigami.Units.smallSpacing

                Rectangle {
                    anchors.verticalCenter: parent.verticalCenter
                    width: Kirigami.Units.gridUnit * 0.6
                    height: width
                    radius: width / 2
                    color: key.index < root.values.length ? root.colorAt(key.index) : Qt.alpha(Kirigami.Theme.textColor, 0.2)
                }
                QQC2.Label {
                    text: root.labels[key.index]
                    opacity: 0.7
                    font: Kirigami.Theme.smallFont
                    textFormat: Text.PlainText
                }
                QQC2.Label {
                    visible: text.length > 0
                    text: key.index < root.texts.length ? root.texts[key.index] : ""
                    font: Kirigami.Theme.smallFont
                    textFormat: Text.PlainText
                }
            }
        }
    }
}
