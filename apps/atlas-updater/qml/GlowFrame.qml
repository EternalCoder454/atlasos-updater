pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Shapes
import QtQuick.Window
import Atlas.Ui

// The drawing of the screen-edge glow: one continuous frame along all four
// edges of a screen-sized item, strongest at the edge and gone `depth` pixels
// in. Four straight bands (linear gradients) and four corner squares (radial
// gradients centred on the inner corner, so the glow turns each corner in a
// quarter circle) share one set of stops, so where they meet the colour is the
// same on both sides: no seam, no gap, no overlap. The pieces are drawn
// without antialiasing: they share their edges exactly, and antialiased edges
// would overlap there and show as lines.
//
// It is static: the pulse is the opacity of this item (or a parent), set by
// ScreenGlow, so the gradients are never redrawn for it. Shapes draw on the
// GPU and with the software renderer alike.
Item {
    id: frame

    // How far the glow reaches in from each edge, in pixels.
    property real depth: 54
    property color tone: AtlasStyle.accent

    // The depth on whole device pixels, so that the pieces along the top and
    // left meet on a pixel boundary at fractional scales too.
    readonly property real _dpr: Screen.devicePixelRatio > 0 ? Screen.devicePixelRatio : 1
    readonly property real d: Math.max(1, Math.round(Math.min(depth, width / 2, height / 2) * _dpr)) / _dpr
    // From the inner end (0, nothing) to the screen edge (1): an ease-in curve
    // with a lighter rim along the very edge.
    readonly property real _top: AtlasStyle["highContrast"] === true ? 1 : 0.9
    readonly property color _rim: Qt.tint(tone, Qt.rgba(1, 1, 1, 0.3))

    Accessible.ignored: true

    // The shared stops (inner end first). Each gradient type repeats this list.
    component Stops: QtObject {
        readonly property color s0: Qt.alpha(frame.tone, 0)
        readonly property color s1: Qt.alpha(frame.tone, frame._top * 0.07)
        readonly property color s2: Qt.alpha(frame.tone, frame._top * 0.2)
        readonly property color s3: Qt.alpha(frame.tone, frame._top * 0.36)
        readonly property color s4: Qt.alpha(frame.tone, frame._top * 0.56)
        readonly property color s5: Qt.alpha(frame.tone, frame._top * 0.78)
        readonly property color s6: Qt.alpha(frame._rim, frame._top * 0.92)
        readonly property color s7: Qt.alpha(frame._rim, frame._top)
    }
    Stops {
        id: stops
    }

    // A straight band: inner end at (x1, y1), screen edge at (x2, y2).
    component Band: LinearGradient {
        spread: ShapeGradient.PadSpread
        GradientStop { position: 0; color: stops.s0 }
        GradientStop { position: 0.25; color: stops.s1 }
        GradientStop { position: 0.45; color: stops.s2 }
        GradientStop { position: 0.6; color: stops.s3 }
        GradientStop { position: 0.75; color: stops.s4 }
        GradientStop { position: 0.88; color: stops.s5 }
        GradientStop { position: 0.96; color: stops.s6 }
        GradientStop { position: 1; color: stops.s7 }
    }
    // A corner: centred on the inner corner (cx, cy), the edge at radius d;
    // beyond it (the corner's tip) the edge colour carries on (pad).
    component Corner: RadialGradient {
        required property real cx
        required property real cy
        spread: ShapeGradient.PadSpread
        centerX: cx
        centerY: cy
        focalX: cx
        focalY: cy
        centerRadius: frame.d
        focalRadius: 0
        GradientStop { position: 0; color: stops.s0 }
        GradientStop { position: 0.25; color: stops.s1 }
        GradientStop { position: 0.45; color: stops.s2 }
        GradientStop { position: 0.6; color: stops.s3 }
        GradientStop { position: 0.75; color: stops.s4 }
        GradientStop { position: 0.88; color: stops.s5 }
        GradientStop { position: 0.96; color: stops.s6 }
        GradientStop { position: 1; color: stops.s7 }
    }
    // One rectangle of the frame, (x0, y0) to (x1, y1), filled, no stroke.
    component Piece: ShapePath {
        id: piece
        required property real x0
        required property real y0
        required property real x1
        required property real y1
        strokeWidth: -1
        strokeColor: "transparent"
        startX: piece.x0
        startY: piece.y0
        PathLine { x: piece.x1; y: piece.y0 }
        PathLine { x: piece.x1; y: piece.y1 }
        PathLine { x: piece.x0; y: piece.y1 }
        PathLine { x: piece.x0; y: piece.y0 }
    }

    Shape {
        anchors.fill: parent
        preferredRendererType: Shape.GeometryRenderer
        antialiasing: false

        // Bands: top, bottom, left, right (between the corners).
        Piece {
            x0: frame.d; y0: 0; x1: frame.width - frame.d; y1: frame.d
            fillGradient: Band { x1: 0; y1: frame.d; x2: 0; y2: 0 }
        }
        Piece {
            x0: frame.d; y0: frame.height - frame.d; x1: frame.width - frame.d; y1: frame.height
            fillGradient: Band { x1: 0; y1: frame.height - frame.d; x2: 0; y2: frame.height }
        }
        Piece {
            x0: 0; y0: frame.d; x1: frame.d; y1: frame.height - frame.d
            fillGradient: Band { x1: frame.d; y1: 0; x2: 0; y2: 0 }
        }
        Piece {
            x0: frame.width - frame.d; y0: frame.d; x1: frame.width; y1: frame.height - frame.d
            fillGradient: Band { x1: frame.width - frame.d; y1: 0; x2: frame.width; y2: 0 }
        }
        // Corners: top left, top right, bottom left, bottom right.
        Piece {
            x0: 0; y0: 0; x1: frame.d; y1: frame.d
            fillGradient: Corner { cx: frame.d; cy: frame.d }
        }
        Piece {
            x0: frame.width - frame.d; y0: 0; x1: frame.width; y1: frame.d
            fillGradient: Corner { cx: frame.width - frame.d; cy: frame.d }
        }
        Piece {
            x0: 0; y0: frame.height - frame.d; x1: frame.d; y1: frame.height
            fillGradient: Corner { cx: frame.d; cy: frame.height - frame.d }
        }
        Piece {
            x0: frame.width - frame.d; y0: frame.height - frame.d; x1: frame.width; y1: frame.height
            fillGradient: Corner { cx: frame.width - frame.d; cy: frame.height - frame.d }
        }
    }
}
