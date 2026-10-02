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

function shortDateTime(secs) {
    return new Date(secs * 1000).toLocaleString(Qt.locale(), Qt.locale().dateTimeFormat(1)) // 1 = Locale.ShortFormat;
}
