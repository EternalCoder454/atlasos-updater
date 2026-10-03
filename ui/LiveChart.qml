import QtQuick
import org.kde.kirigami as Kirigami

// A live chart in the theme's colours: the accent for the first series, the
// neutral colour for the second, the theme's small font for the captions.
// Feed it `values` (and `values2`) as a list of numbers, oldest first; it
// repaints when they change. See livechart.h for every property.
//
//   LiveChart {
//       values: cpu.usageHistory
//       maximum: 100
//       label: qsTr("Load")
//       valueText: Math.round(cpu.usage) + "%"
//       topText: "100%"
//       spanText: qsTr("60 seconds")
//   }
LiveChartItem {
    id: chart

    implicitWidth: Kirigami.Units.gridUnit * 20
    implicitHeight: Kirigami.Units.gridUnit * 8

    color: Kirigami.Theme.highlightColor
    color2: Kirigami.Theme.neutralTextColor
    textColor: Kirigami.Theme.textColor
    font: Kirigami.Theme.smallFont

    Accessible.role: Accessible.Chart
    Accessible.name: chart.label
    Accessible.description: chart.valueText
}
