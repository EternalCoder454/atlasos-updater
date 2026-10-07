pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Window
import Telamon.Ui

// The drawing of the screen-edge glow: a soft light that is strongest at the
// edge of a screen-sized item and fades to nothing `depth` pixels in. It has
// no rim and no outline: the falloff is a smooth curve all the way down to
// zero (value and slope), so there is no line where it ends.
//
// Four bands, one along each edge, each a one-dimensional gradient. The top
// and bottom bands run the full width and the left and right ones the full
// height, so in a corner two bands overlap and the two glows add up the way
// light does (source-over of the same colour: 1 - (1 - a) * (1 - b)). That is
// the whole corner: no separate piece, no quarter circle, no place where one
// gradient stops and another starts, so nothing to see at a seam or an arc.
//
// It is static: the pulse is the opacity of this item (or a parent), set by
// ScreenGlow, so the gradients are never redrawn for it. Rectangles with
// gradients draw on the GPU and with the software renderer alike.
Item {
    id: frame

    // How far the glow reaches in from each edge, in pixels.
    property real depth: 24
    property color tone: TelamonStyle.accent

    // The depth on whole device pixels, so that the bands end on a pixel
    // boundary at fractional scales too.
    readonly property real _dpr: Screen.devicePixelRatio > 0 ? Screen.devicePixelRatio : 1
    readonly property real d: Math.max(1, Math.round(Math.min(depth, width / 2, height / 2) * _dpr)) / _dpr
    // The strength at the screen edge (high contrast: stronger).
    readonly property real _peak: TelamonStyle["highContrast"] === true ? 0.9 : 0.55
    // The falloff exponent: the strength at the fraction `e` of the way from
    // the inner end (0) to the screen edge (1) is `_peak * e^_curve`. Above 1
    // the curve reaches zero with zero slope, so it melts into the screen.
    readonly property real _curve: 1.8

    Accessible.ignored: true

    function shade(e: real): color {
        return Qt.alpha(tone, _peak * Math.pow(e, _curve));
    }

    // The strength along one band, sampled at 25 points (so that the straight
    // pieces between them are too short to see). `edgeFirst`: the screen edge
    // is at the start of the gradient (top, left), else at its end.
    component Falloff: Gradient {
        id: fall
        property bool edgeFirst: false
        GradientStop { position: 0; color: frame.shade(fall.edgeFirst ? 1 : 0) }
        GradientStop { position: 0.0416667; color: frame.shade(fall.edgeFirst ? 0.958333 : 0.0416667) }
        GradientStop { position: 0.0833333; color: frame.shade(fall.edgeFirst ? 0.916667 : 0.0833333) }
        GradientStop { position: 0.125; color: frame.shade(fall.edgeFirst ? 0.875 : 0.125) }
        GradientStop { position: 0.166667; color: frame.shade(fall.edgeFirst ? 0.833333 : 0.166667) }
        GradientStop { position: 0.208333; color: frame.shade(fall.edgeFirst ? 0.791667 : 0.208333) }
        GradientStop { position: 0.25; color: frame.shade(fall.edgeFirst ? 0.75 : 0.25) }
        GradientStop { position: 0.291667; color: frame.shade(fall.edgeFirst ? 0.708333 : 0.291667) }
        GradientStop { position: 0.333333; color: frame.shade(fall.edgeFirst ? 0.666667 : 0.333333) }
        GradientStop { position: 0.375; color: frame.shade(fall.edgeFirst ? 0.625 : 0.375) }
        GradientStop { position: 0.416667; color: frame.shade(fall.edgeFirst ? 0.583333 : 0.416667) }
        GradientStop { position: 0.458333; color: frame.shade(fall.edgeFirst ? 0.541667 : 0.458333) }
        GradientStop { position: 0.5; color: frame.shade(fall.edgeFirst ? 0.5 : 0.5) }
        GradientStop { position: 0.541667; color: frame.shade(fall.edgeFirst ? 0.458333 : 0.541667) }
        GradientStop { position: 0.583333; color: frame.shade(fall.edgeFirst ? 0.416667 : 0.583333) }
        GradientStop { position: 0.625; color: frame.shade(fall.edgeFirst ? 0.375 : 0.625) }
        GradientStop { position: 0.666667; color: frame.shade(fall.edgeFirst ? 0.333333 : 0.666667) }
        GradientStop { position: 0.708333; color: frame.shade(fall.edgeFirst ? 0.291667 : 0.708333) }
        GradientStop { position: 0.75; color: frame.shade(fall.edgeFirst ? 0.25 : 0.75) }
        GradientStop { position: 0.791667; color: frame.shade(fall.edgeFirst ? 0.208333 : 0.791667) }
        GradientStop { position: 0.833333; color: frame.shade(fall.edgeFirst ? 0.166667 : 0.833333) }
        GradientStop { position: 0.875; color: frame.shade(fall.edgeFirst ? 0.125 : 0.875) }
        GradientStop { position: 0.916667; color: frame.shade(fall.edgeFirst ? 0.0833333 : 0.916667) }
        GradientStop { position: 0.958333; color: frame.shade(fall.edgeFirst ? 0.0416667 : 0.958333) }
        GradientStop { position: 1; color: frame.shade(fall.edgeFirst ? 0 : 1) }
    }

    // Top, bottom, left, right. The ends overlap in the corners.
    Rectangle {
        x: 0; y: 0; width: frame.width; height: frame.d
        antialiasing: false
        gradient: Falloff { edgeFirst: true }
    }
    Rectangle {
        x: 0; y: frame.height - frame.d; width: frame.width; height: frame.d
        antialiasing: false
        gradient: Falloff { edgeFirst: false }
    }
    Rectangle {
        x: 0; y: 0; width: frame.d; height: frame.height
        antialiasing: false
        gradient: Falloff { orientation: Gradient.Horizontal; edgeFirst: true }
    }
    Rectangle {
        x: frame.width - frame.d; y: 0; width: frame.d; height: frame.height
        antialiasing: false
        gradient: Falloff { orientation: Gradient.Horizontal; edgeFirst: false }
    }
}
