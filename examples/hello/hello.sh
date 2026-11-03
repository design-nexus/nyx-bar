#!/bin/sh
# Nyx Bar plugins read JSON lines on stdin and write JSON lines on stdout.
# The first line in is "hello"; then come events, and "shutdown" at the end.

read -r _hello
clicks=0

show() {
  printf '{"type":"item","id":"main","icon":"face-smile-symbolic","text":"%s","tooltip":"Click me"}\n' "$1"
}

show "Hello"
while read -r line; do
  case $line in
    *'"type":"shutdown"'*) exit 0 ;;
    *'"kind":"click"'*)
      clicks=$((clicks + 1))
      show "Hello ×$clicks"
      ;;
  esac
done
