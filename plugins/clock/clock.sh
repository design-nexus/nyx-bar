#!/bin/sh
# Nyx Bar clock: an interval plugin. The bar runs it every minute, on the minute, with
# its hello on stdin; it prints the item and the popover and exits.

read -r hello
format=$(printf '%s' "$hello" | sed -n 's/.*"format":"\([^"]*\)".*/\1/p')
[ -n "$format" ] || format='%a %b %-d  %H:%M'

esc() { sed -e 's/\\/\\\\/g' -e 's/"/\\"/g' -e 's/&/\&amp;/g' -e 's/</\&lt;/g' | awk 'BEGIN { ORS = "" } { if (NR > 1) print "\\n"; print }'; }

now=$(date +"$format" | esc)
long=$(date +'%A, %B %-d, %Y' | esc)

printf '{"type":"item","id":"time","text":"%s","tooltip":"%s"}\n' "$now" "$long"

if command -v cal >/dev/null 2>&1; then
  month=$(cal | esc)
  printf '{"type":"popover","item":"time","view":{"node":"box","orientation":"vertical","spacing":8,"children":[{"node":"label","text":"<b>%s</b>","markup":true},{"node":"label","text":"<tt>%s</tt>","markup":true,"color":"dim_text"}]}}\n' "$long" "$month"
else
  printf '{"type":"popover","item":"time","view":{"node":"label","text":"<b>%s</b>","markup":true}}\n' "$long"
fi
