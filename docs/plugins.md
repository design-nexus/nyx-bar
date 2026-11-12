# Writing Nyx plugins

A Nyx plugin is a folder with a `plugin.toml` and a program in any language. The host
(Nyx Bar, Nyx Desktop Widgets, the Control Center) starts the program and
talks to it in JSON lines: one object per line, the host's on the program's stdin,
the program's on its stdout. Diagnostics go to stderr, which ends up in the journal.

```sh
nyx plugin new me.hello --sh      # or --py, --rust
nyx plugin check me.hello         # validate it and try it in the sandbox
nyx plugin link me.hello          # use the folder as an installed plugin
```

Plugins never need the bar rebuilt: adding, removing or editing one shows at once.

## The manifest

```toml
id = "me.weather"            # letters, digits, dots, dashes, underscores
name = "Weather"
version = "1.0.0"
author = "Me"
license = "MIT"
description = "Current conditions."
api_version = 1
kinds = ["bar"]              # also "control-center", "desktop"
exec = ["python3", "main.py"]  # a file in the folder, or a command on PATH; no shell
needs = ["python3"]          # skipped (and flagged by check) when missing

[permissions]                # see Security
network = true

[artifacts]                  # prebuilt programs, for compiled plugins
x86_64-linux = { url = "https://…/weather-1.0.0.tar.gz", sha256 = "…" }

[limits]                     # requests; capped for third-party plugins
memory = "64M"

[bar]
mode = "stream"              # or "interval" with interval = seconds
section = "right"            # left, center, right: where its items go by default
per_monitor = false          # true: send one copy of each item per monitor

[[bar.items]]                # the parts a user can turn on; without any, one part
id = "now"
name = "Current conditions"
description = "Shown in the bar's tooltip and the chooser."
default = true

[settings]                   # defaults; users override them in bar.toml
units = "metric"
```

The field names follow Glimpse's plugin manifest (`api_version`, `exec`, `needs`,
`[permissions]`).

A plugin's items belong to parts by id: item `now` is part `now`; item `forecast-2`
is part `forecast`. A plugin without `items` is one part (`main`) and may send any
items it likes (the tray sends `tray-1`, `tray-2`, …).

## Modes

- **stream**: the program keeps running, sends items when its data changes, and
  reacts to events on stdin. When it exits it's restarted (1 s, then 2 s, … up to
  60 s); five exits in ten minutes and it's turned off, with a notification.
- **interval**: the host runs it every `interval` seconds, lined up with the clock (a
  60-second plugin runs on the minute), and reads what it prints. It must finish in
  10 seconds. Nothing stays running in between. It can't receive events, so clicks use
  the items' `on_click` commands. A plain line of text (not JSON) becomes the text of
  its first part. Keep intervals at 30 seconds or more; use `stream` for faster data.

## From the host

```json
{"type":"hello","api_version":1,"kind":"bar","plugin":"me.weather","items":["now"],
 "settings":{"units":"metric"},"monitors":[{"name":"DP-1","stable_id":"…","width":2560,"height":1440,"scale":1.5,"focused":true}],
 "theme":{"light":false,"name":"nyx-dark"},"permissions":["network"],"bar_height":32}
```

`items` are the parts the user turned on. Then, any time:

| `type` | Fields | Meaning |
| --- | --- | --- |
| `monitors` | `monitors` | the monitors changed |
| `settings` | `settings` | the user changed the plugin's settings |
| `visible` | `visible`, `item?` | whether its items can be seen; pause when not |
| `desktop` | `monitors`, `workspaces`, `active_class`, `active_title` | Hyprland's state (needs `desktop`) |
| `widgets` | `widgets` | its desktop widgets were added, removed, moved or resized |
| `theme` | `theme` | the theme changed |
| `event` | see below | the user did something |
| `shutdown` | | exit now (you're killed after a second) |

An `event` has `item`, `kind` and, depending on the kind, `monitor`, `button` (1, 2, 3),
`direction` (`up`, `down`), `control` (the node id or `action` of a control, or a
menu entry's id), `value` (a toggle's state, a slider's value, an entry's text) and,
for clicks, `anchor` (`monitor`, `x`, `y`, `width`, `height`, `edge`: where the item is).

Kinds: `click`, `scroll`, `action` (buttons, rows, menu entries), `toggle`, `change`
(sliders, at most every 50 ms), `submit` (Enter in an entry), `popover_open`,
`popover_close`.

## To the host

| `type` | Fields | Meaning |
| --- | --- | --- |
| `item` | `id`, `monitor?`, `view?`, `icon?`, `text?`, `badge?`, `progress?`, `tooltip?`, `visible`, `class`, `open`, `on_click`, `on_middle_click`, `on_right_click`, `on_scroll_up`, `on_scroll_down` | show or replace an item |
| `patch` | `item`, `monitor?`, `nodes: [{id, set}]` | change properties of nodes by id |
| `popover` | `item`, `view?` | what a left click opens (no `view` removes it) |
| `menu` | `item`, `entries` | what a right click opens |
| `remove` | `item`, `monitor?` | drop an item |
| `log` | `level`, `message` | write to the journal |
| `frame_request` / `frame_ready` | see Frames | shared-memory pixels |

Clicks: the `on_*` command, if the item has one; else a left click opens the popover
if there is one, a right click the menu if there is one; else the plugin gets an
`event`. Scrolling runs `on_scroll_*` or sends a `scroll` event.

Menu entries: `id`, `label`, `icon?`, `checked?` (a check mark), `cap?` (a short code
in a key cap before the label, "EN"), `enabled` (a disabled entry without an `id` is a
dim line, a hint), `separator`, `children` (a submenu, opened in place under its entry).
An `icon` can list fallbacks as in views.

`open: true` says the item has something open that isn't its popover. The item gets
the class `open`, as while its popover shows, for your CSS; the bar itself tints
nothing for it, nor the pill it shares. The bar's height is in `hello` (`bar_height`): 28, 32, 36 or 40 for
the size presets.

## Desktop widgets

A `desktop` part is a widget on the desktop, behind windows. Its manifest entry says
which sizes it comes in (the first is its default) and whether it takes clicks:

```toml
[desktop]
mode = "stream"

[[desktop.items]]
id = "card"
name = "Forecast"
sizes = ["medium", "large", "compact"]   # 336×160, 336×336, 160×160 logical px
interactive = false                      # true: clicks reach it; else they pass through
resizable = false                        # true: also any size in edit mode ("free")
min = [160, 120]                         # free sizes' range, logical px
max = [672, 504]
default = true
```

`hello` (and a `widgets` message whenever they change) lists where the user put them:

```json
"widgets":[{"item":"card","monitor":"DP-1","size":"medium","width":336,"height":160,"options":{"units":"imperial"}}]
```

Send one item per part, or one per monitor (with `monitor`) when the sizes differ.
A free size comes as `"size":"free"` with its `width` and `height`. Named sizes grow
with the suite's text scale, as `font_size` does everywhere. On interactive widgets a
click opens the item's popover and a right click its menu, as in the bar. While the
**Add widget** list is open, the plugin also gets a widget on monitor `preview`, which
it draws like any other.
Views are kept to the widget's size, inside the card's 16 px padding. While windows
cover every copy of a widget, the plugin gets `visible: false` for that item (and for
the plugin when nothing of it can be seen); pause then. An interval plugin is run
again at once when its widgets change.

## Control Center tiles

A `control-center` part is a tile on the Control Center's dashboard, beside Wi-Fi and
the rest:

```toml
[control-center]
mode = "stream"

[[control-center.items]]
id = "ping"
name = "Ping"
sizes = ["2x1", "1x1"]                   # grid cells, up to 4x3; the first is the default
icon = "network-transmit-receive-symbolic"
swatch = "teal"                          # the tile's colour when on (blue, violet, pink,
                                         # red, coral, amber, green, teal)
detail = true                            # it has a detail view
default = true
```

- An item **without a view** is drawn as a native toggle: `icon` in the chip, the
  part's name, `text` as the status line, and the chip lit when `class` has `on`. A
  click runs `on_click`, else sends a `toggle` event. With `detail = true` and a
  popover, it's a split tile whose arrow opens the detail view.
- An item **with a view** fills the tile (keep to the sizes in `widgets`).
- The item's **popover is its detail view**, shown in the panel under a back button;
  `popover_open` and `popover_close` say when it's showing.
- The plugin starts the first time its tile shows, hears `visible` as the panel opens
  and closes, and is stopped after two minutes hidden (trusted plugins with
  `keep_alive = true` aren't). Its last items show at once on the next open.

`examples/cc-ping` is a complete third-party tile in Python.

## Views

An item without a `view` is a row of its `icon`, `text`, `badge` and `progress`. A
`view` is a tree of nodes, each `{"node": KIND, …}` with these on any node:

`id` · `class` (list) · `width` · `height` · `margin` `[top, right, bottom, left]` ·
`font_size` · `color` · `background` · `tooltip` · `visible` · `expand` · `hover` and
`pressed` (property sets the host applies while the pointer is over the item or a button
is held, and puts back after) · `action` (any node: a click on it sends an `action`
event with this as `control`, as a button does; the pointer becomes a hand).

An `icon` name can list fallbacks, `"a-symbolic|b-symbolic|c"`: the first the icon
theme has is drawn. In the bar a label keeps its full width unless it has a `width`
or the class `shrink`; then it ends in "…" when the bar runs out of room (window and
song titles on a narrow screen). A popover's labels start at their left; with the class
`center` the text centres in a label wider than it. A square `image` without a `radius`
draws at its size, as an icon does; a drawing (`graph`, `vector`) keeps the height it
asks for in a row.

The bar's style has classes for views: `dim`, `tertiary`, `strong`, `small`, `xs`,
`big`, `huge`, `caps`, `tnum` (figures of one width), `chip` (with `warning` or
`danger`), `kbd` (a key cap), `card` and `selected` (popovers).

| `node` | Fields |
| --- | --- |
| `box` | `orientation` (`horizontal`, `vertical`), `spacing`, `align` (`center`, `start`, `end`, `fill`; `center` when unset, so a vertical box needs `fill` to stretch its children across), `children` |
| `overlay` | `children` (the first at the bottom) |
| `stack` | `show` (a child's id), `transition` (`crossfade`, `slide_up`, …), `children` |
| `label` | `text`, `markup` (`true`: `text` is Pango markup, `<b>`, `<span size="small">`, …) |
| `icon` | `name` (an icon-theme name, or a file), `size` |
| `image` | `path` or `data` (base64 PNG/JPEG), `radius` |
| `badge` | `text` |
| `progress` | `value` (0–1) |
| `graph` | `values`, `max`, `style` (`line`, `bars`); `color` tints it |
| `spinner` | |
| `vector` | `svg`, or `commands` and `view_box` (see below) |
| `frame` | (pixels from shared memory; give it an `id`, `width` and `height`) |
| `row` | `icon`, `title`, `subtitle`, `action` (popovers) |
| `toggle` | `label`, `active` |
| `slider` | `min`, `max`, `step`, `value`, `icon` |
| `button` | `label`, `icon`, `action` |
| `entry` | `text`, `placeholder` |
| `separator` | |
| `scroll` | `max_height`, `child` (not inside another scroll: popovers already scroll, and a scroll in a scroll gets almost no height) |

Colours are theme tokens: `bg`, `surface`, `text`, `dim_text`, `accent`, `on_accent`,
`border`, `muted`, `highlight`, `danger`, `panel`, `card`, `raised`, `popover`,
`text_tertiary`, `divider`, `warning`, `success`, `hover`, `field`, `track`,
`hairline`. They follow the user's theme. `#rrggbb[aa]` works, but stays the same in
every theme.

Classes the bar styles: `accent`, `success`, `warning`, `danger`, `dim`, `active`,
`urgent`.

Fields a node doesn't have are ignored without a warning, so check a view by looking at
it.

In popovers, labels wrap and end in "…" when they don't fit. A short label beside a row
or label with a lot of text (one that `expand`s) can be squeezed down to its longest
word and cut off, and `width` doesn't stop that: keep its words together with
non-breaking spaces (U+00A0), or keep the long text shorter.

Inline sizes are capped at the bar's height. Images and files can only come from the
plugin's folder (`/plugin/…`) or its data folder (`/storage/…`) for third-party
plugins.

### Patches

Send the item once with ids on the nodes that change, then patch them:

```json
{"type":"patch","item":"cpu","nodes":[{"id":"pct","set":{"text":"42%"}},{"id":"g","set":{"values":[3,5,8,42]}}]}
```

Properties: `text`, `markup`, `icon`/`name`, `title`, `subtitle`, `label`, `active`,
`value`, `values`, `max`, `svg`, `commands`, `show`, `path`, `data`, `spinning`,
`visible`, `tooltip`, `class`, `color`, `background`, `font_size`, `radius`.

### Vector drawings

`commands` draw in `view_box` units (`[x, y, width, height]`, scaled to the node's
size):

| `op` | Fields |
| --- | --- |
| `path` | `d` (SVG path data: `M L H V C S Q T Z`, absolute and relative), `fill`, `stroke`, `width` |
| `line` | `x1 y1 x2 y2`, `stroke`, `width`, `round` |
| `arc` | `cx cy r start end` (degrees clockwise from 3 o'clock), `fill`, `stroke`, `width`, `round` |
| `circle` | `cx cy r`, `fill`, `stroke`, `width` |
| `rect` | `x y w h radius`, `fill`, `stroke`, `width` |
| `text` | `x y text size`, `fill`, `align` (`start`, `center`, `end`), `bold` |
| `save`, `restore`, `translate {x y}`, `rotate {deg}`, `scale {x y}` | |

`fill` and `stroke` are colours, or gradients:
`{"linear":{"x1":0,"y1":0,"x2":24,"y2":0,"stops":[[0,"accent"],[1,"success"]]}}`,
`{"radial":{"cx":12,"cy":12,"r":12,"stops":[…]}}`. An `svg` document is drawn with
librsvg instead.

### Plugin CSS

A `style.css` in the folder styles the plugin's own widgets (`&` or `:root` is a view's
root, `&.wide` the root with a class): every selector gets
`.plugin-<id>` in front (dots become underscores), `@keyframes` are renamed for the
plugin, other at-rules and `url()` are dropped. Theme colours are available as
`@theme_accent`, `@theme_card`, `@row_hover` and the rest. Animations pause while the
bar is hidden. The user's `~/.config/nyx/bar.css` loads after every plugin's.

```css
.dot { animation: pulse 1.2s ease-in-out infinite; }
@keyframes pulse { from { opacity: 1; } to { opacity: 0.35; } }
```

NyxOS's Quickshell bar reads the same file but draws less: padding, margins, minimum
sizes, backgrounds, corners, borders, inset shadows that are a ring
(`inset 0 0 0 1px …`) or a line on one side (`inset 0 -2px 0 …`), opacity, text colour,
size, weight and spacing, and `-gtk-icon-size`. Colours can be theme names, `#rgb`,
`#rrggbb`, `rgb()`/`rgba()`, `alpha()`, `mix()` and `shade()`. Selectors are classes,
node types, `:hover`, `:not(.x)` and `&`; animations don't run there, so don't rely on
one to show state. Icons without a size of their own follow the bar's size (14 px
compact, 16 default and cozy, 18 spacious).

### Frames

For pixels you draw yourself (needs `frames`): put a `frame` node with an `id` in a
view, connect to `hello.frame_socket`, send
`{"type":"frame_request","item":…,"node":…,"width":W,"height":H}` (pixels: logical
size × the monitor's scale). The host sends two `memfd` buffers over the socket
(SCM_RIGHTS) with a JSON line `{"stride":…,"buffers":2}`. Draw BGRA premultiplied into
one, send `{"type":"frame_ready","item":…,"node":…,"buffer":0}`, draw the next into the
other. The Rust SDK's `FrameSurface` does all of this.

### Your own windows

Click events carry an `anchor`. A plugin with the `windows` permission can open its own
layer-shell window under its item (Rust: `nyx_plugin_sdk::popup::popup_at`, `gtk`
feature) and should implement `io.github.design_nexus.nyx.Panel1` so it closes when the
Control Center or Notification Center opens, and they close when it does.

## Security

Plugins that come with a package or Nyx itself are trusted. Plugins in
`~/.local/share/nyx/plugins` (added, linked or written by you) are third-party:

- They run in bubblewrap: the system's programs and libraries read-only, the plugin's
  folder at `/plugin` (read-only), its data at `/storage` (`$HOME` and
  `$NYX_PLUGIN_STORAGE`), a private `/tmp`, no home folder, no Hyprland sockets, no
  capabilities, a clean environment. Without bubblewrap they don't run at all.
- They get only what `[permissions]` asks for, and only once the user approved it
  (`nyx plugin add` asks). An update that asks for more stays off until it's approved
  again. A plugin that can change the computer through the system bus (network
  settings, power, disks, or a service Nyx doesn't know) gets a separate warning naming
  each change, a prompt that defaults to no, and with `--yes` also needs
  `--allow-system`.

| Permission | Gives |
| --- | --- |
| `network = true` | the network |
| `read = ["~/Music"]` | read-only paths |
| `session_talk = ["org.mpris.MediaPlayer2.*"]` | D-Bus names on the session bus, through a filtering proxy |
| `system_talk = ["org.freedesktop.UPower"]` | the same on the system bus |
| `host_commands = [["nyx-weather", "--toggle"]]` | exact command lines its items' `on_*` may run; anything else is refused |
| `desktop = true` | `desktop` messages (workspaces, the focused window's title) |
| `windows = true` | the Wayland socket, for its own windows |
| `frames = true` | shared-memory frames |

Installing never runs a plugin's code: no scripts, no builds. Compiled plugins ship
`[artifacts]` (`nyx plugin pack` makes them), whose hash is checked, and which the
index pins too.

## Limits

Every plugin runs in its own systemd scope. Third-party defaults, which the user can
change per plugin in `bar.toml` (`[plugins."<id>".limits]`):

| | |
| --- | --- |
| Memory | 128 MB |
| CPU | 10% of one core |
| Processes and threads | 32 |
| Priority | nice 10, IO weight 50 |
| Messages | 30 a second; extras are merged, newest wins; a plugin far over it waits to write |
| Message size | 256 KB, or 2 MB with an image |
| Drawings | 5,000 commands or SVG elements |
| Frames | 30 fps (in every host), 400 × 64 logical pixels in the bar |
| Popovers | 480 × 640 |
| Items | 16 per monitor |
| Interval runs | every 5 s at most, 10 s each |
| Infinite CSS animations | 4 |

## The index

`nyx plugin search` and `nyx plugin add NAME` use
[nyxos-project/nyx-plugin-index](https://github.com/nyxos-project/nyx-plugin-index):
each entry pins a plugin's repo to a commit (and its artifacts to their hashes). Add
yours with a pull request there. Plugins outside the index install from their URL.

## SDKs

- Rust: `nyx-plugin-sdk`, in [nyx-core](https://github.com/design-nexus/nyx-core), as a git
  dependency:

  ```toml
  nyx-plugin-sdk = { git = "https://github.com/design-nexus/nyx-core.git", rev = "…" }
  ```

  `Plugin::start()` gives the
  hello and one stream of inputs (events, settings, visibility, your own data); `view`
  has builders for every node; the `network`, `bluetooth`, `audio`, `power` and `mpris`
  features push live system data; `frames` and `popup` cover the rest. The first-party
  plugins in `plugins/` use it.
- Python: `sdk/python/nyx_plugin.py`, standard library only. `examples/hello-py`.
- Anything else: print JSON lines. `examples/hello` is 20 lines of shell.
