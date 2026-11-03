#!/bin/sh
# Nyx Bar weather: every ten minutes, Nyx Weather's own bar summary as an item.

read -r _hello
out=$(nyx-weather --waybar 2>/dev/null) || exit 0
[ -n "$out" ] || exit 0

# Its JSON strings are already escaped; take them as they are.
field() { printf '%s' "$out" | sed -n -E "s/.*\"$1\":\"(([^\"\\\\]|\\\\.)*)\".*/\\1/p"; }
text=$(field text)
tooltip=$(field tooltip)
[ -n "$text" ] || exit 0

printf '{"type":"item","id":"now","text":"%s","tooltip":"%s","on_click":["nyx-weather","--toggle"],"on_right_click":["nyx-weather","--section","settings"]}\n' "$text" "$tooltip"
