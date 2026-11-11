import QtQuick
import QtQuick.Effects
import Quickshell
import Nyx.Ui
import Nyx.PluginView

// An item's popover: 6 px below it, on the popover surface (15 px corners, 12 px inside,
// a soft shadow), the plugin's view at 13 px with its CSS. Built while closed, so opening
// it only shows it. A click outside closes it (no grab in test runs).
PopupWindow {
    id: p

    required property var item
    property bool opened: false
    readonly property bool keyboardOff: Quickshell.env("NYX_SHELL_NO_KEYBOARD") === "1"
    readonly property var entry: item.entry
    readonly property int room: 16
    readonly property int maxWidth: 960
    readonly property int maxHeight: 680

    // Plugins send their popover again as it changes: a new tree only when it differs.
    property string _text: ""
    property var shown: null
    function take() {
        const v = entry ? entry.popover : null;
        const t = v ? JSON.stringify(v) : "";
        if (t !== _text) {
            _text = t;
            shown = v;
        }
    }
    onEntryChanged: take()
    Component.onCompleted: take()

    function patch(nodes) {
        view.patch(nodes);
    }
    function toggle() {
        opened ? close() : open();
    }
    function open() {
        if (!shown)
            return;
        visible = true;
        opened = true;
        item.win.client.request("PopoverOpened", { monitor: item.win.name, plugin: item.plugin, item: item.itemId, anchor: item.anchorBox() });
    }
    function close() {
        visible = false;
    }
    onVisibleChanged: if (!visible && opened) {
        opened = false;
        item.win.client.request("PopoverClosed", { monitor: item.win.name, plugin: item.plugin, item: item.itemId });
    }

    anchor.item: item
    anchor.rect.x: 0
    anchor.rect.y: 0
    anchor.rect.width: item.width
    // Its surface 6 px below the item (the window's shadow room taken off).
    anchor.rect.height: item.height + 6 - room
    anchor.edges: Edges.Bottom
    anchor.gravity: Edges.Bottom
    anchor.adjustment: PopupAdjustment.SlideX
    grabFocus: !keyboardOff
    color: "transparent"
    visible: false
    implicitWidth: surface.width + 2 * room
    implicitHeight: surface.height + 2 * room

    MultiEffect {
        anchors.fill: surface
        source: surface
        shadowEnabled: true
        shadowColor: Style.alpha(Style.shadow, 0.6)
        shadowBlur: 0.9
        blurMax: 30
        shadowVerticalOffset: 8
    }
    Rectangle {
        id: surface
        x: p.room
        y: p.room
        width: Math.min(p.maxWidth, view.implicitWidth) + 24
        height: Math.min(p.maxHeight, view.implicitHeight) + 24
        radius: 15
        color: Style.popover
        border.width: 1
        border.color: Style.border

        Flickable {
            x: 12
            y: 12
            width: parent.width - 24
            height: parent.height - 24
            clip: view.implicitHeight > p.maxHeight
            contentWidth: width
            contentHeight: view.implicitHeight
            boundsBehavior: Flickable.StopAtBounds
            PluginView {
                id: view
                width: parent.width
                height: implicitHeight
                view: p.shown
                rules: p.item.rules
                rootClasses: ["nyx-bar-popover"]
                rootText: ({ "font-size": 13 })
                tabular: true
                onViewEvent: (kind, control, value) => p.item.win.client.request("ViewEvent", { monitor: p.item.win.name, plugin: p.item.plugin, item: p.item.itemId, kind: kind, control: control, value: value })
            }
        }
    }
}
