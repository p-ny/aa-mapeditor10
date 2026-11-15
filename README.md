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

## Using it

Full user manual (setup, controls, every tool, saving, putting edits into client and server):
[package/MANUAL.md](package/MANUAL.md).

## Build

Rust stable (edition 2024), Windows x64. The engine crates are fetched from AABevy1.2 by git.

```
cargo build --locked --release -p aa-editor
cargo build --locked --release -p aa-cell10-probe --bin pak-patch
```

Make a runnable folder: copy `target/release/aa-editor.exe`, `target/release/pak-patch.exe`, the `LICENSE*` and
`THIRD-PARTY-NOTICES.txt` files and
everything in `package/` into one folder, set the paths in `editor-config.txt`, run `Start Editor.cmd`.
Editor flags are documented at the top of `crates/aa-editor/src/main.rs` and in `docs/aa-editor.md` (written for
the 1.2 lane; the 10.x flags are `--tenx` and the overlay flags used by `package/Start-Editor.ps1`).

## Cell tools (`aa-cell10-probe`)

Command-line tools; `AA_PAK` selects the client's `game_pak`, `AA_PAK_OVERLAY` the saved edits.

| Tool | What it does |
|---|---|
| `pak-patch apply <pak> <undo> game/<path>=<file>...` / `pak-patch undo <pak> <undo>` | write files into a `game_pak` with an undo log, verified by readback |
| `island-gen <mask.txt> <overlay> [preview.ppm]` | raise a new island from a mask into the cells of an overlay (heights, surfaces, cover); used for Saltswept Atoll |
| `island-finish catalog / erode / bake` | ground-colour catalog of real terrain, erosion, and cover baking so generated land looks like the game's own |
| `cell-info <world> [XXX_YYY...]` | height range, sea level, land share of cells |
| `zone-map <world> <x> <y> <w> <h> <out.dds>` | shaded top-down map image of an area, for zone maps |
| `cell-objects`, `cover-info`, `veg-info`, `model-find`, `terrain-sample`, `cell-tidy` | inspection helpers |
| `publish`, `apply`, `nav`, `multi-cell-trial`, `roundtrip-terrain` | the author's publish pipeline; they still default to the author's workspace paths, read before use |

## License

MIT OR Apache-2.0, at your option: [LICENSE-MIT](LICENSE-MIT), [LICENSE-APACHE](LICENSE-APACHE), summary in
[LICENSE.txt](LICENSE.txt). Third-party crates: [THIRD-PARTY-NOTICES.txt](THIRD-PARTY-NOTICES.txt).

The engine is pinned to AABevy1.2 commit `17d6258`, the last one under MIT OR Apache-2.0. AABevy1.2 is
GPL-3.0-or-later from 2026-10-29 on: moving the pin past that makes this project GPL-3.0 as well.

ArcheAge and its assets belong to their owners; none are included. Fan project, not affiliated with XL Games or
Kakao Games.
