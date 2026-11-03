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

## What's in it

| Plugin | Items |
| --- | --- |
| `nyx.workspaces` | workspaces on each monitor: click to go, scroll to move along |
| `nyx.clock` | time and date; click for the month |
| `nyx.weather` | Nyx Weather's conditions |
| `nyx.stats` | CPU, memory, temperature, GPU; click for graphs |
| `nyx.system` | network, Bluetooth, volume, battery; click for quick controls, right-click for the Control Center |
| `nyx.media` | what's playing, with album art and controls |
| `nyx.tray` | other apps' tray icons and menus |
| `nyx.panels` | the Notification Center bell and the Control Center button |

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
left = ["nyx.workspaces"]
center = ["nyx.clock"]
right = ["nyx.tray", "nyx.media", "nyx.weather", "nyx.stats", "nyx.system/network",
         "nyx.system/bluetooth", "nyx.system/volume", "nyx.system/battery",
         "nyx.panels/notifications", "nyx.panels/control-center"]
height = 32

[plugins."nyx.clock"]
format = "%a %b %-d  %H:%M"

[monitors."HDMI-A-1"]
right = ["nyx.clock"]
```

Which items show at all is in `~/.config/nyx/plugins.toml` (`nyx plugin configure`).
Restyle anything in `~/.config/nyx/bar.css`.

## Build

```sh
cargo build --release
cargo test --workspace
```

`nyx-bar --only HEADLESS-1` runs a separate test copy on one output, with
`NYX_BAR_DEBUG=1` for what it decides. The plan is in [docs/PLAN.md](docs/PLAN.md).

## License

MIT
