import json, os, subprocess, sys, socket
sys.stdin.readline()
home = "/home/" + os.environ.get("USER", "ken")
r = {}
def attempt(name, f):
    try:
        f(); r[name] = "allowed"
    except Exception as e:
        r[name] = "refused"
attempt("home", lambda: os.listdir("/home/ken/.ssh"))
attempt("network", lambda: socket.create_connection(("1.1.1.1", 443), timeout=2))
attempt("session-bus", lambda: socket.socket(socket.AF_UNIX).connect(os.environ.get("XDG_RUNTIME_DIR", "/run/user/1000") + "/bus"))
attempt("system-bus", lambda: socket.socket(socket.AF_UNIX).connect("/run/dbus/system_bus_socket"))
attempt("hyprland", lambda: os.listdir(os.environ.get("XDG_RUNTIME_DIR", "/run/user/1000") + "/hypr"))
r["wayland"] = "allowed" if os.environ.get("WAYLAND_DISPLAY") else "refused"
attempt("write-plugin", lambda: open("/plugin/x", "w"))
attempt("write-storage", lambda: open("/storage/x", "w").close())
sys.stderr.write("spy: " + json.dumps(r) + "\n")
print(json.dumps({"type": "item", "id": "main", "text": "spy", "tooltip": json.dumps(r),
                  "on_click": ["sh", "-c", "touch /tmp/nyxbar-pwned"], "on_right_click": ["true"]}), flush=True)
for line in sys.stdin:
    pass
