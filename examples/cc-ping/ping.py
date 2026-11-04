"""Ping: latency to a host as a Control Center tile.

The tile is a native toggle (no view of its own): its icon, its status line, and "on"
while the host answers. Clicking it pauses. The popover is its detail view: recent
pings as a graph and a list. It only pings while the Control Center is open.
"""

import socket
import time

import nyx_plugin as nyx

plugin = nyx.Plugin()
history = []  # (time, milliseconds or None)
paused = False


def target():
    return plugin.settings.get("host", "1.1.1.1"), int(plugin.settings.get("port", 443))


def ping():
    # A TCP connect: no raw sockets needed, so it works in the sandbox with `network`.
    host, port = target()
    start = time.monotonic()
    try:
        socket.create_connection((host, port), timeout=2).close()
    except OSError:
        return None
    return (time.monotonic() - start) * 1000


def show():
    host, port = target()
    last = history[-1][1] if history else None
    if paused:
        status, on = "Paused", False
    elif not history:
        status, on = "Pinging…", False
    elif last is None:
        status, on = "No answer", False
    else:
        status, on = f"{last:.0f} ms", True
    plugin.item(
        "ping",
        icon="network-transmit-receive-symbolic",
        text=status,
        tooltip=f"Latency to {host}" + (" (paused)" if paused else ""),
        class_=["on"] if on else [],
    )
    rows = [
        nyx.row(f"{ms:.0f} ms" if ms is not None else "No answer", subtitle=when,
                icon="emblem-ok-symbolic" if ms is not None else "dialog-warning-symbolic")
        for when, ms in reversed(history[-12:])
    ]
    values = [ms or 0 for _, ms in history[-60:]]
    plugin.popover("ping", nyx.vbox([
        nyx.label(f"TCP connect to {host}:{port}, every 5 seconds", color="dim_text"),
        nyx.graph(values, width=340, height=56, color="accent"),
        *rows,
    ], spacing=6, align="fill"))


show()
for event in plugin.events(timeout=5):
    if event is None:
        if plugin.visible and not paused:
            history.append((time.strftime("%H:%M:%S"), ping()))
            del history[:-60]
            show()
        continue
    if event["kind"] in ("toggle", "click"):
        paused = not paused
        show()
