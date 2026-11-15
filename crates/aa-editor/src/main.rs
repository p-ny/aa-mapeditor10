//! AABevy world editor: the real streamed world (`aa-world`) with editing tools on top.
//!
//! ```text
//! cargo run -p aa-editor -- --pos 15578,15382            # Nuia start
//! cargo run -p aa-editor -- --spawn-dir <Data/Worlds/main_world> --db <compact.sqlite3> --shot shots/aa-editor/ui.png
//! ```
//!
//! * spawns: NPCs and doodads of the AAEmu server's spawn files ([`spawns`]); select, move, rotate, delete, place new
//!   ones from the browser; previews are the real models ([`preview`]);
//! * terrain: raise / lower / smooth / flatten brushes on the streamed terrain, saved as `heightmap.dat` into a pak
//!   overlay folder the client reads through `AA_PAK_OVERLAY` ([`terrain_edit`], [`heightmap_write`]);
//! * game data: NPC, doodad, item and quest rows of the server's `compact.sqlite3` ([`db`]);
//! * Play: walk the edited world as a character ([`play`]).
//!
//! Flags (plus the `aa-dev` ones `--shot --shot-after --exit-after --size`):
//!
//! * `--world <name>`        default `main_world`
//! * `--pos x,y[,z]`         start camera position (cry / server coordinates)
//! * `--no-fog`              start with the atmosphere fog off (View > Fog)
//! * `--look yaw,pitch`      start camera direction in degrees (yaw 0 = north, clockwise; pitch -90 = straight down)
//! * `--spawn-dir <dir>`     the server's `Data/Worlds/<world>` folder the spawn files are read from and saved to
//!   (default: the bevy lane's source tree `server/Data/Worlds/<world>`)
//! * `--mirror <dir>`        another folder that receives a copy of saved spawn files (repeatable; default: the lane's
//!   runtime folder `server/Data/Worlds/<world>` when it exists)
//! * `--db <file>`           the game database to edit (default: the lane's `server/Data/compact.sqlite3`)
//! * `--read-only-db`        never write the database
//! * `--overlay <dir>`       pak overlay folder for terrain saves (default `data/local/editor/overlay`)
//! * `--select <unit_id>`    select the first spawn of this NPC / doodad template near the start (for screenshots)
//! * `--mode <select|place|terrain|objects|paths|housing>`, `--play` start in that mode
//! * `--row <npcs|doodads|items|quests>:<id>` open that database row in the row editor
//! * checks: `--sculpt x,y` holds the terrain brush there for 120 frames (`--brush-radius`, `--brush-strength`),
//!   `--sculpt-save` then saves the terrain
//! * `--preview <npcs|doodads|items|quests>:<id>` open the model preview with that row
//! * `--export-pak <game_pak>` exports the pak overlay into that pak at startup ([`pak_export`]; never `D:/AA1.2`)
//!
//! * `--teleport`            open the teleport window at start
//! * `--tenx` doodads: read from `--world-spawn-dir` (default the test World's `Data/Worlds/<world>`), saved whole into
//!   `--doodad-overlay` (default `data/local/doodad-overlay/<world>`); NPCs: native zone spawners, `--spawner-overlay`;
//!   map doodads of the cells' `doodad.g` (what the World plants), saved to `--level-doodad-overlay`
//!   (default `data/local/level-doodad-overlay`)
//! * `--path-dir <dir>`      the server's `Data/Path` folder of AI routes ([`paths`]; default: next to the spawn folder)
//! * `--switch-world <name>` check: switch to that world (restart) once running
//! * `--housing <area id>`   start in Housing mode with the zone of that `housing_areas` id selected ([`housing`]);
//!   `--housing-nudge dx,dy` (check) then moves it and saves
//!
//! Controls: hold the right mouse button to look, WASD / Space / Ctrl to fly (Shift fast), left click to select or
//! place, drag the selection to move it, `[` `]` or Shift + wheel to turn it, Delete, Ctrl+D duplicate, Ctrl+Z / Ctrl+Y,
//! Ctrl+S save, F focus, F5 play, Esc stop, M teleport map. The gizmo in the top-right corner of the view orbits
//! (drag), looks along an axis (click), zooms and pans ([`nav`]); the world menu restarts on another world ([`worlds`]).

// Bevy systems take many parameters and nested query types.
#![allow(clippy::too_many_arguments, clippy::type_complexity)]

mod brush_library;
mod camera;
mod tenx_spatial;
mod object_save;
mod native_tables;
mod db;
mod ground_colours;
mod heightmap_write;
mod housing;
mod jsonc;
mod level_doodads;
mod model_preview;
mod nav;
mod navmesh;
mod navmesh_add;
mod objects_extra;
mod paths;
mod surface_add;
mod teleport;
mod pak_export;
mod npc_look;
mod play;
mod play_server;
mod preview;
mod server_terrain;
mod spawns;
mod terrain_edit;
mod terrain_paint;
mod cover_paint;
mod tools;
mod ui;
mod veg_brush;
mod views;
mod world_objects;
mod worlds;
mod zone_borders;
mod zone_spawners;

use std::path::PathBuf;

use aa_world::{AaWorldPlugin, cry_to_bevy};
use bevy::prelude::*;
use bevy_egui::{EguiGlobalSettings, EguiPlugin, PrimaryEguiContext};

use crate::db::{GameData, Table};
use crate::spawns::{Kind, SpawnStore};

pub fn arg(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn args(name: &str) -> Vec<String> {
    let args: Vec<String> = std::env::args().collect();
    args.windows(2).filter(|w| w[0] == name).map(|w| w[1].clone()).collect()
}

pub fn flag(name: &str) -> bool {
    std::env::args().any(|a| a == name)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Select,
    Place,
    Terrain,
    /// Static world objects of the cell files (brushes, vegetation, entities).
    Objects,
    /// The AI routes NPCs walk ([`paths`]).
    Paths,
    /// Housing zones and their rules ([`housing`]).
    Housing,
}

/// Where things are saved.
#[derive(Clone, Debug)]
pub struct Paths {
    pub spawn_dir: PathBuf,
    pub mirrors: Vec<PathBuf>,
    pub db: PathBuf,
    pub overlay: PathBuf,
}

/// The editor's state shared by the tools and the panels.
#[derive(Resource)]
pub struct Editor {
    pub world: String,
    pub paths: Paths,
    pub mode: Mode,
    /// Index into [`SpawnStore::spawns`].
    pub selected: Option<usize>,
    /// The template the Place tool puts down.
    pub place: Option<(Kind, u32)>,
    pub browser: Table,
    /// The Browser shows the 10.x model library (Buildings tab) instead of a table; `models_all` lifts the building filter.
    pub models_tab: bool,
    pub models_all: bool,
    pub search: String,
    pub view_radius: f32,
    pub show_npcs: bool,
    pub show_doodads: bool,
    /// The time of day's atmosphere fog (`--no-fog` starts with it off).
    pub fog: bool,
    pub snap_to_ground: bool,
    pub status: String,
    /// Open row editor.
    pub row: Option<ui::RowEdit>,
    pub playing: bool,
    /// Requested by the panels, done by the systems that own the data.
    pub requests: Vec<Request>,
}

#[derive(Clone, Debug)]
pub enum Request {
    SaveSpawns,
    SaveTerrain,
    Focus(Vec3),
    Play,
    Stop,
    UndoTerrain,
    /// Open / close the "Play on server" window.
    PlayOnServer,
    /// Save the edited static world objects ([`world_objects`]).
    SaveObjects,
    /// Move the camera above a cry-space point (`z` NaN: the ground there) ([`nav`]).
    Teleport(Vec3),
    /// Look down at a cry-space point from this many metres ([`nav`]): frames a large thing like a housing zone.
    View(Vec3, f32),
    /// Open / close the teleport window ([`teleport`]).
    OpenTeleport,
    /// Restart the editor on another world ([`worlds`]).
    SwitchWorld(String),
    /// Save the edited AI routes ([`paths`]).
    SavePaths,
    /// Save the edited housing zones and rules ([`housing`]).
    SaveHousing,
    /// Start placing this static model in Objects mode (Browser > Buildings, [`objects_extra`]).
    PlaceModel(String),
}

impl Editor {
    pub fn status(&mut self, text: impl Into<String>) {
        let text = text.into();
        info!("{text}");
        self.status = text;
    }
}

/// The game DB's character tables for humanoid previews (aa-anim reads the shared copy read-only).
#[derive(Resource)]
pub struct AnimDb(pub Option<std::sync::Mutex<aa_anim::GameDb>>);

fn vec_arg(name: &str) -> Option<Vec<f32>> {
    let v: Vec<f32> = arg(name)?.split(',').filter_map(|s| s.trim().parse().ok()).collect();
    (v.len() >= 2).then_some(v)
}

fn main() {
    let world = arg("--world").unwrap_or_else(|| "main_world".into());
    let root = std::env::current_dir().unwrap_or_default();
    let overlay = arg("--overlay").map(PathBuf::from).unwrap_or_else(|| if flag("--tenx"){PathBuf::from("data/local/overlay")}else{root.join("data/local/editor/overlay")});
    if let Err(e) = std::fs::create_dir_all(&overlay) {
        eprintln!("cannot create overlay folder {}: {e}", overlay.display());
    }
    if flag("--tenx") {
        match object_save::recover(&overlay) {
            Ok(true)=>eprintln!("Recovered an interrupted native object save before loading cells"),
            Ok(false)=>{},
            Err(e)=>{eprintln!("Cannot recover native object save: {e:#}");std::process::exit(1);}
        }
    }
    // before anything opens the pak: the client data then includes our saved terrain
    // SAFETY: single threaded here, nothing reads the environment concurrently.
    unsafe { std::env::set_var("AA_PAK_OVERLAY", &overlay) };

    let spawn_dir = arg("--spawn-dir")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!("{}/{world}",if flag("--tenx"){"data/local/test-data/Worlds"}else{"server/Data/Worlds"})));
    let mut mirrors: Vec<PathBuf> = args("--mirror").into_iter().map(PathBuf::from).collect();
    if !flag("--tenx") && mirrors.is_empty() && arg("--spawn-dir").is_none() {
        let runtime = PathBuf::from(format!("server/Data/Worlds/{world}"));
        if runtime.is_dir() {
            mirrors.push(runtime);
        }
    }
    let db_path = arg("--db").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(if flag("--tenx"){"data/local/test-data/compact.sqlite3"}else{"server/Data/compact.sqlite3"}));

    // 10.x: doodads come from the test World's spawn files, saved edits from the doodad overlay.
    let doodad_overlay = arg("--doodad-overlay").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(format!("data/local/doodad-overlay/{world}")));
    let loaded = if flag("--tenx") {
        let base = arg("--world-spawn-dir").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(format!("data/local/server/game/Data/Worlds/{world}")));
        SpawnStore::load_layered(&base, &doodad_overlay)
    } else {
        SpawnStore::load(&spawn_dir)
    };
    let mut store = loaded.unwrap_or_else(|e| {
        eprintln!("no spawns loaded from {}: {e}", spawn_dir.display());
        SpawnStore::empty(if flag("--tenx") { &doodad_overlay } else { &spawn_dir })
    });
    if flag("--tenx") {
        let spawners = arg("--spawner-overlay").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("data/local/spawner-overlay"));
        if let Err(e) = store.load_native(&world, &spawners, &db_path) {
            eprintln!("native NPC spawners unavailable for {world}: {e:#}");
        }
        let level = arg("--level-doodad-overlay").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("data/local/level-doodad-overlay"));
        let base = arg("--spawner-base").map(PathBuf::from);
        if let Err(e) = store.load_level(&world, &level, base.as_deref()) {
            eprintln!("level-pack doodads unavailable for {world}: {e:#}");
        }
    }
    let game = match GameData::open(&db_path, flag("--read-only-db")) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("cannot open the game database {}: {e}; using the shared copy read-only", db_path.display());
            GameData::open(std::path::Path::new(aa_anim::db::DEFAULT_DB), true).expect("game database")
        }
    };
    let anim_db = aa_anim::GameDb::open().map_err(|e| eprintln!("humanoid previews off: {e}")).ok().map(std::sync::Mutex::new);

    let look = vec_arg("--look").filter(|v| v.len() >= 2).map_or((0.0, -20.0), |v| (v[0], v[1]));
    let start = match vec_arg("--pos") {
        Some(v) if v.len() >= 3 => Vec3::new(v[0], v[1], v[2]),
        Some(v) => {
            let ground = aa_world::TerrainQuery::new(&world).height_at(v[0], v[1]).unwrap_or(100.0);
            Vec3::new(v[0], v[1], ground + 25.0)
        }
        // main_world: the Nuia new-character spawn, from above; else the server's start point
        None => worlds::start_camera(&world, &worlds::data_dirs(&spawn_dir)),
    };
    let mode = match arg("--mode").as_deref() {
        Some("place") => Mode::Place,
        Some("terrain") => Mode::Terrain,
        Some("objects") => Mode::Objects,
        Some("paths") => Mode::Paths,
        Some("housing") => Mode::Housing,
        _ => Mode::Select,
    };
    let editor = Editor {
        world: world.clone(),
        paths: Paths { spawn_dir, mirrors, db: db_path, overlay },
        mode,
        selected: None,
        place: None,
        browser: Table::Npcs,
        models_tab: arg("--browser").as_deref() == Some("buildings"),
        models_all: false,
        search: String::new(),
        view_radius: 220.0,
        show_npcs: true,
        show_doodads: true,
        fog: !flag("--no-fog"),
        snap_to_ground: true,
        status: "ready".into(),
        row: None,
        playing: false,
        requests: if flag("--play") { vec![Request::Play] } else { Vec::new() },
    };

    let mut plugin = AaWorldPlugin::new(&world);
    plugin.config.load_radius = 1600.0;
    plugin.config.unload_radius = 2000.0;
    let mut app = App::new();
    app.add_plugins(aa_dev::default_plugins("AABevy world editor"))
        .add_plugins(aa_dev::DevPlugin)
        .insert_resource(EguiGlobalSettings { auto_create_primary_context: false, ..default() })
        .add_plugins(EguiPlugin::default())
        // (adds aa-anim's AnimPlugin too)
        .add_plugins(plugin)
        .insert_resource(editor)
        .insert_resource(store)
        .insert_resource(game)
        .insert_resource(AnimDb(anim_db))
        .add_plugins((camera::plugin, tools::plugin, views::plugin, terrain_edit::plugin, server_terrain::plugin, ui::plugin, play::plugin, play_server::plugin))
        .add_plugins(pak_export::plugin)
        .add_plugins(world_objects::plugin)
        .add_plugins(surface_add::plugin)
        .add_plugins(objects_extra::plugin)
        .add_plugins(veg_brush::plugin)
        .add_plugins(model_preview::plugin)
        .add_plugins((nav::plugin, teleport::plugin, worlds::plugin, paths::plugin, housing::plugin))
        .add_plugins(zone_borders::plugin)
        .add_systems(Startup, move |mut commands: Commands| {
            let transform = Transform::from_translation(cry_to_bevy(start))
                .with_rotation(Quat::from_rotation_y(-look.0.to_radians()) * Quat::from_rotation_x(look.1.to_radians()));
            commands.spawn((
                Camera3d::default(),
                Projection::Perspective(PerspectiveProjection { fov: 60f32.to_radians(), near: 0.3, far: 20000.0, ..default() }),
                transform,
                camera::EditorCamera,
                PrimaryEguiContext,
            ));
        });
    if let Some((table, id)) = arg("--row").and_then(|v| {
        let (t, id) = v.split_once(':')?;
        let table = Table::ALL.into_iter().find(|x| x.label().eq_ignore_ascii_case(t))?;
        Some((table, id.parse::<u32>().ok()?))
    }) {
        app.add_systems(Startup, move |mut editor: ResMut<Editor>, game: Res<GameData>| match ui::RowEdit::open(&game, table, id) {
            Ok(r) => editor.row = Some(r),
            Err(e) => editor.status(e),
        });
    }
    if let Some(id) = arg("--select").and_then(|v| v.parse::<u32>().ok()) {
        app.add_systems(Update, move |mut editor: ResMut<Editor>, store: Res<SpawnStore>, mut done: Local<bool>| {
            if *done {
                return;
            }
            *done = true;
            let pick = store.near(start, 2000.0).filter(|&i| store.spawns[i].now.unit_id == id).min_by(|&a, &b| {
                store.spawns[a].now.pos.distance(start).total_cmp(&store.spawns[b].now.pos.distance(start))
            });
            if let Some(i) = pick {
                editor.selected = Some(i);
                editor.requests.push(Request::Focus(store.spawns[i].now.pos));
            }
        });
    }
    // `--place-npc <npc>@x,y,z[,yaw]`, `--place-doodad <template>@x,y,z[,yaw]`, `--move-doodad x,y@x,y,z`, `--move-spawner <id>@x,y,z [--spawns-save]`:
    // unattended native spawn checks.
    if flag("--tenx") && ["--place-npc", "--place-doodad", "--move-doodad", "--move-spawner"].iter().any(|a| arg(a).is_some()) {
        let (place, doodad, moved) = (arg("--place-npc"), arg("--place-doodad"), arg("--move-spawner"));
        let moved_doodad = arg("--move-doodad");
        app.add_systems(Update, move |mut editor: ResMut<Editor>, mut store: ResMut<SpawnStore>, mut done: Local<bool>| {
            if *done {
                return;
            }
            *done = true;
            let parse = |v: &str| -> Option<(u32, Vec<f32>)> {
                let (id, p) = v.split_once('@')?;
                let p: Vec<f32> = p.split(',').filter_map(|s| s.trim().parse().ok()).collect();
                (p.len() >= 3).then_some((id.parse().ok()?, p))
            };
            if let Some((npc, p)) = place.as_deref().and_then(parse) {
                let placement = spawns::Placement::new(npc, Vec3::new(p[0], p[1], p[2]), p.get(3).copied().unwrap_or(0.0));
                match store.add(Kind::Npc, placement) {
                    Ok(i) => {
                        editor.selected = Some(i);
                        editor.status(format!("placed native NPC {npc} as spawner {}", store.spawns[i].native.map_or(0, |r| r.id)));
                    }
                    Err(e) => editor.status(format!("cannot place NPC {npc}: {e}")),
                }
            }
            if let Some((template, p)) = doodad.as_deref().and_then(parse) {
                let placement = spawns::Placement::new(template, Vec3::new(p[0], p[1], p[2]), p.get(3).copied().unwrap_or(0.0));
                match store.add(Kind::Doodad, placement) {
                    Ok(i) => {
                        editor.selected = Some(i);
                        editor.status(format!("placed doodad {template}"));
                    }
                    Err(e) => editor.status(format!("cannot place doodad {template}: {e}")),
                }
            }
            // `--move-doodad x,y@x,y,z`: the doodad nearest to x,y (within 2 m) moves
            if let Some((from, to)) = moved_doodad.as_deref().and_then(|v| v.split_once('@')) {
                let xy: Vec<f32> = from.split(',').filter_map(|s| s.trim().parse().ok()).collect();
                let to: Vec<f32> = to.split(',').filter_map(|s| s.trim().parse().ok()).collect();
                let found = (xy.len() == 2 && to.len() >= 3)
                    .then(|| {
                        store
                            .spawns
                            .iter()
                            .enumerate()
                            .filter(|(_, s)| s.kind == Kind::Doodad && !s.now.deleted)
                            .map(|(i, s)| (i, s.now.pos.truncate().distance(Vec2::new(xy[0], xy[1]))))
                            .filter(|(_, d)| *d <= 2.0)
                            .min_by(|a, b| a.1.total_cmp(&b.1))
                    })
                    .flatten();
                match found {
                    Some((i, _)) => {
                        let now = spawns::Placement { pos: Vec3::new(to[0], to[1], to[2]), ..store.spawns[i].now };
                        store.apply(vec![(i, now)]);
                        editor.status(format!("moved doodad {}", now.unit_id));
                    }
                    None => editor.status(format!("no doodad near {from}")),
                }
            }
            if let Some((id, p)) = moved.as_deref().and_then(parse) {
                match store.spawns.iter().position(|s| !s.now.deleted && s.native.is_some_and(|r| r.id == id)) {
                    Some(i) => {
                        let now = spawns::Placement { pos: Vec3::new(p[0], p[1], p[2]), ..store.spawns[i].now };
                        store.apply(vec![(i, now)]);
                        editor.status(format!("moved native spawner {id}"));
                    }
                    None => editor.status(format!("no native spawner {id}")),
                }
            }
            if flag("--spawns-save") {
                editor.requests.push(Request::SaveSpawns);
            }
        });
    }
    app.run();
}
