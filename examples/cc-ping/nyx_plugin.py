"""Write Nyx plugins in Python, with nothing but the standard library.

    import nyx_plugin as nyx

    plugin = nyx.Plugin()                     # reads the host's hello
    plugin.item("main", view=nyx.label("Hi"))
    for event in plugin.events():             # clicks, scrolls, controls…
        if event["kind"] == "click":
            plugin.patch("main", "text", text="Clicked")

Copy this file next to your plugin. The protocol is in docs/plugins.md.
"""

import json
import queue
import sys
import threading

API_VERSION = 1


class Plugin:
    def __init__(self):
        line = sys.stdin.readline()
        if not line:
            sys.exit(0)
        self.hello = json.loads(line)
        self.settings = self.hello.get("settings") or {}
        self.parts = self.hello.get("items") or []
        self.visible = True
        self._queue = queue.Queue()
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self):
        for line in sys.stdin:
            try:
                msg = json.loads(line)
            except ValueError:
                continue
            self._queue.put(msg)
        self._queue.put({"type": "shutdown"})

    def enabled(self, part):
        return part in self.parts or "main" in self.parts

    def send(self, msg):
        try:
            sys.stdout.write(json.dumps(msg, separators=(",", ":")) + "\n")
            sys.stdout.flush()
        except BrokenPipeError:
            sys.exit(0)

    def item(self, id, view=None, monitor=None, **fields):
        """fields: icon, text, badge, progress, tooltip, visible, class_ (a list),
        on_click, on_right_click, on_middle_click, on_scroll_up, on_scroll_down."""
        msg = {"type": "item", "id": id}
        if view is not None:
            msg["view"] = view
        if monitor:
            msg["monitor"] = monitor
        if "class_" in fields:
            fields["class"] = fields.pop("class_")
        msg.update({k: v for k, v in fields.items() if v is not None})
        self.send(msg)

    def patch(self, item, node, **props):
        self.send({"type": "patch", "item": item, "nodes": [{"id": node, "set": props}]})

    def popover(self, item, view):
        self.send({"type": "popover", "item": item, "view": view})

    def menu(self, item, entries):
        self.send({"type": "menu", "item": item, "entries": entries})

    def remove(self, item):
        self.send({"type": "remove", "item": item})

    def log(self, message, level="info"):
        self.send({"type": "log", "level": level, "message": message})

    def events(self, timeout=None):
        """Events from the host. With a timeout, yields None when it passes, so a plugin
        can also update on a timer. Settings and visibility are kept on the plugin."""
        while True:
            try:
                msg = self._queue.get(timeout=timeout)
            except queue.Empty:
                yield None
                continue
            kind = msg.get("type")
            if kind == "shutdown":
                return
            if kind == "settings":
                self.settings = msg.get("settings") or {}
            elif kind == "visible" and not msg.get("item"):
                self.visible = bool(msg.get("visible", True))
            elif kind == "event":
                yield msg


def _node(kind, **fields):
    n = {"node": kind}
    n.update({k: v for k, v in fields.items() if v is not None})
    return n


def hbox(children, spacing=4, **f):
    return _node("box", orientation="horizontal", spacing=spacing, children=list(children), **f)


def vbox(children, spacing=4, **f):
    return _node("box", orientation="vertical", spacing=spacing, children=list(children), **f)


def label(text, **f):
    return _node("label", text=text, **f)


def markup(text, **f):
    return _node("label", text=text, markup=True, **f)


def icon(name, size=None, **f):
    return _node("icon", name=name, size=size, **f)


def image(path=None, data=None, **f):
    return _node("image", path=path, data=data, **f)


def badge(text, **f):
    return _node("badge", text=text, **f)


def progress(value, **f):
    return _node("progress", value=value, **f)


def graph(values, max=None, style="line", **f):
    return _node("graph", values=list(values), max=max, style=style, **f)


def vector(commands, view_box=None, **f):
    return _node("vector", commands=list(commands), view_box=view_box, **f)


def svg(document, **f):
    return _node("vector", svg=document, **f)


def row(title, icon=None, subtitle=None, action=None, **f):
    return _node("row", title=title, icon=icon, subtitle=subtitle, action=action, **f)


def toggle(id, label=None, active=False, **f):
    return _node("toggle", id=id, label=label, active=active, **f)


def slider(id, value, min=0, max=100, step=1, icon=None, **f):
    return _node("slider", id=id, value=value, min=min, max=max, step=step, icon=icon, **f)


def button(action, label=None, icon=None, **f):
    return _node("button", action=action, label=label, icon=icon, **f)


def separator():
    return _node("separator")
