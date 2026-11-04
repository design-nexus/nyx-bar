# Packaging Nyx Bar for NyxOS

For the session working on `nyx-packages` and `nyxos`. Nyx Bar is pushed at
`design-nexus/nyx-bar`; its build was tested on Arch, not yet under Nix.

## nyx-packages

The plugin crates moved out of this repo on 2026-11-03: `nyx-plugin`,
`nyx-plugin-protocol`, `nyx-plugin-host` and `nyx-plugin-sdk` are in nyx-core
(e478d83), `nyx-plugin-view` in nyx-design (aec4b03). They're git dependencies here,
so the recipe's cargo lock needs their output hashes like the other git crates.

1. `flake.nix` inputs: `nyx-bar = { url = "github:design-nexus/nyx-bar"; flake = false; };`
2. Copy [nyx-bar.nix](nyx-bar.nix) to `pkgs/nyx-bar.nix` and add `"nyx-bar"` to the
   `fromSource` list in `pkgs/default.nix`.
3. `modules/nixos/default.nix`: add `bar = "Nyx Bar and its plugins";` to `apps`. The
   unit starts with the graphical session through `passthru.autostart`.
4. A check: `nyx.testWorkspace { pname = "nyx-bar-tests"; src = inputs.nyx-bar;
   buildInputs = [ gtk4-layer-shell librsvg ]; nativeCheckInputs = [ bubblewrap ]; }`.
   The sandbox and scope tests skip themselves where bubblewrap or a user systemd
   isn't available (the Nix sandbox).
5. Optional: Weather's plugin needs `nyx-weather` on PATH, so it only shows when
   `nyx.weather.enable` is on. The Control Center and Notification Center buttons
   likewise hide until those are packaged.

## nyxos

1. `modules/nixos/apps.nix`: `nyx.bar.enable = lib.mkDefault true;`.
2. `modules/home/hyprland.nix`: the "temporary minimal session" comment can lose
   "Nyx Bar".
3. `pkgs/nyx-cli/nyx.sh` already passes `nyx plugin …` to `nyx-plugin` (commit
   013391b, not pushed).

## On the VM, check

- The bar shows on the VM's display with workspaces, clock, stats, system items.
- `nyx plugin list` shows the first-party plugins as `packaged, trusted`.
- `nyx plugin new me.test --sh && nyx plugin link me.test && nyx plugin enable me.test --yes`
  shows a sandboxed item (bubblewrap needs unprivileged user namespaces, which NixOS
  allows by default).
- `systemctl --user status 'nyx-plugin-bar-*'` lists one scope per plugin.
