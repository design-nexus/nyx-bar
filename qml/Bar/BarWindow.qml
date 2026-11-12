import QtQuick
import Quickshell
import Quickshell.Wayland
import Nyx.Ui
import Nyx.PluginView

// One screen's bar: a full-width strip at the top in the bar's colour, the left, centre
// and right sections, related items sharing a pill, thin dividers.
PanelWindow {
    id: win

    required property string name
    required property var client
    required property var app
    readonly property var bar: app.bar
    readonly property var mon: (bar.monitors || {})[name] || ({ sections: [[], [], []], items: ({}), hidden: false })
    readonly property var screenObj: Quickshell.screens.find(s => s.name === name) || null

    // compact 28, default 32, cozy 36, spacious 40; an exact height sizes the rest.
    readonly property var sizes: ({
        compact: { group: 22, item: 18, font: 12, radius: 7, pad: 7, icon: 14 },
        default: { group: 26, item: 22, font: 13, radius: 8, pad: 8, icon: 16 },
        cozy: { group: 28, item: 24, font: 13, radius: 8, pad: 8, icon: 16 },
        spacious: { group: 30, item: 24, font: 13, radius: 10, pad: 9, icon: 18 }
    })
    readonly property var size: sizes[bar.size] || { group: bar.height - 6, item: bar.height - 10, font: 13, radius: 8, pad: 8, icon: 16 }
    // The user's bar.css for the bar, its pills and dividers (their GTK names).
    readonly property var user: bar.user || ({})
    function userColor(part, fallback) {
        const p = user[part];
        return p && p["background-color"] ? PluginStyle.color(p["background-color"]) : fallback;
    }

    screen: screenObj
    visible: screenObj !== null && !mon.hidden
    anchors.top: true
    anchors.left: true
    anchors.right: true
    implicitHeight: bar.height
    exclusiveZone: bar.height
    color: userColor("bar", Style.panel)
    WlrLayershell.layer: WlrLayer.Top
    WlrLayershell.namespace: "nyx-bar"
    WlrLayershell.keyboardFocus: WlrKeyboardFocus.None

    // Short of room, titles give way: items' labels marked `shrink` give up this share
    // of their spare width (as GTK ellipsizes them). Worked out after layout, not in a
    // binding, as the sections' widths depend on it.
    property real squeeze: 0
    property var shrinkers: []
    function fit() {
        let spare = 0;
        for (const it of shrinkers)
            if (it.visible && it.shownOnBar)
                spare += it.spare;
        const taken = spare * squeeze;
        const natural = startSec.width + centerSec.width + endSec.width + taken + 2 * centerSec.gap;
        const over = Math.max(0, natural - sections.width);
        const next = spare > 0 ? Math.min(1, over / spare) : 0;
        if (Math.abs(next - squeeze) > 0.002)
            squeeze = next;
    }
    function refit() {
        Qt.callLater(fit);
    }

    // Whether an item shows now, from the state the backend sent.
    function itemShown(key) {
        const e = mon.items[key];
        return !!e && e.visible !== false;
    }
    function slotShown(slot) {
        return slot.kind === "group" ? slot.keys.some(k => itemShown(k)) : slot.kind === "item" ? itemShown(slot.key) : null;
    }
    // nyx-bar-core's shown_dividers: a divider only between shown slots.
    function shownDividers(slots) {
        const out = slots.map(() => false);
        let seen = false, pending = -1;
        slots.forEach((s, i) => {
            const on = slotShown(s);
            if (on === true) {
                if (pending >= 0)
                    out[pending] = true;
                pending = -1;
                seen = true;
            } else if (on === null && seen)
                pending = i;
        });
        return out;
    }

    // A pill: related items on one faint background (never tinted while one is open).
    component Pill: Rectangle {
        id: pill
        required property var w
        property var keys: []
        // What shows comes from the items' state, not from the children's visibility:
        // a hidden pill's children are hidden too and could never bring it back.
        readonly property var shownKeys: keys.filter(k => w.itemShown(k))
        visible: shownKeys.length > 0
        implicitWidth: pillRow.implicitWidth + 6
        implicitHeight: w.size.group
        radius: w.user.group && w.user.group["border-radius"] !== undefined ? w.user.group["border-radius"] : w.size.radius
        color: w.userColor("group", Style.alpha(Style.text, 0.055))
        Row {
            id: pillRow
            x: 3
            anchors.verticalCenter: parent.verticalCenter
            spacing: 1
            Repeater {
                model: pill.keys
                delegate: BarItem {
                    required property string modelData
                    key: modelData
                    win: pill.w
                    grouped: true
                    // The first and last shown items of the pill get its end padding.
                    first: pill.shownKeys[0] === modelData
                    last: pill.shownKeys[pill.shownKeys.length - 1] === modelData
                }
            }
        }
    }

    component Section: Row {
        id: sec
        required property var w
        property var slots: []
        readonly property var dividers: w.shownDividers(slots)
        spacing: 6
        anchors.verticalCenter: parent.verticalCenter
        Repeater {
            model: sec.slots
            delegate: Loader {
                required property var modelData
                required property int index
                anchors.verticalCenter: parent ? parent.verticalCenter : undefined
                // A Row skips what isn't visible, so a hidden item, an empty pill or a
                // divider with nothing beside it leaves no gap.
                visible: modelData.kind === "divider" ? !!sec.dividers[index] : sec.w.slotShown(modelData) === true
                sourceComponent: modelData.kind === "group" ? pillC : modelData.kind === "divider" ? dividerC : itemC
                Component {
                    id: pillC
                    Pill {
                        w: sec.w
                        keys: modelData.keys
                    }
                }
                Component {
                    id: itemC
                    BarItem {
                        key: modelData.key
                        win: sec.w
                    }
                }
                Component {
                    id: dividerC
                    Item {
                        implicitWidth: 9
                        implicitHeight: 16
                        Rectangle {
                            x: 4
                            width: 1
                            height: 16
                            color: sec.w.userColor("divider", Style.alpha(Style.text, 0.12))
                        }
                    }
                }
            }
        }
    }

    Item {
        id: sections
        anchors.fill: parent
        anchors.leftMargin: 8
        anchors.rightMargin: 8
        onWidthChanged: win.refit()
        Section {
            w: win
            id: startSec
            anchors.left: parent.left
            slots: win.mon.sections[0] || []
            onWidthChanged: win.refit()
        }
        Section {
            w: win
            id: centerSec
            // Centred, as GTK's CenterBox does, until a side needs the room; then it
            // moves aside.
            readonly property real gap: 12
            x: Math.max(startSec.width + gap, Math.min((parent.width - width) / 2, endSec.x - gap - width))
            slots: win.mon.sections[1] || []
            onWidthChanged: win.refit()
        }
        Section {
            w: win
            id: endSec
            anchors.right: parent.right
            slots: win.mon.sections[2] || []
            onWidthChanged: win.refit()
        }
    }
}
