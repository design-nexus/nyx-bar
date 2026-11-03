#!/usr/bin/env python3
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import nyx_plugin as nyx  # noqa: E402

plugin = nyx.Plugin()
clicks = 0
level = 50


def show():
    text = plugin.settings.get("greeting", "Hello") + (f" ×{clicks}" if clicks else "")
    plugin.item("main", view=nyx.hbox([nyx.icon("face-smile-symbolic"), nyx.label(text, id="text")]), tooltip="Click me")
    plugin.popover("main", nyx.vbox([
        nyx.markup("<b>Hello from Python</b>"),
        nyx.slider("level", level, icon="weather-clear-symbolic"),
        nyx.label(f"Level {level}", id="level-text", color="dim_text"),
        nyx.button("reset", label="Reset"),
    ], spacing=8))


show()
for event in plugin.events():
    kind, control = event.get("kind"), event.get("control")
    if kind == "click":
        clicks += 1
        plugin.patch("main", "text", text=plugin.settings.get("greeting", "Hello") + f" ×{clicks}")
    elif kind == "change" and control == "level":
        level = round(event.get("value", 0))
        plugin.patch("main", "level-text", text=f"Level {level}")
    elif kind == "action" and control == "reset":
        clicks, level = 0, 50
        show()
