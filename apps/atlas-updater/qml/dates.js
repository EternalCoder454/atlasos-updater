.pragma library
.import Atlas.Ui 1.0 as Ui

// "2026-10-02T04:00:00Z" -> "2 October 2026" in the user's locale.
function longDate(iso) {
    if (!iso) {
        return "";
    }
    var d = new Date(iso);
    if (isNaN(d.getTime())) {
        return iso;
    }
    return Ui.AtlasFormat.date(d, "long");
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
    return Ui.AtlasFormat.date(d, "short");
}

// Unix seconds -> "Today at 9:41", "Yesterday at 18:02" or the full date,
// as seen at `nowMs` (pass the page's clock so it updates after midnight).
function relative(secs, nowMs) {
    var d = new Date(secs * 1000);
    if (isNaN(d.getTime())) {
        return "";
    }
    var time = d.toLocaleTimeString(Qt.locale(), Qt.locale().timeFormat(1));
    var today = new Date(nowMs);
    today.setHours(0, 0, 0, 0);
    var day = new Date(d.getTime());
    day.setHours(0, 0, 0, 0);
    var days = Math.round((today.getTime() - day.getTime()) / 86400000);
    if (days === 0) {
        return qsTr("Today at %1").arg(time);
    }
    if (days === 1) {
        return qsTr("Yesterday at %1").arg(time);
    }
    return atTime(secs);
}
