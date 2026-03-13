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
