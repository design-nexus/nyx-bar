import QtQuick
import Quickshell
import Nyx.Ui

// An item's right-click menu, 6 px below it on the popover surface. A pick goes to the
// plugin as an action and closes the menu; a click outside closes it.
PopupWindow {
    id: m

    required property var item
    readonly property bool keyboardOff: Quickshell.env("NYX_SHELL_NO_KEYBOARD") === "1"
    readonly property var entries: item.entry && item.entry.menu ? item.entry.menu : []

    function open() {
        if (entries.length)
            visible = true;
    }
    function close() {
        visible = false;
    }

    anchor.item: item
    anchor.rect.width: item.width
    anchor.rect.height: item.height + 6
    anchor.edges: Edges.Bottom
    anchor.gravity: Edges.Bottom
    anchor.adjustment: PopupAdjustment.SlideX
    grabFocus: !keyboardOff
    color: "transparent"
    visible: false
    implicitWidth: 270
    implicitHeight: rows.implicitHeight + 10

    Rectangle {
        anchors.fill: parent
        radius: 15
        color: Style.popover
        border.width: 1
        border.color: Style.border
        MenuRows {
            id: rows
            x: 5
            y: 5
            width: parent.width - 10
            entries: m.entries
            onPicked: id => {
                m.close();
                m.item.win.client.request("MenuAction", { monitor: m.item.win.name, plugin: m.item.plugin, item: m.item.itemId, id: id });
            }
        }
    }
}
