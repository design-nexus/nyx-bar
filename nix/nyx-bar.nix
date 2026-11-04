# Nyx Bar for nyx-packages (copy to pkgs/nyx-bar.nix): the bar, `nyx-plugin`, and the
# first-party plugins in share/nyx/plugins, where the bar finds them as packaged
# (trusted) plugins through XDG_DATA_DIRS.
{
  nyx,
  src,
  gtk4-layer-shell,
  librsvg,
  bubblewrap,
  xdg-dbus-proxy,
  git,
  curl,
  gnutar,
  gzip,
  systemd,
  wireplumber,
  pulseaudio,
  networkmanager,
  util-linux,
  libnotify,
}:
nyx.buildApp {
  pname = "nyx-bar";
  inherit src;
  cargoBuildFlags = [
    "-p" "nyx-bar"
    "-p" "nyx-plugin-cli"
    "-p" "nyx-bar-workspaces"
    "-p" "nyx-bar-system"
    "-p" "nyx-bar-stats"
    "-p" "nyx-bar-panels"
    "-p" "nyx-bar-media"
    "-p" "nyx-bar-tray"
  ];
  buildInputs = [
    gtk4-layer-shell
    librsvg
  ];
  # The host runs plugins in systemd scopes and bubblewrap; nyx-plugin clones, fetches
  # and unpacks; the first-party plugins use wpctl, pactl, nmcli and cal. Trusted
  # plugins inherit the bar's PATH.
  runtimeInputs = [
    bubblewrap
    xdg-dbus-proxy
    git
    curl
    gnutar
    gzip
    systemd
    wireplumber
    pulseaudio
    networkmanager
    util-linux
    libnotify
  ];
  userUnits = [ "data/nyx-bar.service" ];
  passthru.autostart."nyx-bar" = "graphical-session.target";
  postInstall = ''
    for dir in plugins/*/; do
      [ -f "$dir/plugin.toml" ] || continue
      id=$(sed -n 's/^id = "\(.*\)"/\1/p' "$dir/plugin.toml" | head -1)
      dest=$out/share/nyx/plugins/$id
      install -Dm644 "$dir/plugin.toml" -t "$dest"
      for f in "$dir"*.sh; do [ -e "$f" ] && install -Dm755 "$f" -t "$dest"; done
      [ -e "$dir/style.css" ] && install -Dm644 "$dir/style.css" -t "$dest"
      name=$(basename "$dir")
      if [ -e "$out/bin/nyx-bar-$name" ]; then mv "$out/bin/nyx-bar-$name" "$dest/"; fi
    done
  '';
  meta = {
    description = "A top bar for Hyprland whose every item is a plugin";
    homepage = "https://github.com/design-nexus/nyx-bar";
    mainProgram = "nyx-bar";
  };
}
