.pragma library

// "2026-10-02T04:00:00Z" -> "2 October 2026" in the user's locale.
function longDate(iso) {
    if (!iso) {
        return "";
    }
    var d = new Date(iso);
    if (isNaN(d.getTime())) {
        return iso;
    }
    return d.toLocaleDateString(Qt.locale(), Qt.locale().dateFormat(0)) // 0 = Locale.LongFormat;
}

// Unix seconds -> "Thursday, 1 January 2099 at 03:00", in the user's locale.
function atTime(secs) {
    var d = new Date(secs * 1000);
    return qsTr("%1 at %2").arg(longDate(d.toISOString())).arg(d.toLocaleTimeString(Qt.locale(), Qt.locale().timeFormat(1)));
}

function shortDateTime(secs) {
    return new Date(secs * 1000).toLocaleString(Qt.locale(), Qt.locale().dateTimeFormat(1)) // 1 = Locale.ShortFormat;
}

// Compact numeric date for tight places (button labels).
function shortDate(iso) {
    if (!iso) {
        return "";
    }
    var d = new Date(iso);
    if (isNaN(d.getTime())) {
        return iso;
    }
    return d.toLocaleDateString(Qt.locale(), Qt.locale().dateFormat(1)) // 1 = Locale.ShortFormat;
}
