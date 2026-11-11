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
    readonly property var mon: bar.monitors.find(m => m.name === name) || ({ sections: [[], [], []], items: ({}), hidden: false })
    readonly property var screenObj: Quickshell.screens.find(s => s.name === name) || null

    // compact 28, default 32, cozy 36, spacious 40; an exact height sizes the rest.
    readonly property var sizes: ({
        compact: { group: 22, item: 18, font: 12, radius: 7, pad: 7 },
        default: { group: 26, item: 22, font: 13, radius: 8, pad: 8 },
        cozy: { group: 28, item: 24, font: 13, radius: 8, pad: 8 },
        spacious: { group: 30, item: 24, font: 13, radius: 10, pad: 9 }
    })
    readonly property var size: sizes[bar.size] || { group: bar.height - 6, item: bar.height - 10, font: 13, radius: 8, pad: 8 }
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

    // A pill: related items on one faint background, tinted while one of them is open.
    component Pill: Rectangle {
        id: pill
        required property var w
        property var keys: []
        property int openCount: 0
        implicitWidth: pillRow.implicitWidth + 6
        implicitHeight: w.size.group
        radius: w.user.group && w.user.group["border-radius"] !== undefined ? w.user.group["border-radius"] : w.size.radius
        color: openCount > 0 ? w.userColor("groupOpen", Style.mix(Style.panel, Style.accent, 0.14)) : w.userColor("group", Style.alpha(Style.text, 0.055))
        visible: pillRow.shown > 0
        Behavior on color {
            ColorAnimation {
                duration: 120
            }
        }
        Row {
            id: pillRow
            x: 3
            anchors.verticalCenter: parent.verticalCenter
            spacing: 1
            property int shown: 0
            Repeater {
                id: rep
                model: pill.keys
                delegate: BarItem {
                    required property string modelData
                    required property int index
                    key: modelData
                    win: pill.w
                    grouped: true
                    // The first and last shown items of the pill get its end padding.
                    first: index === pill.firstShown
                    last: index === pill.lastShown
                    onIsOpenChanged: pill.openCount += isOpen ? 1 : -1
                    onVisibleChanged: pill.recount()
                    Component.onCompleted: pill.recount()
                }
            }
        }
        property int firstShown: 0
        property int lastShown: 0
        function recount() {
            let first = -1, last = -1, n = 0;
            for (let i = 0; i < rep.count; i++) {
                const it = rep.itemAt(i);
                if (it && it.shownOnBar) {
                    if (first < 0)
                        first = i;
                    last = i;
                    n++;
                }
            }
            firstShown = first;
            lastShown = last;
            pillRow.shown = n;
        }
    }

    component Section: Row {
        id: sec
        required property var w
        property var slots: []
        spacing: 6
        anchors.verticalCenter: parent.verticalCenter
        Repeater {
            model: sec.slots
            delegate: Loader {
                required property var modelData
                anchors.verticalCenter: parent ? parent.verticalCenter : undefined
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
        anchors.fill: parent
        anchors.leftMargin: 8
        anchors.rightMargin: 8
        Section {
            w: win
            id: startSec
            anchors.left: parent.left
            slots: win.mon.sections[0] || []
        }
        Section {
            w: win
            id: centerSec
            // Centred, as GTK's CenterBox does, until a side needs the room; then it
            // moves aside.
            readonly property real gap: 12
            x: Math.max(startSec.width + gap, Math.min((parent.width - width) / 2, endSec.x - gap - width))
            slots: win.mon.sections[1] || []
        }
        Section {
            w: win
            id: endSec
            anchors.right: parent.right
            slots: win.mon.sections[2] || []
        }
    }
}
