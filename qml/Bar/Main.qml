import QtQuick
import Quickshell
import Nyx.Shell
import "hostrules.js" as Host

// Nyx Bar in the shared Quickshell runtime: a bar on each screen its backend
// (nyx-bar-backend) shows one on. The backend runs the plugins and works out the layout;
// this draws it and tells the backend what was clicked.
Scope {
    id: root

    readonly property string pinnedScreen: Quickshell.env("NYX_SHELL_SCREEN") || ""
    readonly property var bar: backend.state.bar || ({ monitors: ({}), styles: ({}), height: 32, size: "default" })

    // Each plugin's rules (the bar's, its style.css, the user's bar.css), the same array
    // until the styles change, so items don't restyle on every update. A plain object
    // changed in place: no bindings follow it.
    readonly property var _rules: ({ styles: null, user: null, byPlugin: {} })
    function rulesFor(plugin) {
        const c = _rules, st = bar.styles || {}, us = bar.user || null;
        if (c.styles !== st || c.user !== us) {
            c.styles = st;
            c.user = us;
            c.byPlugin = {};
        }
        return c.byPlugin[plugin] || (c.byPlugin[plugin] = Host.rules.concat(st[plugin] || [], (us && us.items) || []));
    }

    // Every view on screen by "plugin\titem", for the plugins' patches.
    property var views: ({})
    function register(key, v) {
        (views[key] = views[key] || []).push(v);
    }
    function unregister(key, v) {
        const list = views[key];
        if (list) {
            const i = list.indexOf(v);
            if (i >= 0)
                list.splice(i, 1);
        }
    }

    // Screenshot hooks (Bar1.TestClick and TestClose, with NYX_BAR_DEBUG).
    signal testClick(string plugin, string item, int button)
    signal testClose

    BackendClient {
        id: backend
        app: "nyx-bar"
        topics: ["bar"]
    }

    Connections {
        target: backend
        function onEvent(name, body) {
            if (name === "NodePatch") {
                for (const v of root.views[body.plugin + "\t" + body.item] || [])
                    if (!body.monitor || v.monitor === body.monitor)
                        v.patch(body.nodes);
            } else if (name === "TestClick")
                root.testClick(body.plugin, body.item, body.button);
            else if (name === "TestClose")
                root.testClose();
        }
    }

    Variants {
        model: Object.keys(root.bar.monitors || {}).filter(name => (root.pinnedScreen === "" || name === root.pinnedScreen) && Quickshell.screens.some(s => s.name === name)).sort()
        delegate: BarWindow {
            required property string modelData
            name: modelData
            client: backend
            app: root
        }
    }
}
