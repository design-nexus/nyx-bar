#!/bin/sh
# Nyx weather as a Control Center tile: every ten minutes, the current conditions as
# a native tile (its icon, and the temperature and sky as its status). A click opens
# Nyx Weather.

read -r _hello
out=$(nyx-weather --waybar 2>/dev/null) || exit 0
[ -n "$out" ] || exit 0

# Its JSON strings are already escaped; take them as they are.
field() { printf '%s' "$out" | sed -n -E "s/.*\"$1\":\"(([^\"\\\\]|\\\\.)*)\".*/\\1/p"; }
text=$(field text)
alt=$(field alt)
tooltip=$(field tooltip)
[ -n "$text" ] || exit 0

first=$(printf '%s' "$tooltip" | awk 'BEGIN { RS = "\\\\n" } NR == 1 { print; exit }')
condition=${first##* — }

case $alt in
  *thunder* | *storm*) icon=weather-storm-symbolic ;;
  *snow* | *sleet*) icon=weather-snow-symbolic ;;
  *rain* | *drizzle* | *shower*) icon=weather-showers-symbolic ;;
  *fog* | *mist* | *haze*) icon=weather-fog-symbolic ;;
  *overcast* | *cloudy*) icon=weather-overcast-symbolic ;;
  *partly* | *few*) icon=weather-few-clouds-symbolic ;;
  *night*) icon=weather-clear-night-symbolic ;;
  *) icon=weather-clear-symbolic ;;
esac

printf '{"type":"item","id":"tile","icon":"%s","text":"%s · %s","tooltip":"%s","on_click":["nyx-weather","--toggle"]}\n' \
  "$icon" "${text#* }" "$condition" "$tooltip"
