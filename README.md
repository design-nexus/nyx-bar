# Nyx Bar

A full-width top bar for Hyprland, on every monitor, following the Nyx theme. It hides
for fullscreen windows. The bar itself only hosts: every item in it, the workspaces and
the clock included, is a plugin, and plugins never need the bar rebuilt.

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/design-nexus/nyx-bar/main/install.sh | bash
```

or from a checkout, `./install.sh`. It installs `nyx-bar`, `nyx-plugin` and the
first-party plugins (to `~/.local/lib/nyx/plugins`), and a user service that isn't
turned on. Try the bar with `nyx-bar`; run it at login with
`systemctl --user enable --now nyx-bar`. On NyxOS it comes with the system.

It needs GTK 4, gtk4-layer-shell, librsvg, bubblewrap and xdg-dbus-proxy.

The bar has two interfaces with the same plugins, layout and look: GTK (`nyx-bar`), and
Quickshell (`nyx-bar-backend`, which runs the plugins, with the bar's QML in `qml/Bar`
drawn by Nyx Shell's `nyx-shell-qs`). NyxOS runs the Quickshell one. Elsewhere,
`nyx-bar --ui qml` switches to it where Nyx Shell is installed, `nyx-bar --ui gtk`
switches back, and `nyx-bar --ui` says which is on.

## What's in it

| Plugin | Items |
| --- | --- |
| `nyx.launcher` | the Nyx mark: click for Glimpse (apps and search), right-click for Settings |
| `nyx.workspaces` | workspaces on each monitor as pills, numbers, icons, dots or an underline: click to go, scroll to move along, hover for the windows |
| `nyx.window` | the focused window's app and title |
| `nyx.clock` | date and time with the next event; click for the month, the day's agenda and reminders |
| `nyx.media` | what's playing; click for the player, seek and its volume |
| `nyx.weather` | Nyx Weather's conditions; click for the next hours, five days and alerts |
| `nyx.stats` | CPU, memory and temperature as sparklines, text or rings; click for graphs and the busiest apps |
| `nyx.privacy` | dots while the microphone, camera or screen sharing is in use |
| `nyx.recording` | a timer and stop button while Nyx Capture records the screen |
| `nyx.updates` | waiting package updates (Arch) |
| `nyx.keyboard` | the keyboard layout, when there's more than one |
| `nyx.keepawake` | keep the computer awake (off by default) |
| `nyx.tray` | other apps' tray icons and menus, the rest in a drawer |
| `nyx.system` | network, Bluetooth, volume, battery: each opens its Control Center page (right-click: the dashboard) |
| `nyx.panels` | the Notification Center bell (right-click: Do not disturb, clear), and the Control Center button |

## Plugins

`nyx plugin` (also `nyx-plugin`):

```sh
nyx plugin list                    # what's installed, what's on, what it uses
nyx plugin search weather          # the Nyx plugin index
nyx plugin add ping                # from the index, or a git URL
nyx plugin configure nyx.system    # which parts, and where
nyx plugin disable nyx.stats
nyx plugin update
```

Third-party plugins run in a sandbox, with the permissions you approve, under memory
and CPU limits. Writing one: [docs/plugins.md](docs/plugins.md).

## Layout

`~/.config/nyx/bar.toml` (written on first start, reloaded live):

```toml
left = [["nyx.launcher", "nyx.workspaces"], "nyx.window"]
center = [["nyx.clock"]]
right = ["nyx.privacy", "nyx.recording", "nyx.keyboard", ["nyx.media"], "*",
         ["nyx.weather", "nyx.stats"], "nyx.tray", "nyx.updates", "nyx.keepawake",
         ["nyx.system"], ["nyx.panels/notifications"], ["nyx.panels/control-center"]]
size = "default"

[plugins."nyx.clock"]
format = "%a %b %-d  %H:%M"

[monitors."HDMI-A-1"]
right = ["nyx.clock"]
```

- Entries in brackets share one pill (`["nyx.weather", "nyx.stats"]`); the pill lights
  while one of them is open. `"|"` draws a thin divider. `"*"` is where enabled items
  the file doesn't place go (another plugin's, AI Hub's).
- The file is written on first start as the layout above, commented out, so NyxOS's
  layout applies until you change it. The very first bar's file, never edited, is
  replaced by that template on start (kept as `bar.toml.old`).
- `size` is `compact` (28 px), `default` (32), `cozy` (36) or `spacious` (40); an exact
  `height` wins over it. Settings → Nyx Bar sets the size, the status pill, the
  calendar's layout and the workspaces' look.

Which items show at all is in `~/.config/nyx/plugins.toml` (`nyx plugin configure`).
Restyle anything in `~/.config/nyx/bar.css` (the GTK interface; the Quickshell one
doesn't read it yet).

## Build

```sh
cargo build --release
cargo test --workspace
```

`nyx-bar --only HEADLESS-1` runs a separate test copy on one output, with
`NYX_BAR_DEBUG=1` for what it decides; `NYX_BAR_ONLY=HEADLESS-1 nyx-bar-backend` does the
same for the Quickshell interface, with `NYX_SHELL_SCREEN=HEADLESS-1` for `nyx-shell-qs`. The plan is in [docs/PLAN.md](docs/PLAN.md).

## License

MIT
