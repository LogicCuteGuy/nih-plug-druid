# nih-plug-druid

Druid editor support for [NIH-plug](https://github.com/robbert-vdh/nih-plug), plus an example gain plugin.

## Workspace

- `nih_plug_druid`: adapter crate that integrates Druid GUIs with NIH-plug editors
- `examples/gain_gui_druid`: example plugin using the adapter
- `xtask`: packaging tasks powered by `nih_plug_xtask`

## Prerequisites

- Rust toolchain (stable)
- Cargo
- A plugin host/DAW for testing built binaries

## Build

```powershell
cargo check --workspace
```

## Bundle example plugin

```powershell
cargo xtask bundle gain_gui_druid --release
```

Generated bundles are written to:

- `target/bundled/`

## Notes

- The example plugin crate is `gain_gui_druid`.
- The adapter crate in this repository is `nih_plug_druid`.

## Platform notes

### Linux

On Linux, Druid requires gtk+3; see the GTK installation page. (On Ubuntu-based distros, running `sudo apt-get install libgtk-3-dev` from the terminal will do the job.)

### OpenBSD

On OpenBSD, Druid requires gtk+3; install from packages:

```sh
pkg_add gtk+3
```

Alternatively, there is an X11 backend available, although it is currently missing quite a few features. You can try it out with `--features=x11`.

## Known issues

- Linux: still buggy (GUI close/reopen can be unstable depending on host).
- macOS: still buggy.
