import json
import math
import mmap
import socket
import sys
import threading
import time

hello = json.loads(sys.stdin.readline())
visible = True


def send(m):
    sys.stdout.write(json.dumps(m) + "\n")
    sys.stdout.flush()


send({"type": "item", "id": "layout", "view": {
    "node": "box", "orientation": "vertical", "spacing": 0, "children": [
        {"node": "label", "text": "<b>Two</b>", "markup": True, "font_size": 11, "id": "top"},
        {"node": "label", "text": "lines", "font_size": 9, "color": "dim_text"},
    ],
    "hover": {"background": "accent"},
}})
send({"type": "item", "id": "pulse", "view": {"node": "label", "text": "●", "class": ["dot"]}})
ring = lambda pct: {"node": "vector", "width": 20, "height": 20, "view_box": [0, 0, 24, 24], "commands": [
    {"op": "circle", "cx": 12, "cy": 12, "r": 9, "stroke": "track", "width": 3},
    {"op": "arc", "cx": 12, "cy": 12, "r": 9, "start": -90, "end": -90 + 360 * pct, "stroke": "accent", "width": 3, "round": True},
], "id": "ring"}
send({"type": "item", "id": "ring", "view": ring(0.7)})
send({"type": "item", "id": "viz", "view": {"node": "frame", "id": "f", "width": 120, "height": 20}})

W, H = 120, 20
sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
sock.connect(hello["frame_socket"])
time.sleep(0.05)
send({"type": "frame_request", "item": "viz", "node": "f", "width": W, "height": H})
msg, fds, _, _ = socket.recv_fds(sock, 512, 4)
head = json.loads(msg.decode().strip())
stride = head["stride"]
maps = [mmap.mmap(fd, stride * H) for fd in fds]
sys.stderr.write(f"frames: {head}\n")


def events():
    global visible
    for line in sys.stdin:
        m = json.loads(line)
        if m.get("type") == "visible" and not m.get("item"):
            visible = m["visible"]
        if m.get("type") == "shutdown":
            sys.exit(0)


threading.Thread(target=events, daemon=True).start()
buf, t, n = 0, 0.0, 0
while True:
    if visible:
        px = bytearray(stride * H)
        for x in range(W):
            h = int((math.sin(x / 9 + t) * 0.5 + 0.5) * (H - 2)) + 1
            for y in range(H - h, H):
                i = y * stride + x * 4
                px[i:i + 4] = bytes([0xf0, 0x9a, 0x5c, 0xff])
        maps[buf].seek(0)
        maps[buf].write(bytes(px))
        send({"type": "frame_ready", "item": "viz", "node": "f", "buffer": buf})
        buf, t, n = 1 - buf, t + 0.25, n + 1
        if n % 30 == 0:
            send({"type": "patch", "item": "ring", "nodes": [{"id": "ring", "set": {"commands": ring((n // 30 % 10) / 10)["commands"]}}]})
    time.sleep(1 / 30)
