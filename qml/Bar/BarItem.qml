import QtQuick
import Quickshell
import Nyx.Ui
import Nyx.PluginView
import "hostrules.js" as Host

// One plugin item on the bar: its view, the item's own look from the plugin's CSS (`&`),
// lit while hovered or open; a click runs its command, opens its popover or menu, or
// goes to the plugin; scrolling too. Its popover is built ahead, so it opens at once.
Item {
    id: it

    required property string key
    required property var win
    property bool grouped: false
    property bool first: false
    property bool last: false

    readonly property var entry: win.mon.items[key] || null
    readonly property bool shownOnBar: entry !== null && entry.visible !== false
    readonly property string plugin: key.split("\t")[0]
    readonly property string itemId: key.split("\t")[1] || ""
    readonly property var rules: win.app.rulesFor(plugin)
    property bool popOpen: false
    property bool menuOpen: false
    // Lit while its popover is open or the plugin says so; a menu doesn't light it, as in
    // the GTK bar.
    readonly property bool isOpen: popOpen || !!(entry && entry.open)
    readonly property var rootClasses: ["bar-item", "plugin-" + plugin.replace(/\./g, "_"), "item-" + itemId.replace(/[^A-Za-z0-9-]/g, "_")]
        .concat(grouped ? ["grouped"] : [], first ? ["group-first"] : [], last ? ["group-last"] : [], isOpen ? ["open"] : [], entry ? (entry["class"] || []) : [])
    // The item's own rules (`&.bar-item…`), the bar's then the plugin's.
    readonly property var rs: PluginStyle.compute(rules, [{ root: true, classes: rootClasses, hovered: hover.hovered }])
    function side(k, i) {
        const one = rs[k + ["-top", "-right", "-bottom", "-left"][i]];
        if (one !== undefined)
            return one;
        return rs[k] ? rs[k][i] : (k === "padding" && (i === 1 || i === 3) ? win.size.pad : 0);
    }
    readonly property var pad: [side("padding", 0), side("padding", 1), side("padding", 2), side("padding", 3)]
    readonly property var mar: [side("margin", 0), side("margin", 1), side("margin", 2), side("margin", 3)]

    // A new tree only when the plugin sends the item again; patches change it in place.
    property int rev: -1
    property var shownView: null
    function takeView() {
        if (entry && entry.rev !== rev) {
            rev = entry.rev;
            shownView = entry.view;
        }
    }
    onEntryChanged: takeView()
    Component.onCompleted: {
        takeView();
        win.app.register(key, patcher);
        win.shrinkers = win.shrinkers.concat([it]);
    }
    Component.onDestruction: {
        win.app.unregister(key, patcher);
        win.shrinkers = win.shrinkers.filter(x => x !== it);
        win.refit();
    }
    // What its `shrink` labels can give up when the bar is short of room.
    readonly property real spare: view.spare
    onSpareChanged: win.refit()
    QtObject {
        id: patcher
        readonly property string monitor: it.win.name
        function patch(nodes) {
            view.patch(nodes);
            pop.patch(nodes);
        }
    }

    visible: shownOnBar
    width: visible ? implicitWidth : 0
    implicitWidth: Math.max(rs["min-width"] || 0, view.implicitWidth) + pad[1] + pad[3] + mar[1] + mar[3]
    implicitHeight: grouped ? win.size.group : win.size.item

    Rectangle {
        x: it.mar[3]
        width: it.width - it.mar[1] - it.mar[3]
        height: it.height
        radius: it.rs["border-radius"] !== undefined ? it.rs["border-radius"] : 7
        color: PluginStyle.color(it.rs["background-color"])
        Behavior on color {
            ColorAnimation {
                duration: 120
            }
        }
    }

    PluginView {
        id: view
        x: it.mar[3] + it.pad[3]
        anchors.verticalCenter: parent.verticalCenter
        view: it.shownView
        maxHeight: it.win.bar.height - 8
        rules: it.rules
        rootClasses: it.rootClasses
        rootHovered: hover.hovered
        rootText: Object.assign({ "font-size": it.win.size.font, "icon-size": it.win.size.icon }, it.rs.color ? { color: it.rs.color } : {})
        tabular: true
        tipBelow: true
        squeeze: it.win.squeeze
        onViewEvent: (kind, control, value) => it.win.client.request("ViewEvent", { monitor: it.win.name, plugin: it.plugin, item: it.itemId, kind: kind, control: control, value: value })
    }

    HoverHandler {
        id: hover
    }

    function anchorBox() {
        const p = it.mapToItem(null, 0, 0);
        return { x: p.x, y: p.y, w: it.width, h: it.height };
    }
    function click(button) {
        win.client.request("Click", { monitor: win.name, plugin: plugin, item: itemId, button: button, anchor: anchorBox() }, ok => {
            if (!ok)
                return;
            if (ok.did === "popover")
                pop.toggle();
            else if (ok.did === "menu")
                menu.open();
        });
    }
    // Under the view: a node with an action (a workspace, a button in the item) takes
    // its own clicks; the rest are the item's.
    MouseArea {
        z: -1
        anchors.fill: parent
        acceptedButtons: Qt.LeftButton | Qt.MiddleButton | Qt.RightButton
        cursorShape: Qt.PointingHandCursor
        onClicked: m => it.click(m.button === Qt.RightButton ? 3 : m.button === Qt.MiddleButton ? 2 : 1)
        // Whole wheel steps (120), as GTK sends them: a touchpad's stream of small
        // deltas doesn't turn into a step for each.
        property real wheel: 0
        onWheel: w => {
            wheel += w.angleDelta.y;
            while (Math.abs(wheel) >= 120) {
                const up = wheel > 0;
                wheel -= up ? 120 : -120;
                it.win.client.request("Scroll", { monitor: it.win.name, plugin: it.plugin, item: it.itemId, up: up });
            }
        }
    }

    // The item's tip gives way to a tooltip of a node inside it.
    Tip {
        target: it
        text: it.entry && it.entry.tooltip ? it.entry.tooltip : ""
        shown: hover.hovered && !it.isOpen && !it.menuOpen && view.tipHovers === 0
        below: true
    }

    BarPopover {
        id: pop
        item: it
        onOpenedChanged: it.popOpen = opened
    }
    BarMenu {
        id: menu
        item: it
        onVisibleChanged: it.menuOpen = visible
    }

    Connections {
        target: it.win ? it.win.app : null
        function onTestClick(plugin, item, button) {
            if (plugin === it.plugin && item === it.itemId)
                it.click(button);
        }
        function onTestClose() {
            pop.close();
            menu.close();
        }
    }
}
