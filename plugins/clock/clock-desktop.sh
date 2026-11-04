#!/bin/sh
# Nyx clock as a desktop widget: an interval plugin. The host runs it every minute,
# on the minute, with its hello on stdin; it prints the `large` item for each placed
# widget, sized for it, and exits.

read -r hello
setting() { printf '%s' "$hello" | sed -n "s/.*\"$1\":\"\([^\"]*\)\".*/\1/p"; }
esc() { sed -e 's/\\/\\\\/g' -e 's/"/\\"/g' -e 's/&/\&amp;/g' -e 's/</\&lt;/g' | awk 'BEGIN { ORS = "" } { if (NR > 1) print "\\n"; print }'; }

time_format=$(setting time_format)
[ -n "$time_format" ] || time_format='%H:%M'
date_format=$(setting date_format)
[ -n "$date_format" ] || date_format='%A, %B %-d'

now=$(date +"$time_format" | esc)
day=$(date +"$date_format" | esc)
short=$(date +'%a %-d' | esc)

label() { printf '{"node":"label","text":"%s","markup":true,"font_size":%s%s}' "$1" "$2" "$3"; }
big() { label "<span weight=\\\"light\\\">$now</span>" "$1" ""; }

# Field order follows the protocol: item, monitor, size.
printf '%s' "$hello" | grep -o '"item":"large","monitor":"[^"]*","size":"[^"]*"' | while IFS= read -r w; do
  monitor=$(printf '%s' "$w" | sed -n 's/.*"monitor":"\([^"]*\)".*/\1/p')
  size=$(printf '%s' "$w" | sed -n 's/.*"size":"\([^"]*\)".*/\1/p')
  case $size in
    compact) children="$(big 44),$(label "$short" 13 ',"color":"dim_text"')" ;;
    large)
      children="$(big 92),$(label "$day" 16 ',"color":"dim_text"')"
      if command -v cal >/dev/null 2>&1; then
        month=$(cal | esc)
        children="$children,$(label "<tt>$month</tt>" 13 ',"color":"dim_text","margin":[18,0,0,0]')"
      fi
      ;;
    *) children="$(big 68),$(label "$day" 15 ',"color":"dim_text"')" ;;
  esac
  printf '{"type":"item","id":"large","monitor":"%s","view":{"node":"box","orientation":"vertical","spacing":2,"expand":true,"children":[%s]}}\n' "$monitor" "$children"
done
