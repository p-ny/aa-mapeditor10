# AA Map Editor 10.x

A world editor for ArcheAge 10.x and [AAEmu](https://github.com/AAEmu) servers, written in Rust on
[Bevy](https://bevyengine.org) 0.20 and egui. It is a fork of Nick Messer's aa-editor and uses the engine crates of
[AABevy1.2](https://github.com/NickMesser/AABevy1.2).

It renders the real streamed world (terrain, objects, vegetation, water, sky) and edits:

* terrain: sculpt, paint (also new surface types), tint, holes, Stamp brush (copies ground from elsewhere)
* static world objects: move, tilt, copy, delete, place new models, roads and water copied from donors
* vegetation brush
* NPC spawners and map doodads (`doodad.g` per cell), plus AAEmu doodad spawn files
* housing zones and their rule sets
* navigation (navmesh heights) for the server

> Work in progress. Fan project, not affiliated with XL Games or Kakao Games. **No game files are included.**
> You need your own 10.x client and an AAEmu server's `compact.sqlite3`.

## Build

Rust stable (edition 2024), Windows x64.

```
cargo build --locked --release -p aa-editor
cargo build --locked --release -p aa-cell10-probe   # optional cell/publish tools
```

## Run

Copy `target/release/aa-editor.exe` next to the files in `package/`, set the paths in `editor-config.txt` and
run `Start Editor.cmd`. Flags are documented at the top of `crates/aa-editor/src/main.rs` and in `docs/aa-editor.md`.

Some tools in `aa-cell10-probe` (publish, nav, multi-cell-trial) still have the author's workspace paths as
defaults; read them before use.

## License

MIT OR Apache-2.0, see `LICENSE.txt`.
