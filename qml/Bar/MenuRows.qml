import QtQuick
import Nyx.Ui
import Nyx.PluginView

// A menu's rows as the bar draws them: an icon, a key cap, the label and a check at the
// right; separators; a disabled entry without an id as a dim hint; submenus opening in
// place under their entry, indented beside a line.
Column {
    id: rows

    property var entries: []
    property bool sub: false
    signal picked(string id)

    Repeater {
        model: rows.entries
        delegate: Column {
            id: entry
            required property var modelData
            readonly property var e: modelData
            readonly property bool sep: !!e.separator
            readonly property bool hint: e.enabled === false && !e.id
            readonly property bool hasKids: (e.children || []).length > 0
            property bool expanded: false
            width: rows.width

            Item {
                visible: entry.sep
                width: parent.width
                height: 11
                Rectangle {
                    x: 4
                    y: 5
                    width: parent.width - 8
                    height: 1
                    color: Style.alpha(Style.text, 0.09)
                }
            }
            Rectangle {
                visible: !entry.sep
                width: parent.width
                implicitWidth: line.implicitWidth + 18
                height: entry.hint ? 24 : rows.sub ? 28 : 32
                radius: 7
                color: area.containsMouse && entry.e.enabled !== false && !entry.hint ? Style.alpha(Style.accent, 0.14) : "transparent"
                opacity: entry.e.enabled === false && !entry.hint ? 0.45 : 1
                Row {
                    id: line
                    x: 9
                    anchors.verticalCenter: parent.verticalCenter
                    spacing: 10
                    Icon {
                        visible: !!entry.e.icon
                        anchors.verticalCenter: parent.verticalCenter
                        name: PluginStyle.icon(entry.e.icon)
                        size: 16
                        recolor: true
                        color: area.containsMouse ? Style.accent : Style.dimText
                    }
                    Rectangle {
                        visible: !!entry.e.cap
                        anchors.verticalCenter: parent.verticalCenter
                        width: Math.max(22, capText.implicitWidth + 8)
                        height: 17
                        radius: 5
                        color: "transparent"
                        border.width: 1.3
                        border.color: Style.alpha(Style.text, 0.35)
                        Label {
                            id: capText
                            anchors.centerIn: parent
                            text: entry.e.cap || ""
                            size: 10.5
                            font.weight: Font.Bold
                        }
                    }
                    Label {
                        anchors.verticalCenter: parent.verticalCenter
                        text: entry.e.label || ""
                        size: entry.hint ? 11.5 : rows.sub ? 12.5 : 13
                        color: entry.hint || rows.sub ? Style.dimText : Style.text
                    }
                }
                Icon {
                    visible: entry.e.checked === true || entry.hasKids
                    anchors.right: parent.right
                    anchors.rightMargin: 9
                    anchors.verticalCenter: parent.verticalCenter
                    name: entry.hasKids ? "pan-down-symbolic" : "object-select-symbolic"
                    size: 16
                    recolor: true
                    color: entry.hasKids ? Style.dimText : Style.accent
                    rotation: entry.expanded ? 180 : 0
                }
                MouseArea {
                    id: area
                    anchors.fill: parent
                    hoverEnabled: true
                    enabled: entry.e.enabled !== false && !entry.hint
                    cursorShape: Qt.PointingHandCursor
                    onClicked: entry.hasKids ? (entry.expanded = !entry.expanded) : rows.picked(entry.e.id)
                }
            }
            // A submenu, in place: indented beside a line.
            Item {
                visible: entry.expanded
                width: parent.width
                height: visible ? kids.implicitHeight + 4 : 0
                Rectangle {
                    x: 26
                    y: 2
                    width: 2
                    height: parent.height - 4
                    color: Style.alpha(Style.text, 0.09)
                }
                Loader {
                    id: kids
                    x: 36
                    y: 2
                    width: parent.width - 36
                    Component.onCompleted: if (entry.hasKids)
                        setSource(Qt.resolvedUrl("MenuRows.qml"), { width: Qt.binding(() => kids.width), entries: entry.e.children, sub: true })
                    onLoaded: item.picked.connect(id => rows.picked(id))
                }
            }
        }
    }
}
