# Bar audit (November 2026)

What a pass over both bars (the GTK `nyx-bar` and NyxOS's Quickshell bar,
`nyx-bar-backend` with `qml/Bar`) fixed, and what it left for later.

## Fixed

- **Gaps:** pills and dividers follow which items are shown, not which widgets exist;
  a divider shows only between shown items. Settings lists only plugins that run, with
  each item's description.
- **Flicker:** an item's tree is rebuilt only when its view changes (not its tooltip,
  classes or open state), and QML rebuilds a view only when its shape changes. The
  bar's state keys monitors by name, so updates stay small.
- **Control Center** lights nothing in the bar while it's open.
- **Tooltips** never take the pointer, wrap at 360 px, open below the bar, and only one
  shows at a time. Clicks reach buttons, toggles and sliders inside items.
- **Popovers** close when the plugin withdraws them, tell the plugin when they're
  destroyed, and a click on their item right after closing doesn't reopen them.
- **Patches never reached the Quickshell bar:** its `Patch` event was the UI link's own
  state-patch event, so the link's client swallowed it. It's `NodePatch` now (in
  nyx-desktop-widgets too), and nodes register their id when they get their slot.
- **AI Hub** and other unplaced plugins each get a pill; AI Hub's icon is its symbolic
  one. Icons follow the bar's size. The status pill has 13 px between icons.
- **Clock:** date style, 12/24-hour (automatic by region), seconds, and a format of your
  own that can't crash the plugin; one time format in the bar, tooltip, agenda,
  upcoming days and reminders.
- **CSS for QML:** rules after `@keyframes` survive; `#rgb`, `rgb()`, `rgba()`, named
  colours and nested functions are read; one-sided borders and one-sided inset
  shadows draw; `@nav_active`, `@row_hover`, `@lift` and the rest resolve.
- **Backend:** a plugin that stops or removes an item takes its menus, popovers and
  revisions with it; a patch for one monitor doesn't change another's popover; patches
  to tray-style pictures resend the item so they're tinted.

## Left

- **Two hosts, one behaviour:** the GTK bar (`apps/bar/src/bar.rs`) and the QML bar
  (`qml/Bar/BarWindow.qml`) each apply the layout; divider and pill visibility is
  ported to JavaScript from `nyx_bar_core::shown_dividers`. A change to one needs the
  other. The GTK bar can go once NyxOS no longer needs it.
- **CSS subset:** the QML bar ignores animations, blurred or offset shadows,
  `:first-child`, `>` as a real child combinator and transitions. Plugins that show
  state only through an animation look static there (AI Hub's working dot was one).
- **Desktop clock:** the desktop widget (`clock-desktop.sh`) keeps its own
  `time_format` and `date_format` and doesn't follow `hours`.
- **Shared popovers and per-monitor patches:** a patch for one monitor isn't kept in a
  popover shared by every monitor; the plugin's next popover brings it up to date.
- **Popover reopen guard** is a 200 ms window after closing, a heuristic for the click
  that closes it landing on its item.
- **Unexplained once:** in one test run the backend was gone after starting its
  plugins, with nothing in its log. It didn't happen again in later runs.
