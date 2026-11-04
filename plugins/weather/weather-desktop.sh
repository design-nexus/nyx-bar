#!/bin/sh
# Nyx weather as a desktop widget: every ten minutes, Nyx Weather's conditions (and,
# when it's large, the next hours) as a card for each placed widget. A click opens
# Nyx Weather.

read -r hello

label() { printf '{"node":"label","text":"%s"%s}' "$1" "$2"; }
icon() { printf '{"node":"icon","name":"%s","size":%s}' "$2" "$1"; }
dim=',"color":"dim_text"'

# `nyx-weather --json` (newer): flat fields, already in the user's units.
out=$(nyx-weather --json 2>/dev/null)
field() { printf '%s' "$out" | sed -n -E "s/.*\"$1\":\"(([^\"\\\\]|\\\\.)*)\".*/\\1/p"; }
if [ -n "$(field condition)" ]; then
  condition=$(field condition)
  place=$(field location)
  weather_icon=$(field icon)
  temp=$(field temperature)
  feels="Feels like $(field feels_like)"
  highlow="High: $(field high) · Low: $(field low)"
  wind="Wind: $(field wind) · Humidity: $(printf '%s' "$out" | sed -n 's/.*"humidity":\([0-9]*\).*/\1/p')%"
  tooltip="$(field place) — $condition"
  # Keys come sorted: icon, precipitation, temperature, time.
  hours=$(printf '%s' "$out" | grep -o '{"icon":"[^"]*","precipitation":[0-9]*,"temperature":"[^"]*","time":"[^"]*"}' | head -5)
else
  # Older Nyx Weather: its bar summary, without hours.
  out=$(nyx-weather --waybar 2>/dev/null) || exit 0
  text=$(field text)
  [ -n "$text" ] || exit 0
  alt=$(field alt)
  tooltip=$(field tooltip)
  line() { printf '%s' "$tooltip" | awk -v n="$1" 'BEGIN { RS = "\\\\n" } NR == n { print; exit }'; }
  first=$(line 1)
  place=${first%% · *}
  place=${place%% — *}
  condition=${first##* — }
  feels=$(line 2 | sed -n 's/.*(\(Feels like [^)]*\)).*/\1/p')
  highlow=$(line 3)
  wind=$(line 4)
  temp=${text#* }
  hours=""
  case $alt in
    *thunder* | *storm*) weather_icon=weather-storm-symbolic ;;
    *snow* | *sleet*) weather_icon=weather-snow-symbolic ;;
    *rain* | *drizzle* | *shower*) weather_icon=weather-showers-symbolic ;;
    *fog* | *mist* | *haze*) weather_icon=weather-fog-symbolic ;;
    *overcast* | *cloudy*) weather_icon=weather-overcast-symbolic ;;
    *partly* | *few*) weather_icon=weather-few-clouds-symbolic ;;
    *night*) weather_icon=weather-clear-night-symbolic ;;
    *) weather_icon=weather-clear-symbolic ;;
  esac
fi

temp_label() { label "<span weight=\\\"light\\\">$temp</span>" ",\"markup\":true,\"font_size\":$1"; }

# The next hours: time, icon, temperature, side by side.
strip=""
if [ -n "$hours" ]; then
  cells=$(printf '%s\n' "$hours" | while IFS= read -r h; do
    t=$(printf '%s' "$h" | sed -n 's/.*"time":"\([^"]*\)".*/\1/p')
    i=$(printf '%s' "$h" | sed -n 's/.*"icon":"\([^"]*\)".*/\1/p')
    d=$(printf '%s' "$h" | sed -n 's/.*"temperature":"\([^"]*\)".*/\1/p')
    printf '{"node":"box","orientation":"vertical","spacing":4,"width":56,"children":[%s,%s,%s]},' \
      "$(label "$t" "$dim,\"font_size\":12")" "$(icon 20 "$i")" "$(label "$d")"
  done)
  strip="{\"node\":\"box\",\"spacing\":4,\"margin\":[18,0,0,0],\"children\":[${cells%,}]}"
fi

printf '%s' "$hello" | grep -o '"item":"card","monitor":"[^"]*","size":"[^"]*"' | while IFS= read -r w; do
  monitor=$(printf '%s' "$w" | sed -n 's/.*"monitor":"\([^"]*\)".*/\1/p')
  size=$(printf '%s' "$w" | sed -n 's/.*"size":"\([^"]*\)".*/\1/p')
  case $size in
    compact)
      view="{\"node\":\"box\",\"orientation\":\"vertical\",\"spacing\":4,\"expand\":true,\"children\":[$(icon 40 "$weather_icon"),$(temp_label 36),$(label "$condition" "$dim")]}"
      ;;
    large)
      top="{\"node\":\"box\",\"spacing\":16,\"children\":[$(icon 64 "$weather_icon"),$(temp_label 56)]}"
      if [ -n "$strip" ]; then
        below=$strip
      else
        below="{\"node\":\"box\",\"orientation\":\"vertical\",\"spacing\":4,\"margin\":[16,0,0,0],\"children\":[$(label "$feels" "$dim"),$(label "$highlow" "$dim"),$(label "$wind" "$dim")]}"
      fi
      view="{\"node\":\"box\",\"orientation\":\"vertical\",\"spacing\":6,\"expand\":true,\"children\":[$(label "$place" ',"class":["title"]'),$top,$(label "$condition · $highlow" "$dim"),$below]}"
      ;;
    *)
      left="{\"node\":\"box\",\"spacing\":10,\"children\":[$(icon 44 "$weather_icon"),$(temp_label 44)]}"
      right="{\"node\":\"box\",\"orientation\":\"vertical\",\"spacing\":2,\"children\":[$(label "$condition" ',"class":["title"]'),$(label "$place" "$dim"),$(label "$highlow" "$dim")]}"
      view="{\"node\":\"box\",\"spacing\":16,\"expand\":true,\"children\":[$left,$right]}"
      ;;
  esac
  printf '{"type":"item","id":"card","monitor":"%s","view":%s,"tooltip":"%s","on_click":["nyx-weather","--toggle"]}\n' "$monitor" "$view" "$tooltip"
done
