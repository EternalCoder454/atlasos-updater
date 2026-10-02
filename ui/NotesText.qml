import QtQuick
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami

// Release notes (Markdown). The backend sanitizes the text (no images, no HTML,
// https links only); the page decides, through backend.isSafeLink, whether a
// clicked link opens.
QQC2.Label {
    id: root

    property string markdown

    // The section title already says "What's new": drop a leading top-level heading.
    text: markdown.replace(/^\s*#{1,3}\s[^\n]*\n+/, "")
    linkColor: Kirigami.Theme.linkColor
    textFormat: Text.MarkdownText
    wrapMode: Text.Wrap
    signal linkClicked(string link)
    onLinkActivated: link => root.linkClicked(link)
}
