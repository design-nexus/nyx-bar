#!/bin/sh
# Nyx weather as a desktop widget: every ten minutes, Nyx Weather's current conditions
# as a card for each placed widget. A click opens Nyx Weather.

read -r hello
out=$(nyx-weather --waybar 2>/dev/null) || exit 0
[ -n "$out" ] || exit 0

# Its JSON strings are already escaped; take them as they are.
field() { printf '%s' "$out" | sed -n -E "s/.*\"$1\":\"(([^\"\\\\]|\\\\.)*)\".*/\\1/p"; }
text=$(field text)
alt=$(field alt)
tooltip=$(field tooltip)
[ -n "$text" ] || exit 0

# "Calera · Alabama, United States — Overcast\nTemperature: 72°F (Feels like 73°)\nHigh: 74° · Low: 65°\nWind: …"
line() { printf '%s' "$tooltip" | awk -v n="$1" 'BEGIN { RS = "\\\\n" } NR == n { print; exit }'; }
first=$(line 1)
place=${first%% · *}
place=${place%% — *}
condition=${first##* — }
feels=$(line 2 | sed -n 's/.*(\(Feels like [^)]*\)).*/\1/p')
highlow=$(line 3)
wind=$(line 4)
temp=${text#* }

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

label() { printf '{"node":"label","text":"%s"%s}' "$1" "$2"; }
icon() { printf '{"node":"icon","name":"%s","size":%s}' "$icon" "$1"; }
temp_label() { label "<span weight=\\\"light\\\">$temp</span>" ",\"markup\":true,\"font_size\":$1"; }
dim=',"color":"dim_text"'

printf '%s' "$hello" | grep -o '"item":"card","monitor":"[^"]*","size":"[^"]*"' | while IFS= read -r w; do
  monitor=$(printf '%s' "$w" | sed -n 's/.*"monitor":"\([^"]*\)".*/\1/p')
  size=$(printf '%s' "$w" | sed -n 's/.*"size":"\([^"]*\)".*/\1/p')
  case $size in
    compact)
      view="{\"node\":\"box\",\"orientation\":\"vertical\",\"spacing\":4,\"expand\":true,\"children\":[$(icon 40),$(temp_label 36),$(label "$condition" "$dim")]}"
      ;;
    large)
      top="{\"node\":\"box\",\"spacing\":16,\"children\":[$(icon 72),$(temp_label 64)]}"
      details="{\"node\":\"box\",\"orientation\":\"vertical\",\"spacing\":4,\"margin\":[16,0,0,0],\"children\":[$(label "$feels" "$dim"),$(label "$highlow" "$dim"),$(label "$wind" "$dim")]}"
      view="{\"node\":\"box\",\"orientation\":\"vertical\",\"spacing\":6,\"expand\":true,\"children\":[$(label "$place" ',"class":["title"]'),$top,$(label "$condition" ',"font_size":17'),$details]}"
      ;;
    *)
      left="{\"node\":\"box\",\"spacing\":10,\"children\":[$(icon 44),$(temp_label 44)]}"
      right="{\"node\":\"box\",\"orientation\":\"vertical\",\"spacing\":2,\"children\":[$(label "$condition" ',"class":["title"]'),$(label "$place" "$dim"),$(label "$highlow" "$dim")]}"
      view="{\"node\":\"box\",\"spacing\":16,\"expand\":true,\"children\":[$left,$right]}"
      ;;
  esac
  printf '{"type":"item","id":"card","monitor":"%s","view":%s,"tooltip":"%s","on_click":["nyx-weather","--toggle"]}\n' "$monitor" "$view" "$tooltip"
done
