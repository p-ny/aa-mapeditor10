# AA Map Editor 10.x: manual

A world editor for the ArcheAge 10.x client and AAEmu servers. It renders the real game world (streamed terrain,
objects, vegetation, water, sky) from your own client's `game_pak` and lets you change terrain, objects, vegetation,
spawns and housing zones.

No game files come with it. You need:

* an ArcheAge **10.x** client folder (the one that contains `game_pak`)
* the `compact.sqlite3` of an AAEmu server for 10.x (in the server's `Data` folder). The editor only reads it.
* Windows 10/11 x64 and a GPU with DirectX 12 or Vulkan

## 1. Setup

1. Unzip anywhere, for example `C:\AAMapEditor`.
2. Open `editor-config.txt` in Notepad and set:
   * `client_folder` = your 10.x client folder, e.g. `C:\ArcheAge`
   * `database` = the server's `compact.sqlite3`, e.g. `C:\AAEmu\Data\compact.sqlite3`
   * optional `server_worlds` = the server's `Data\Worlds` folder, to show and edit its doodad spawn files
   * `save_folder` = where your edits go (default `saves` next to the editor)
3. Double-click **Start Editor.cmd**. The first start takes a while (shaders, first cells).

The editor never writes into your client or the database. Everything you save lands in `save_folder`.

Start somewhere else: `Start-Editor.ps1 -Position 15578,15382` (world X,Y in metres) or
`Start-Editor.ps1 -World instance_library` for another world. The world menu inside the editor also switches worlds.

## 2. Moving around

| Input | Action |
|---|---|
| hold right mouse + move | look |
| W A S D, Space / Ctrl | fly; hold Shift for fast |
| left click | select (or place / paint, depending on the mode) |
| drag the selection | move it |
| `[` `]` or Shift + wheel | turn the selection |
| Alt + wheel, numpad keys | tilt / raise objects freely in the air (bridges etc.) |
| F | focus the selection |
| Delete, Ctrl+D | delete, duplicate |
| Ctrl+Z / Ctrl+Y | undo / redo |
| Ctrl+S | **save** |
| M | teleport map |
| B | show zone borders as walls (the panel names the zone you are in) |
| F5 / Esc | play mode (walk the world) / stop |

The gizmo in the top-right corner orbits (drag), looks along an axis (click), zooms and pans.

## 3. Modes

Pick a mode in the toolbar.

**Select**: click NPCs, doodads and map objects, move/turn/delete them, edit them in the inspector.

**Terrain**: brushes Raise, Lower, Smooth, Flatten, Paint, Hole, Tint and Stamp.
* *Paint* uses the cell's surface types; you can add new surface types to a cell. Paint also tints the ground
  colour so the material looks right from far away.
* *Tint* only recolours the ground (useful on bare or snow ground).
* *Stamp* copies real ground from elsewhere: Alt+click a source spot (anywhere in the world), then paint. The first
  stroke fixes the offset; later strokes copy surface materials and ground colour from the same offset.
* Brush size and strength are in the tool panel.

**Objects**: static world objects of the cells (houses, rocks, bridges, props). Move, tilt, copy, delete. **More
objects** / Browser > Buildings places any model from the pak, with a preview on hover. Roads and water can be
copied from existing ones ("donors").

**Vegetation brush** (Objects > More objects > New vegetation > Paint with a brush): click one or more plants in the
list so they are highlighted, then hold the left mouse button to scatter them; Shift + drag removes plants. Works on
cells that had no plants before (sea cells, new islands). If nothing is picked the panel says "No plant chosen yet".

**Place**: pick an NPC or doodad in the browser, then click the ground to place it (Shift-click keeps placing, Esc
stops). NPC spawners and the map doodads of each cell (`doodad.g`) are shown in Select and can be moved; new doodads
go into `doodad_spawns_editor_overlay.json`.

**Housing**: housing zones (where players may build) with their corners and rule sets.

**Paths**: AI routes NPCs walk.

## 4. Saving

Ctrl+S saves everything you changed. Layout of `save_folder`:

| Folder | Contents | Goes to |
|---|---|---|
| `overlay\worlds\<world>\cells\XXX_YYY\client\...` | terrain (`heightmap.dat`, `cover.ctc`, `leveldata.xml`), objects (`brush.dat`, `object.dat`, `statobjs.dat`, `vegetation*`, `materials.dat`, `material_list.dat`) | the **client** pak (step 5), and the server's client data for terrain heights |
| `level-doodad-overlay\...` | changed map doodads per cell (`doodad.g`) | the server's `game\worlds\...` client data folder |
| `spawner-overlay\...` | changed NPC spawners | the server's `game\worlds\...` client data folder |
| `doodad-overlay\<world>\doodad_spawns_editor_overlay.json` | new doodads | the server's `Data\Worlds\<world>\` |
| `housing-overlay\...` | changed housing zones | client pak + server client data |
| `logs\editor.log` | log | send it along with bug reports |

Cells are 1024 m squares named `XXX_YYY` (x and y index from the south-west corner).

## 5. Putting your edits into the game

**Client** (everyone who plays needs the same files):

1. Close the game client. **Make a backup copy of `game_pak` the first time.**
2. Double-click **Install Edits Into Client.cmd**. It writes every file of `saves\overlay` into `game_pak`,
   reads them back to check, and keeps an undo log in `saves\installs`.
3. **Undo Last Install.cmd** restores the pak to how it was before the last install. To go back further, run it again.

Players need the patched `game_pak` (or the same cell files patched into theirs). A client without them shows the
old terrain and objects.

**Server** (AAEmu):

* Terrain heights: copy `saves\overlay\worlds` into the folder the server reads the client files from (its
  `game\worlds`), so the server's ground matches the client.
* Map doodads / NPC spawners: copy `level-doodad-overlay` and `spawner-overlay` contents into the same place.
* New doodads: copy the json from `doodad-overlay\<world>\` into the server's `Data\Worlds\<world>\`. AAEmu loads
  extra `doodad_spawns*.json` files whose name contains `overlay`.
* Restart the server after copying: it loads world data at startup.
* This build does **not** regenerate the server's navigation (NPC pathing) for changed terrain. Small edits are fine;
  on big terrain changes NPCs may walk through or under the new ground.

## 6. Tips and limits

* Save often; each Ctrl+S writes the current state of the edited cells.
* Neighbouring cells share edges: edit both sides of a border in one session so they match.
* Brand-new land far out at sea (like a new island) also needs a zone/map entry on the server before players can
  use it; the editor only makes the ground.
* If the window stays black or the editor quits at start, check `saves\logs\editor.log`. The most common cause is a
  wrong `client_folder` or `database` path.
* Fan project, not affiliated with XL Games or Kakao Games. License MIT OR Apache-2.0 (see `LICENSE.txt`, `LICENSE-MIT`, `LICENSE-APACHE`;
  bundled crates in `THIRD-PARTY-NOTICES.txt`).
  Based on Nick Messer's aa-editor and the AABevy1.2 engine crates.
