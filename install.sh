#!/usr/bin/env bash
# Install Nyx Bar, the `nyx-plugin` command and the first-party plugins.
#
#   curl -fsSL https://raw.githubusercontent.com/design-nexus/nyx-bar/main/install.sh | bash
#
# Installing doesn't start the bar. Try it with `nyx-bar`, or run it at login with
#   systemctl --user enable --now nyx-bar
set -euo pipefail

REPO="design-nexus/nyx-bar"
say() { printf '\033[1m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[33m==>\033[0m %s\n' "$*" >&2; }
die() { printf '\033[31m==>\033[0m %s\n' "$*" >&2; exit 1; }

command -v hyprctl >/dev/null || warn "Hyprland wasn't found. Nyx Bar is made for Hyprland."

if command -v pacman >/dev/null; then
  need=()
  for p in gtk4 gtk4-layer-shell librsvg bubblewrap xdg-dbus-proxy pkgconf gcc git; do pacman -Qq "$p" >/dev/null 2>&1 || need+=("$p"); done
  if ((${#need[@]})); then
    say "Installing ${need[*]}"
    sudo pacman -S --needed --noconfirm "${need[@]}"
  fi
fi

# This checkout, or a fresh one when piped from curl.
here=""
[[ -n ${BASH_SOURCE[0]:-} && -f ${BASH_SOURCE[0]} ]] && here=$(cd "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
if [[ -n $here && -f $here/Cargo.toml && -d $here/crates ]]; then
  src=$here
else
  say "Fetching the source"
  git clone -q --depth 1 "https://github.com/$REPO.git" "$work/src"
  src="$work/src"
fi

if ! command -v cargo >/dev/null; then
  command -v pacman >/dev/null || die "Rust (cargo) is needed to build Nyx Bar. Install it and run this again."
  say "Installing rust"
  sudo pacman -S --needed --noconfirm rust
fi

say "Building Nyx Bar and its plugins (a few minutes the first time)"
plugins=(workspaces system stats panels media tray)
pkgs=(-p nyx-bar -p nyx-plugin-cli)
for p in "${plugins[@]}"; do pkgs+=(-p "nyx-bar-$p"); done
(cd "$src" && cargo build --release --locked "${pkgs[@]}")
target="${CARGO_TARGET_DIR:-$src/target}/release"

bin="$HOME/.local/bin"
lib="$HOME/.local/lib/nyx/plugins"
units="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
mkdir -p "$bin" "$lib" "$units"
# Replace files rather than writing into them, so running copies keep their old binary.
for b in nyx-bar nyx-plugin; do
  install -m 755 "$target/$b" "$bin/$b.new" && mv -f "$bin/$b.new" "$bin/$b"
done

# Each first-party plugin is a folder: its plugin.toml, scripts, style, and program.
for dir in "$src"/plugins/*/; do
  [[ -f $dir/plugin.toml ]] || continue
  id=$(sed -n 's/^id = "\(.*\)"/\1/p' "$dir/plugin.toml" | head -1)
  [[ -n $id ]] || continue
  stage="$lib/.$id.new"
  rm -rf "$stage"
  mkdir -p "$stage"
  for f in "$dir"plugin.toml "$dir"*.sh "$dir"*.py "$dir"style.css; do
    [[ -f $f ]] && install -m "$([[ $f == *.sh || $f == *.py ]] && echo 755 || echo 644)" "$f" "$stage/"
  done
  name=$(basename "$dir")
  [[ -x $target/nyx-bar-$name ]] && install -m 755 "$target/nyx-bar-$name" "$stage/"
  rm -rf "$lib/.$id.old"
  [[ -e $lib/$id ]] && mv "$lib/$id" "$lib/.$id.old"
  mv "$stage" "$lib/$id"
  rm -rf "$lib/.$id.old"
done

install -m 644 "$src/data/nyx-bar.service" "$units/nyx-bar.service"
systemctl --user daemon-reload 2>/dev/null || true

if systemctl --user is-active --quiet nyx-bar.service 2>/dev/null; then
  say "Installed. The running bar uses the new build after: systemctl --user restart nyx-bar"
else
  say "Installed. Try it with: nyx-bar   (or at every login: systemctl --user enable --now nyx-bar)"
fi
say "Plugins: nyx plugin list"
case ":$PATH:" in
  *":$bin:"*) ;;
  *) warn "$bin isn't on your PATH." ;;
esac
