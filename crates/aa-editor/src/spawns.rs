//! NPC and doodad spawns: the AAEmu server's `Data/Worlds/<world>/{npc,doodad}_spawns*.json` files.
//!
//! Entry format (case-insensitive keys, `//` comments allowed, angles in degrees, Cry space: Z up):
//! `{"UnitId": 502, "Position": {"X": 23039.76, "Y": 11540.64, "Z": 252.7, "Yaw": -36.5625}}`.
//! `UnitId` is `npcs.id` / `doodad_almighties.id`. An NPC entry may name the AI route it walks: `"FollowPath": "<name>"`
//! (`Data/Path/<name>.path`, see [`crate::paths`]). The server reads every `npc_spawns*.json` /
//! `doodad_spawns*.json` of the folder, so new spawns go to `*_spawns_editor.json` and the retail
//! files are only rewritten when one of their entries was moved or deleted (untouched entries keep
//! their exact text, comments included).

use std::path::{Path, PathBuf};

use bevy::prelude::*;
use serde_json::{Map, Value};

use crate::jsonc;
use crate::level_doodads;
use crate::zone_spawners::{self, Templates, WorldZones};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Npc,
    Doodad,
}

impl Kind {
    pub fn prefix(self) -> &'static str {
        match self {
            Kind::Npc => "npc_spawns",
            Kind::Doodad => "doodad_spawns",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Kind::Npc => "NPC",
            Kind::Doodad => "Doodad",
        }
    }
}

/// The editable part of a spawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub unit_id: u32,
    /// Cry space metres.
    pub pos: Vec3,
    /// Degrees, as in the file.
    pub yaw: f32,
    pub pitch: f32,
    pub roll: f32,
    pub scale: f32,
    pub deleted: bool,
    /// `FollowPath`: index into [`SpawnStore::path_names`], 0 = none.
    pub path: u16,
}

impl Placement {
    pub fn new(unit_id: u32, pos: Vec3, yaw: f32) -> Self {
        Placement { unit_id, pos, yaw, pitch: 0.0, roll: 0.0, scale: 1.0, deleted: false, path: 0 }
    }

    /// Bevy-space rotation of a model placed with this spawn (yaw about Cry Z, counter-clockwise from north).
    pub fn rotation(&self) -> Quat {
        let cry = Quat::from_euler(EulerRot::ZYX, self.yaw.to_radians(), self.pitch.to_radians(), self.roll.to_radians());
        aa_world::coords::cry_rot_to_bevy(cry)
    }
}

pub struct Spawn {
    pub kind: Kind,
    pub file: usize,
    pub now: Placement,
    /// The entry as loaded (all fields), `None` for a new one.
    value: Option<Value>,
    /// Exact source text of an untouched loaded entry.
    source: Option<String>,
    original: Option<Placement>,
    pub native: Option<NativeRef>,
    level: Option<LevelRef>,
}

/// A level-pack doodad (see [`crate::level_doodads`]): its block in the cell file and that block's text, as loaded
/// or last saved.
#[derive(Clone, Debug)]
struct LevelRef {
    block: Option<usize>,
    text: String,
}

/// A spawner of a native zone file (see [`crate::zone_spawners`]); `Placement::unit_id` is the NPC it shows.
#[derive(Clone, Copy, Debug)]
pub struct NativeRef {
    pub id: u32,
    pub spawner_type: u32,
    /// Its block in the file as loaded or last saved.
    block: Option<usize>,
}

impl Spawn {
    pub fn changed(&self) -> bool {
        self.original != Some(self.now) && !(self.original.is_none() && self.now.deleted)
    }
}

pub struct SpawnFile {
    pub path: PathBuf,
    pub existed: bool,
    pub native: Option<NativeFile>,
    /// A cell `doodad.g` (see [`crate::level_doodads`]).
    level: Option<LevelFile>,
    /// JSON entries kept out of the editor (shown from the level pack instead) with their place in the file as
    /// loaded; written back as they are, in that place.
    kept: Vec<(usize, String)>,
    /// Line end and entry indentation of the file as loaded, kept when it is rewritten.
    style: Style,
}

pub struct LevelFile {
    cell: (i32, i32),
    parsed: level_doodads::File,
}

#[derive(Clone, Debug, PartialEq)]
struct Style {
    eol: &'static str,
    indent: String,
}

impl Default for Style {
    fn default() -> Self {
        Style { eol: "\n", indent: "    ".into() }
    }
}

impl Style {
    /// The style of a JSON array text whose first entry starts at `first`.
    fn of(text: &str, first: Option<usize>) -> Style {
        let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
        let indent = first
            .map(|s| text[..s].rsplit('\n').next().unwrap_or_default().to_string())
            .filter(|i| !i.is_empty() && i.chars().all(|c| c == ' ' || c == '\t'))
            .unwrap_or_else(|| "    ".into());
        Style { eol, indent }
    }
}

pub struct NativeFile {
    pub zone: u32,
    pub origin: Vec2,
    parsed: zone_spawners::File,
}

/// NPCs of a 10.x world come from the native zone spawner files; saves go to `overlay`.
pub struct NativeWorld {
    pub world: String,
    pub overlay: PathBuf,
    pub zones: WorldZones,
    pub templates: Templates,
    next_id: u32,
}

/// One undoable step.
struct Step {
    index: usize,
    before: Placement,
    after: Placement,
}

#[derive(Resource, Default)]
pub struct SpawnStore {
    /// The folder new spawns are saved in (`*_spawns_editor.json`).
    dir: PathBuf,
    pub files: Vec<SpawnFile>,
    pub spawns: Vec<Spawn>,
    undo: Vec<Vec<Step>>,
    redo: Vec<Vec<Step>>,
    /// Incremented on every change, so views know to refresh.
    pub generation: u64,
    /// `FollowPath` names; index 0 is "no path".
    pub path_names: Vec<String>,
    pub native: Option<NativeWorld>,
    /// Loaded with [`SpawnStore::load_layered`]: saves never touch the server files, so no `.editor-orig` backups.
    layered: bool,
    /// World and overlay of the level-pack doodads, see [`SpawnStore::load_level`].
    level_world: Option<(String, PathBuf)>,
}

fn get_ci<'a>(obj: &'a Map<String, Value>, key: &str) -> Option<&'a Value> {
    obj.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v)
}

fn set_ci(obj: &mut Map<String, Value>, key: &str, value: Value) {
    let existing = obj.keys().find(|k| k.eq_ignore_ascii_case(key)).cloned();
    obj.insert(existing.unwrap_or_else(|| key.to_string()), value);
}

fn num(obj: &Map<String, Value>, key: &str) -> Option<f64> {
    get_ci(obj, key).and_then(Value::as_f64)
}

fn round(v: f32, digits: i32) -> Value {
    let p = 10f64.powi(digits);
    serde_json::Number::from_f64((v as f64 * p).round() / p).map(Value::Number).unwrap_or(Value::Null)
}

fn placement_of(v: &Value) -> Option<Placement> {
    let obj = v.as_object()?;
    let unit_id = num(obj, "UnitId")? as u32;
    let p = get_ci(obj, "Position")?.as_object()?;
    let f = |k: &str| num(p, k).unwrap_or(0.0) as f32;
    Some(Placement {
        unit_id,
        pos: Vec3::new(f("X"), f("Y"), f("Z")),
        yaw: f("Yaw"),
        pitch: f("Pitch"),
        roll: f("Roll"),
        scale: num(obj, "Scale").map(|s| s as f32).filter(|s| *s > 0.0).unwrap_or(1.0),
        deleted: false,
        path: 0,
    })
}

impl SpawnStore {
    /// The spawn files of `dir` by lowercase name.
    fn spawn_files(dir: &Path) -> anyhow::Result<std::collections::BTreeMap<String, (Kind, PathBuf)>> {
        let mut out = std::collections::BTreeMap::new();
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()).map(str::to_ascii_lowercase) else { continue };
            if !name.ends_with(".json") || !path.is_file() {
                continue;
            }
            for kind in [Kind::Npc, Kind::Doodad] {
                if name.starts_with(kind.prefix()) {
                    out.insert(name.clone(), (kind, path.clone()));
                }
            }
        }
        Ok(out)
    }

    /// Loads every spawn file of `dir` (`.../Data/Worlds/<world>`).
    pub fn load(dir: &Path) -> anyhow::Result<SpawnStore> {
        let files = Self::spawn_files(dir)?.into_values().map(|(kind, path)| (kind, path.clone(), path)).collect();
        Self::load_files(SpawnStore::empty(dir), files)
    }

    /// Loads the spawn files of a server folder `base` that is never written: a file saved before in `overlay` replaces
    /// the server's file of the same name, and every save (new spawns too) goes to `overlay` as whole files.
    pub fn load_layered(base: &Path, overlay: &Path) -> anyhow::Result<SpawnStore> {
        let mut files = Self::spawn_files(base)?;
        if overlay.is_dir() {
            files.extend(Self::spawn_files(overlay)?);
        }
        let files = files
            .into_iter()
            .map(|(name, (kind, read))| {
                let save = overlay.join(read.file_name().map(PathBuf::from).unwrap_or_else(|| PathBuf::from(&name)));
                (kind, read, save)
            })
            .collect();
        let mut store = Self::load_files(SpawnStore::empty(overlay), files)?;
        store.layered = true;
        Ok(store)
    }

    /// `files`: kind, the file read, the file saved to.
    fn load_files(mut store: SpawnStore, files: Vec<(Kind, PathBuf, PathBuf)>) -> anyhow::Result<SpawnStore> {
        for (kind, read, path) in files {
            let text = std::fs::read_to_string(&read).map_err(|e| anyhow::anyhow!("{}: {e}", read.display()))?;
            let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
            let file = store.files.len();
            let spans = jsonc::array_elements(text).map_err(|e| anyhow::anyhow!("{}: {e}", read.display()))?;
            let style = Style::of(text, spans.first().map(|s| s.0));
            store.files.push(SpawnFile { path: path.clone(), existed: true, native: None, level: None, kept: Vec::new(), style });
            let mut bad = 0;
            for (s, e) in spans {
                let source = &text[s..e];
                let value: Option<Value> = serde_json::from_str(&jsonc::strip_comments(source)).ok();
                match value.as_ref().and_then(placement_of) {
                    Some(mut p) => {
                        let follow = value.as_ref().and_then(Value::as_object).and_then(|o| get_ci(o, "FollowPath")).and_then(Value::as_str);
                        p.path = follow.map_or(0, |n| store.path_id(n));
                        store.spawns.push(Spawn { kind, file, now: p, value, source: Some(source.to_string()), original: Some(p), native: None, level: None });
                    }
                    None => bad += 1,
                }
            }
            if bad > 0 {
                warn!("{}: {bad} entries without UnitId / Position are kept out of the editor", read.display());
            }
        }
        info!("spawns: {} from {} files, saved to {}", store.spawns.len(), store.files.len(), store.dir.display());
        Ok(store)
    }

    /// Replaces the JSON NPC spawns with the native spawners of `world`'s zones (saved edits from `overlay`, else the pak).
    pub fn load_native(&mut self, world: &str, overlay: &Path, db: &Path) -> anyhow::Result<()> {
        let pak = aa_pak::Pak::global();
        let xml = String::from_utf8(pak.read(&format!("worlds/{world}/world.xml"))?)?;
        // `--spawner-base <game dir>`: the server's loose files (what its zone hosts run) before the pak's copy
        let base = crate::arg("--spawner-base").map(std::path::PathBuf::from);
        let read = |name: &str| -> anyhow::Result<Option<String>> {
            if let Some(file) = base.as_ref().map(|b| b.join(name.trim_start_matches("game/"))).filter(|f| f.is_file()) {
                return Ok(Some(std::fs::read_to_string(file)?));
            }
            match pak.read(name) {
                Ok(bytes) => Ok(Some(String::from_utf8(bytes)?)),
                Err(_) => Ok(None),
            }
        };
        self.load_native_with(world, overlay, WorldZones::parse(&xml)?, Templates::load(db)?, read)
    }

    fn load_native_with(
        &mut self,
        world: &str,
        overlay: &Path,
        zones: WorldZones,
        templates: Templates,
        read: impl Fn(&str) -> anyhow::Result<Option<String>>,
    ) -> anyhow::Result<()> {
        let json = self.spawns.iter().filter(|s| s.kind == Kind::Npc).count();
        if json > 0 {
            warn!("{json} JSON NPC spawns are not read by the native 10.x server and stay out of the editor");
        }
        self.spawns.retain(|s| s.kind != Kind::Npc);
        let (mut max_id, mut fixed) = (0, 0);
        for zone in &zones.zones {
            let name = zone_spawners::file_name(world, zone.id);
            let path = overlay.join(&name);
            let text = if path.is_file() {
                std::fs::read_to_string(&path)?
            } else {
                match read(&name)? {
                    Some(text) => text,
                    None => continue,
                }
            };
            let parsed = zone_spawners::parse(&text).map_err(|e| anyhow::anyhow!("{name}: {e:#}"))?;
            let file = self.files.len();
            for (block, b) in parsed.blocks.iter().enumerate() {
                max_id = max_id.max(b.id);
                let Some((local, rot)) = b.point else {
                    fixed += 1;
                    continue;
                };
                let npc = templates.npc_of.get(&b.spawner_type).copied().unwrap_or(0);
                let p = Placement::new(npc, local + zone.origin.extend(0.0), rot.to_degrees());
                let native = Some(NativeRef { id: b.id, spawner_type: b.spawner_type, block: Some(block) });
                self.spawns.push(Spawn { kind: Kind::Npc, file, now: p, value: None, source: None, original: Some(p), native, level: None });
            }
            self.files.push(SpawnFile { path, existed: true, native: Some(NativeFile { zone: zone.id, origin: zone.origin, parsed }), level: None, kept: Vec::new(), style: default() });
        }
        info!(
            "native spawners: {} editable from {} zone files of {world} ({fixed} area/path spawners kept as they are)",
            self.spawns.iter().filter(|s| s.native.is_some()).count(),
            self.files.iter().filter(|f| f.native.is_some()).count()
        );
        self.native = Some(NativeWorld { world: world.to_string(), overlay: overlay.to_path_buf(), zones, templates, next_id: max_id + 1 });
        self.generation += 1;
        Ok(())
    }

    fn native_file(&mut self, zone: u32) -> anyhow::Result<usize> {
        if let Some(i) = self.files.iter().position(|f| f.native.as_ref().is_some_and(|n| n.zone == zone)) {
            return Ok(i);
        }
        let n = self.native.as_ref().ok_or_else(|| anyhow::anyhow!("no native world"))?;
        let origin = n.zones.get(zone).ok_or_else(|| anyhow::anyhow!("unknown zone {zone}"))?.origin;
        let path = n.overlay.join(zone_spawners::file_name(&n.world, zone));
        let parsed = zone_spawners::File { eol: "\r\n", ..default() };
        self.files.push(SpawnFile { path, existed: false, native: Some(NativeFile { zone, origin, parsed }), level: None, kept: Vec::new(), style: default() });
        Ok(self.files.len() - 1)
    }

    /// Adds the level-pack doodads of `world` (`cells/*/doodad.g`): saved edits from `overlay`, else the server's loose
    /// `base` game folder, else the pak. Retail `doodad_spawns.json` rows of templates the cells carry are kept out of
    /// the editor (the World only uses that file for templates no cell has) and written back unchanged.
    pub fn load_level(&mut self, world: &str, overlay: &Path, base: Option<&Path>) -> anyhow::Result<()> {
        let prefix = format!("worlds/{world}/level_design/cells/");
        fn from_dir(dir: &Path, prefix: &str, names: &mut std::collections::BTreeSet<String>) -> anyhow::Result<()> {
            if dir.is_dir() {
                for entry in std::fs::read_dir(dir)? {
                    let entry = entry?;
                    if entry.path().join("doodad.g").is_file() {
                        names.insert(format!("{prefix}{}/doodad.g", entry.file_name().to_string_lossy()));
                    }
                }
            }
            Ok(())
        }
        let mut names = std::collections::BTreeSet::new();
        match base.map(|b| b.join(&prefix)).filter(|d| d.is_dir()) {
            Some(dir) => from_dir(&dir, &prefix, &mut names)?,
            None => names.extend(aa_pak::Pak::global().list(&prefix).filter(|n| n.ends_with("/doodad.g")).map(str::to_string)),
        }
        from_dir(&overlay.join(&prefix), &prefix, &mut names)?;
        let read = |name: &str| -> anyhow::Result<Option<String>> {
            let saved = overlay.join(name);
            if saved.is_file() {
                return Ok(Some(std::fs::read_to_string(saved)?));
            }
            if let Some(file) = base.map(|b| b.join(name)).filter(|f| f.is_file()) {
                return Ok(Some(std::fs::read_to_string(file)?));
            }
            match aa_pak::Pak::global().read(name) {
                Ok(bytes) => Ok(Some(String::from_utf8(bytes)?)),
                Err(_) => Ok(None),
            }
        };
        self.load_level_with(world, overlay, names, read)
    }

    fn load_level_with(
        &mut self,
        world: &str,
        overlay: &Path,
        names: impl IntoIterator<Item = String>,
        read: impl Fn(&str) -> anyhow::Result<Option<String>>,
    ) -> anyhow::Result<()> {
        let mut types = std::collections::HashSet::new();
        let (mut cells, mut count) = (0, 0);
        for name in names {
            let Some(cell) = level_doodads::cell_of_name(&name) else { continue };
            let Some(text) = read(&name)? else { continue };
            let parsed = level_doodads::parse(&text).map_err(|e| anyhow::anyhow!("{name}: {e:#}"))?;
            let file = self.files.len();
            let origin = level_doodads::origin(cell).extend(0.0);
            for (bi, b) in parsed.blocks.iter().enumerate() {
                types.insert(b.type_id);
                let (yaw, pitch, roll) = level_doodads::angles(b.ori);
                let p = Placement { unit_id: b.type_id, pos: b.local + origin, yaw, pitch, roll, scale: b.scale, deleted: false, path: 0 };
                let level = Some(LevelRef { block: Some(bi), text: b.text.clone() });
                self.spawns.push(Spawn { kind: Kind::Doodad, file, now: p, value: None, source: None, original: Some(p), native: None, level });
                count += 1;
            }
            self.files.push(SpawnFile { path: overlay.join(&name), existed: true, native: None, level: Some(LevelFile { cell, parsed }), kept: Vec::new(), style: default() });
            cells += 1;
        }
        let retail: Vec<usize> = (0..self.files.len())
            .filter(|&i| {
                let f = &self.files[i];
                f.level.is_none() && f.native.is_none() && f.path.file_name().is_some_and(|n| n.eq_ignore_ascii_case("doodad_spawns.json"))
            })
            .collect();
        let mut kept = Vec::new();
        let mut ordinal = std::collections::HashMap::new();
        self.spawns.retain(|s| {
            let at = ordinal.entry(s.file).or_insert(0usize);
            *at += 1;
            let hide = s.kind == Kind::Doodad && s.level.is_none() && retail.contains(&s.file) && types.contains(&s.now.unit_id) && s.source.is_some();
            if hide {
                kept.push((s.file, (*at - 1, s.source.clone().unwrap_or_default())));
            }
            !hide
        });
        let hidden = kept.len();
        for (file, entry) in kept {
            self.files[file].kept.push(entry);
        }
        info!("level doodads: {count} from {cells} cell files of {world}; {hidden} retail JSON rows of the same templates hidden");
        self.level_world = Some((world.to_string(), overlay.to_path_buf()));
        self.generation += 1;
        Ok(())
    }

    fn level_file(&mut self, cell: (i32, i32)) -> anyhow::Result<usize> {
        if let Some(i) = self.files.iter().position(|f| f.level.as_ref().is_some_and(|l| l.cell == cell)) {
            return Ok(i);
        }
        let (world, overlay) = self.level_world.as_ref().ok_or_else(|| anyhow::anyhow!("no level doodads loaded"))?;
        let path = overlay.join(level_doodads::file_name(world, cell));
        let parsed = level_doodads::File { eol: "\r\n", ..default() };
        self.files.push(SpawnFile { path, existed: false, native: None, level: Some(LevelFile { cell, parsed }), kept: Vec::new(), style: default() });
        Ok(self.files.len() - 1)
    }

    /// Rewrites the cell `doodad.g` files with changes. A doodad moved into another cell leaves its old file and is
    /// added to that cell's file. Nothing is written if any file fails to parse back.
    fn save_level(&mut self) -> anyhow::Result<Vec<PathBuf>> {
        if self.level_world.is_none() {
            return Ok(Vec::new());
        }
        let n = self.spawns.len();
        let mut target: Vec<Option<usize>> = vec![None; n];
        let mut dirty = std::collections::BTreeSet::new();
        for i in 0..n {
            let s = &self.spawns[i];
            let Some(r) = &s.level else { continue };
            let (changed, deleted, file, pos, had_block) = (s.changed(), s.now.deleted, s.file, s.now.pos, r.block.is_some());
            if !changed {
                target[i] = (!deleted && had_block).then_some(file);
                continue;
            }
            if had_block {
                dirty.insert(file);
            }
            if !deleted {
                let fi = self.level_file(level_doodads::cell_at(pos))?;
                target[i] = Some(fi);
                dirty.insert(fi);
            }
        }
        if dirty.is_empty() {
            return Ok(Vec::new());
        }
        let mut at = std::collections::HashMap::new();
        for (i, s) in self.spawns.iter().enumerate() {
            if let Some(LevelRef { block: Some(b), .. }) = &s.level {
                at.insert((s.file, *b), i);
            }
        }
        let render = |s: &Spawn, cell: (i32, i32)| -> String {
            let r = s.level.as_ref().expect("level doodad");
            let p = s.now;
            let local = p.pos - level_doodads::origin(cell).extend(0.0);
            let turned = s.original.is_none_or(|o| (o.yaw, o.pitch, o.roll) != (p.yaw, p.pitch, p.roll));
            let scaled = s.original.is_none_or(|o| o.scale != p.scale);
            level_doodads::rewrite(&r.text, local, turned.then(|| level_doodads::quat(p.yaw, p.pitch, p.roll)), scaled.then_some(p.scale))
        };
        let mut texts = Vec::new();
        for &fi in &dirty {
            let lf = self.files[fi].level.as_ref().expect("level file");
            let (mut blocks, mut order) = (Vec::new(), Vec::new());
            for (bi, b) in lf.parsed.blocks.iter().enumerate() {
                match at.get(&(fi, bi)) {
                    None => {
                        blocks.push(b.text.clone());
                        order.push(None);
                    }
                    Some(&i) if target[i] == Some(fi) => {
                        let s = &self.spawns[i];
                        blocks.push(if s.changed() { render(s, lf.cell) } else { b.text.clone() });
                        order.push(Some(i));
                    }
                    Some(_) => {}
                }
            }
            for (i, s) in self.spawns.iter().enumerate() {
                let Some(r) = &s.level else { continue };
                if target[i] == Some(fi) && (r.block.is_none() || s.file != fi) {
                    blocks.push(render(s, lf.cell));
                    order.push(Some(i));
                }
            }
            let text = level_doodads::write(&lf.parsed, blocks);
            let parsed = level_doodads::parse(&text).map_err(|e| anyhow::anyhow!("{}: {e:#}", self.files[fi].path.display()))?;
            anyhow::ensure!(parsed.blocks.len() == order.len(), "{}: block count changed while saving", self.files[fi].path.display());
            texts.push((fi, text, parsed, order));
        }
        for (fi, text, _, _) in &texts {
            let path = &self.files[*fi].path;
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(path.with_extension("g.editor-tmp"), text)?;
        }
        let mut written = Vec::new();
        for (fi, _, parsed, order) in texts {
            let path = self.files[fi].path.clone();
            std::fs::rename(path.with_extension("g.editor-tmp"), &path)?;
            for (k, i) in order.iter().enumerate() {
                if let Some(i) = *i {
                    let s = &mut self.spawns[i];
                    s.file = fi;
                    s.level = Some(LevelRef { block: Some(k), text: parsed.blocks[k].text.clone() });
                }
            }
            let f = &mut self.files[fi];
            f.existed = true;
            f.level.as_mut().expect("level file").parsed = parsed;
            written.push(path);
        }
        for s in self.spawns.iter_mut() {
            if s.level.is_none() || !s.changed() {
                continue;
            }
            if s.now.deleted {
                if let Some(r) = s.level.as_mut() {
                    r.block = None;
                }
                s.original = None;
            } else {
                s.original = Some(s.now);
            }
        }
        Ok(written)
    }

    /// A store without spawns that saves new ones into `dir`.
    pub fn empty(dir: &Path) -> SpawnStore {
        SpawnStore { dir: dir.to_path_buf(), path_names: vec![String::new()], ..default() }
    }

    /// The folder JSON spawn saves go to.
    pub fn save_dir(&self) -> &Path {
        &self.dir
    }

    /// The index of a `FollowPath` name ("" is 0, no path).
    pub fn path_id(&mut self, name: &str) -> u16 {
        let name = name.trim();
        if let Some(i) = self.path_names.iter().position(|n| n == name) {
            return i as u16;
        }
        self.path_names.push(name.to_string());
        (self.path_names.len() - 1) as u16
    }

    /// The `FollowPath` of a placement.
    pub fn path_of(&self, p: &Placement) -> Option<&str> {
        self.path_names.get(p.path as usize).map(String::as_str).filter(|n| !n.is_empty())
    }

    fn editor_file(&mut self, kind: Kind) -> usize {
        // The World loads extra doodad files only when the name contains "overlay" (MapDoodadSource LevelPack).
        let suffix = if kind == Kind::Doodad { "_editor_overlay" } else { "_editor" };
        let path = self.dir.join(format!("{}{suffix}.json", kind.prefix()));
        match self.files.iter().position(|f| f.path == path) {
            Some(i) => i,
            None => {
                self.files.push(SpawnFile { path, existed: false, native: None, level: None, kept: Vec::new(), style: default() });
                self.files.len() - 1
            }
        }
    }

    pub fn add(&mut self, kind: Kind, placement: Placement) -> Result<usize, String> {
        let (file, native) = match (&mut self.native, kind) {
            (None, _) => (self.editor_file(kind), None),
            (Some(_), Kind::Doodad) if !self.layered => {
                return Err("doodads of a native 10.x world are saved only over the World's spawn files (--tenx)".into());
            }
            (Some(_), Kind::Doodad) => (self.editor_file(kind), None),
            (Some(n), Kind::Npc) => {
                let spawner_type = *n
                    .templates
                    .by_npc
                    .get(&placement.unit_id)
                    .ok_or_else(|| format!("NPC {} has no always-active single-NPC spawner template", placement.unit_id))?;
                let zone = n.zones.owner(placement.pos).ok_or("no native zone owns this position")?.id;
                let id = n.next_id;
                n.next_id += 1;
                (self.native_file(zone).map_err(|e| e.to_string())?, Some(NativeRef { id, spawner_type, block: None }))
            }
        };
        let index = self.spawns.len();
        self.spawns.push(Spawn { kind, file, now: Placement { deleted: true, ..placement }, value: None, source: None, original: None, native, level: None });
        self.apply(vec![(index, placement)]);
        Ok(index)
    }

    /// Changes spawns, as one undo step.
    pub fn apply(&mut self, changes: Vec<(usize, Placement)>) {
        let mut steps = Vec::new();
        for (index, after) in changes {
            let Some(s) = self.spawns.get_mut(index) else { continue };
            if s.now == after {
                continue;
            }
            steps.push(Step { index, before: s.now, after });
            s.now = after;
        }
        if !steps.is_empty() {
            self.undo.push(steps);
            self.redo.clear();
            self.generation += 1;
        }
    }

    /// Like [`SpawnStore::apply`], merged into the previous undo step (a drag in progress).
    pub fn apply_continued(&mut self, changes: Vec<(usize, Placement)>) {
        let Some(mut last) = self.undo.pop() else { return self.apply(changes) };
        for (index, after) in changes {
            let Some(s) = self.spawns.get_mut(index) else { continue };
            match last.iter_mut().find(|st| st.index == index) {
                Some(st) => st.after = after,
                None => last.push(Step { index, before: s.now, after }),
            }
            s.now = after;
        }
        self.undo.push(last);
        self.generation += 1;
    }

    pub fn undo(&mut self) -> bool {
        let Some(steps) = self.undo.pop() else { return false };
        for st in steps.iter().rev() {
            self.spawns[st.index].now = st.before;
        }
        self.redo.push(steps);
        self.generation += 1;
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(steps) = self.redo.pop() else { return false };
        for st in &steps {
            self.spawns[st.index].now = st.after;
        }
        self.undo.push(steps);
        self.generation += 1;
        true
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn changed_count(&self) -> usize {
        self.spawns.iter().filter(|s| s.changed()).count()
    }

    /// Live spawns within `radius` metres (horizontal) of `center` (cry space).
    pub fn near(&self, center: Vec3, radius: f32) -> impl Iterator<Item = usize> + '_ {
        let r2 = radius * radius;
        self.spawns
            .iter()
            .enumerate()
            .filter(move |(_, s)| !s.now.deleted && s.now.pos.truncate().distance_squared(center.truncate()) <= r2)
            .map(|(i, _)| i)
    }

    /// The JSON text of one entry.
    fn entry_text(spawn: &Spawn, name: Option<&str>, path_names: &[String], style: &Style) -> String {
        if spawn.original == Some(spawn.now)
            && let Some(src) = &spawn.source {
                return src.clone();
            }
        let mut obj = spawn.value.as_ref().and_then(Value::as_object).cloned().unwrap_or_default();
        let p = spawn.now;
        set_ci(&mut obj, "UnitId", Value::from(p.unit_id));
        let mut pos = get_ci(&obj, "Position").and_then(Value::as_object).cloned().unwrap_or_default();
        set_ci(&mut pos, "X", round(p.pos.x, 3));
        set_ci(&mut pos, "Y", round(p.pos.y, 3));
        set_ci(&mut pos, "Z", round(p.pos.z, 3));
        for (key, v) in [("Yaw", p.yaw), ("Pitch", p.pitch), ("Roll", p.roll)] {
            if v != 0.0 || get_ci(&pos, key).is_some() {
                set_ci(&mut pos, key, round(v, 4));
            }
        }
        set_ci(&mut obj, "Position", Value::Object(pos));
        // a stored scale of 0 or less means the default (1), kept as it is unless the scale was changed
        let default_kept = p.scale == 1.0 && num(&obj, "Scale").is_some_and(|s| s <= 0.0);
        if spawn.kind == Kind::Doodad && !default_kept && (p.scale != 1.0 || get_ci(&obj, "Scale").is_some()) {
            set_ci(&mut obj, "Scale", round(p.scale, 4));
        }
        match path_names.get(p.path as usize).filter(|n| !n.is_empty()) {
            Some(name) => set_ci(&mut obj, "FollowPath", Value::from(name.as_str())),
            None => {
                if let Some(k) = obj.keys().find(|k| k.eq_ignore_ascii_case("FollowPath")).cloned() {
                    obj.remove(&k);
                }
            }
        }
        let mut text = Vec::new();
        let mut ser = serde_json::Serializer::with_formatter(&mut text, serde_json::ser::PrettyFormatter::with_indent(style.indent.as_bytes()));
        serde::Serialize::serialize(&Value::Object(obj), &mut ser).expect("json value serialises");
        let text = String::from_utf8(text).unwrap_or_default();
        let mut out = String::new();
        for (i, line) in text.lines().enumerate() {
            if i > 0 {
                out.push_str(style.eol);
                out.push_str(&style.indent);
            }
            out.push_str(line);
            if let Some(name) = name.filter(|_| line.trim_start().to_ascii_lowercase().starts_with("\"unitid\"")) {
                out.push_str(&format!(" // {}", name.replace(['\n', '\r'], " ")));
            }
        }
        out
    }

    /// Writes every file that has changes into `dir` (normally [`SpawnStore::dir`]) and copies the written files to
    /// each of `mirrors`. A retail file overwritten for the first time is kept as `<name>.editor-orig`.
    /// Returns the written paths.
    pub fn save(&mut self, mirrors: &[PathBuf], name_of: impl Fn(Kind, u32) -> Option<String>) -> anyhow::Result<Vec<PathBuf>> {
        let mut written = self.save_native()?;
        written.extend(self.save_level()?);
        for (fi, file) in self.files.iter().enumerate() {
            if file.native.is_some() || file.level.is_some() {
                continue;
            }
            let entries: Vec<&Spawn> = self.spawns.iter().filter(|s| s.file == fi).collect();
            if !entries.iter().any(|s| s.changed()) {
                continue;
            }
            let mut live = Vec::new();
            let mut kept = file.kept.iter().peekable();
            for s in entries.iter().filter(|s| !s.now.deleted) {
                while let Some((_, text)) = kept.next_if(|(at, _)| *at <= live.len()) {
                    live.push(text.clone());
                }
                live.push(Self::entry_text(s, name_of(s.kind, s.now.unit_id).as_deref(), &self.path_names, &file.style));
            }
            live.extend(kept.map(|(_, text)| text.clone()));
            let (eol, indent) = (file.style.eol, &file.style.indent);
            let text = if live.is_empty() {
                format!("[{eol}]{eol}")
            } else {
                format!("[{eol}{indent}{}{eol}]{eol}", live.join(&format!(",{eol}{indent}")))
            };
            if let Some(dir) = file.path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            if file.path.exists() && !self.layered {
                let backup = file.path.with_extension("json.editor-orig");
                if !backup.exists() {
                    std::fs::copy(&file.path, &backup)?;
                }
            }
            let tmp = file.path.with_extension("json.editor-tmp");
            std::fs::write(&tmp, &text)?;
            std::fs::rename(&tmp, &file.path)?;
            for mirror in mirrors {
                if mirror.is_dir() {
                    let target = mirror.join(file.path.file_name().unwrap());
                    if target.exists() {
                        let backup = target.with_extension("json.editor-orig");
                        if !backup.exists() {
                            std::fs::copy(&target, &backup)?;
                        }
                    }
                    std::fs::write(&target, &text)?;
                    written.push(target);
                } else {
                    warn!("spawn mirror {} is not a folder; skipped", mirror.display());
                }
            }
            written.push(file.path.clone());
        }
        // the saved state is the new baseline
        for s in &mut self.spawns {
            if s.changed() {
                s.source = None;
                s.original = (!s.now.deleted).then_some(s.now);
            }
        }
        for f in &mut self.files {
            if f.path.exists() {
                f.existed = true;
            }
        }
        Ok(written)
    }

    /// Rewrites the native zone files with changes. A spawner belongs to the zone owning its position, so one moved
    /// across a zone border leaves its old file and keeps its id in the new one. Nothing is written if any change
    /// cannot be placed.
    fn save_native(&mut self) -> anyhow::Result<Vec<PathBuf>> {
        let Some(n) = &self.native else { return Ok(Vec::new()) };
        let mut plan: Vec<Option<(u32, u32)>> = vec![None; self.spawns.len()];
        for (i, s) in self.spawns.iter().enumerate() {
            let Some(r) = s.native else { continue };
            if !s.changed() || s.now.deleted {
                continue;
            }
            let p = s.now;
            let zone = n
                .zones
                .owner(p.pos)
                .ok_or_else(|| anyhow::anyhow!("NPC {} at {:.1}, {:.1} is outside every native zone", p.unit_id, p.pos.x, p.pos.y))?
                .id;
            let spawner_type = if s.original.is_some_and(|o| o.unit_id == p.unit_id) {
                r.spawner_type
            } else {
                *n.templates
                    .by_npc
                    .get(&p.unit_id)
                    .ok_or_else(|| anyhow::anyhow!("NPC {} has no always-active single-NPC spawner template", p.unit_id))?
            };
            plan[i] = Some((zone, spawner_type));
        }
        let mut target: Vec<Option<usize>> = vec![None; self.spawns.len()];
        let mut dirty = std::collections::BTreeSet::new();
        for i in 0..self.spawns.len() {
            let s = &self.spawns[i];
            let Some(r) = s.native else { continue };
            if !s.changed() {
                target[i] = (!s.now.deleted).then_some(s.file);
                continue;
            }
            if r.block.is_some() {
                dirty.insert(s.file);
            }
            if let Some((zone, _)) = plan[i] {
                let fi = self.native_file(zone)?;
                target[i] = Some(fi);
                dirty.insert(fi);
            }
        }
        let mut at = std::collections::HashMap::new();
        for (i, s) in self.spawns.iter().enumerate() {
            if let Some(block) = s.native.and_then(|r| r.block) {
                at.insert((s.file, block), i);
            }
        }
        let mut texts = Vec::new();
        for &fi in &dirty {
            let nf = self.files[fi].native.as_ref().expect("native file");
            let local = |p: Placement| p.pos - nf.origin.extend(0.0);
            let mut blocks = Vec::new();
            for (bi, b) in nf.parsed.blocks.iter().enumerate() {
                match at.get(&(fi, bi)) {
                    None => blocks.push(b.text.clone()),
                    Some(&i) if target[i] == Some(fi) => match plan[i] {
                        Some((_, t)) => blocks.push(zone_spawners::rewrite(&b.text, t, local(self.spawns[i].now), self.spawns[i].now.yaw.to_radians())),
                        None => blocks.push(b.text.clone()),
                    },
                    Some(_) => {}
                }
            }
            for (i, s) in self.spawns.iter().enumerate() {
                let Some(r) = s.native else { continue };
                if target[i] == Some(fi)
                    && (r.block.is_none() || s.file != fi)
                    && let Some((_, t)) = plan[i]
                {
                    blocks.push(zone_spawners::new_block(r.id, t, local(s.now), s.now.yaw.to_radians(), nf.parsed.eol));
                }
            }
            let text = zone_spawners::write(&nf.parsed, blocks);
            let parsed = zone_spawners::parse(&text)?;
            texts.push((fi, text, parsed));
        }
        for (fi, text, _) in &texts {
            let path = &self.files[*fi].path;
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(path.with_extension("g.editor-tmp"), text)?;
        }
        let mut written = Vec::new();
        for (fi, _, parsed) in texts {
            let path = self.files[fi].path.clone();
            std::fs::rename(path.with_extension("g.editor-tmp"), &path)?;
            self.files[fi].existed = true;
            self.files[fi].native.as_mut().expect("native file").parsed = parsed;
            written.push(path);
        }
        for i in 0..self.spawns.len() {
            let Some(mut r) = self.spawns[i].native else { continue };
            let changed = self.spawns[i].changed();
            if let Some((_, t)) = plan[i] {
                r.spawner_type = t;
            }
            match target[i] {
                Some(fi) => {
                    r.block = self.files[fi].native.as_ref().expect("native file").parsed.blocks.iter().position(|b| b.id == r.id);
                    self.spawns[i].file = fi;
                }
                None => r.block = None,
            }
            let s = &mut self.spawns[i];
            s.native = Some(r);
            if changed {
                s.original = (!s.now.deleted).then_some(s.now);
            }
        }
        Ok(written)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_doodads_move_across_cells_delete_and_keep_hidden_json() {
        let root = std::env::temp_dir().join(format!("aa-level-doodads-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (base, overlay) = (root.join("server"), root.join("overlay"));
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(
            base.join("doodad_spawns.json"),
            "[\n  {\"UnitId\": 2768, \"Position\": {\"X\": 1174, \"Y\": 1573, \"Z\": 100}},\n  {\"UnitId\": 9000, \"Position\": {\"X\": 5, \"Y\": 6, \"Z\": 7}}\n]\n",
        )
        .unwrap();
        let cell = "doodad\r\n    category 17\r\n    type 2768\r\n    family 0\r\n    pos ( x 150, y 549, z 100 )\r\n    ori ( x 0, y 0, z 0, w 1 )\r\n    scale 1\r\ndoodad\r\n    category 17\r\n    type 3484\r\n    family 0\r\n    pos ( x 10, y 20, z 0 )\r\n    ori ( x 0, y 0, z 0, w 1 )\r\n    scale 1\r\n";
        let name = |c: (i32, i32)| level_doodads::file_name("w", c);
        let mut store = SpawnStore::load_layered(&base, &overlay).unwrap();
        let cell_text = cell.to_string();
        store.load_level_with("w", &overlay, [name((1, 1))], |n| Ok((n == name((1, 1))).then(|| cell_text.clone()))).unwrap();
        // the retail JSON row of 2768 is shown from the cell instead; 9000 (no cell has it) stays
        let json: Vec<u32> = store.spawns.iter().filter(|s| s.level.is_none()).map(|s| s.now.unit_id).collect();
        assert_eq!(json, vec![9000]);
        let moved = store.spawns.iter().position(|s| s.now.unit_id == 2768 && s.level.is_some()).unwrap();
        let gone = store.spawns.iter().position(|s| s.now.unit_id == 3484).unwrap();
        assert_eq!(store.spawns[moved].now.pos, Vec3::new(1174.0, 1573.0, 100.0));
        let mut p = store.spawns[moved].now;
        p.pos.x = 2048.0 + 30.0;
        let mut d = store.spawns[gone].now;
        d.deleted = true;
        let json_row = store.spawns.iter().position(|s| s.now.unit_id == 9000).unwrap();
        let mut j = store.spawns[json_row].now;
        j.pos.x = 50.0;
        store.apply(vec![(moved, p), (gone, d), (json_row, j)]);
        store.save(&[], |_, _| None).unwrap();

        let old = std::fs::read_to_string(overlay.join(name((1, 1)))).unwrap();
        assert_eq!(old, "");
        let new = std::fs::read_to_string(overlay.join(name((2, 1)))).unwrap();
        assert!(new.contains("    category 17\r\n    type 2768\r\n") && new.contains("    pos ( x 30, y 549, z 100 )\r\n"), "{new}");
        assert!(new.contains("    ori ( x 0, y 0, z 0, w 1 )\r\n"), "{new}");
        let json = std::fs::read_to_string(overlay.join("doodad_spawns.json")).unwrap();
        assert!(json.find("2768").unwrap() < json.find("9000").unwrap(), "hidden row kept in its place: {json}");

        let mut again = SpawnStore::empty(&overlay);
        let names = [name((1, 1)), name((2, 1))];
        let read = |n: &str| Ok(std::fs::read_to_string(overlay.join(n)).ok());
        again.load_level_with("w", &overlay, names, read).unwrap();
        assert_eq!(again.spawns.len(), 1);
        assert_eq!(again.spawns[0].now.pos, Vec3::new(2078.0, 1573.0, 100.0));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn layered_doodads_save_whole_files_to_the_overlay_only() {
        let root = std::env::temp_dir().join(format!("aa-layered-doodads-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (base, overlay) = (root.join("server"), root.join("overlay"));
        std::fs::create_dir_all(&base).unwrap();
        let retail = r#"[
  {
    "Id": 0,
    "UnitId": 320,
    "Title": "Mailbox",
    "Position": { "X": 1.5, "Y": 2.5, "Z": 3.0, "Roll": 0.0, "Pitch": 0.0, "Yaw": 0.0 },
    "funcGroupId": 0,
    "Scale": 0.0
  },
  {
    "Id": 0,
    "UnitId": 2319,
    "Title": "Lamp",
    "Position": { "X": 10.0, "Y": 20.0, "Z": 30.0, "Roll": 0.0, "Pitch": 0.0, "Yaw": 0.0 },
    "funcGroupId": 0,
    "Scale": 0.0
  }
]"#;
        let retail = retail.replace('\n', "\r\n") + "\r\n";
        std::fs::write(base.join("doodad_spawns.json"), &retail).unwrap();
        let mut store = SpawnStore::load_layered(&base, &overlay).unwrap();
        assert_eq!(store.spawns.len(), 2);
        let mut p = store.spawns[1].now;
        p.pos.x = 11.0;
        store.apply(vec![(1, p)]);
        store.add(Kind::Doodad, Placement::new(320, Vec3::new(5.0, 6.0, 7.0), 90.0)).unwrap();
        let written = store.save(&[], |_, _| None).unwrap();
        assert_eq!(written.len(), 2);
        assert_eq!(std::fs::read_to_string(base.join("doodad_spawns.json")).unwrap(), retail);
        assert_eq!(std::fs::read_dir(&base).unwrap().count(), 1);
        assert!(!overlay.join("doodad_spawns.json.editor-orig").exists());
        let saved = std::fs::read_to_string(overlay.join("doodad_spawns.json")).unwrap();
        assert!(saved.contains("\"Title\": \"Mailbox\"") && saved.contains("11.0") && saved.contains("\"funcGroupId\": 0"), "{saved}");
        assert!(!saved.contains("\"Scale\": 1"), "{saved}");
        assert!(saved.starts_with(&retail[..retail.find("\"Lamp\"").unwrap()]), "{saved}");
        assert!(!saved.replace("\r\n", "").contains('\n') && saved.contains("\r\n  {\r\n    \"Id\": 0"), "{saved}");
        let reloaded = SpawnStore::load_layered(&base, &overlay).unwrap();
        assert_eq!(reloaded.spawns.len(), 3);
        assert!(reloaded.spawns.iter().any(|s| s.now.pos.x == 11.0));
        assert_eq!(reloaded.save_dir(), overlay.as_path());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn native_spawners_move_between_zone_files_and_keep_untouched_blocks() {
        let dir = std::env::temp_dir().join(format!("aa-native-spawners-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let xml = r#"<World><ZoneList>
            <Zone name="a" id="1" originX="1" originY="0"><cellList><cell x="1" y="0"><sectorList><sector x="0" y="0"/></sectorList></cell></cellList></Zone>
            <Zone name="b" id="2" originX="1" originY="0"><cellList><cell x="1" y="0"><sectorList><sector x="1" y="0"/></sectorList></cell></cellList></Zone>
        </ZoneList></World>"#;
        let area = "spawner
    spawnerId 30
    spawnAreaType area
    spawnerType 9000
    points
        point
            pos ( x 1, y 1, z 1 )
            zRot 0
        point
            pos ( x 2, y 2, z 2 )
            zRot 0
";
        let one = format!(
            "{}{}{area}",
            crate::zone_spawners::new_block(10, 9000, Vec3::new(5.0, 5.0, 1.0), 0.0, "
"),
            crate::zone_spawners::new_block(20, 9000, Vec3::new(6.0, 6.0, 1.0), 0.0, "
")
        );
        let templates = || {
            let mut t = Templates::default();
            t.by_npc.extend([(500, 9000), (600, 9100)]);
            t.npc_of.extend([(9000, 500), (9100, 600)]);
            t
        };
        let read = |name: &str| Ok((name == crate::zone_spawners::file_name("w", 1)).then(|| one.clone()));
        let mut store = SpawnStore::empty(&dir);
        store.load_native_with("w", &dir, WorldZones::parse(xml).unwrap(), templates(), read).unwrap();
        assert_eq!(store.spawns.len(), 2);
        let origin = Vec3::new(1024.0, 0.0, 0.0);
        assert_eq!((store.spawns[0].now.unit_id, store.spawns[0].now.pos), (500, origin + Vec3::new(5.0, 5.0, 1.0)));
        assert!(store.add(Kind::Doodad, Placement::new(1, origin, 0.0)).is_err());
        assert!(store.add(Kind::Npc, Placement::new(777, origin, 0.0)).is_err());
        assert!(store.add(Kind::Npc, Placement::new(600, Vec3::new(10.0, 10.0, 0.0), 0.0)).is_err());
        let added = store.add(Kind::Npc, Placement::new(600, origin + Vec3::new(10.0, 10.0, 0.0), 90.0)).unwrap();
        store.apply(vec![
            (0, Placement { pos: origin + Vec3::new(7.5, 5.0, 1.0), ..store.spawns[0].now }),
            (1, Placement { pos: origin + Vec3::new(100.0, 6.0, 1.0), ..store.spawns[1].now }),
        ]);
        store.save(&[], |_, _| None).unwrap();
        let a = std::fs::read_to_string(dir.join(crate::zone_spawners::file_name("w", 1))).unwrap();
        let b = std::fs::read_to_string(dir.join(crate::zone_spawners::file_name("w", 2))).unwrap();
        let fa = crate::zone_spawners::parse(&a).unwrap();
        let fb = crate::zone_spawners::parse(&b).unwrap();
        assert_eq!(fa.blocks.iter().map(|b| b.id).collect::<Vec<_>>(), vec![10, 30, 31]);
        assert_eq!(fa.blocks[0].point.unwrap().0, Vec3::new(7.5, 5.0, 1.0));
        assert_eq!(fa.blocks[1].text, area);
        assert_eq!((fa.blocks[2].spawner_type, fa.blocks[2].point.unwrap().0), (9100, Vec3::new(10.0, 10.0, 0.0)));
        assert!((fa.blocks[2].point.unwrap().1 - std::f32::consts::FRAC_PI_2).abs() < 1e-5);
        assert_eq!((fb.blocks.len(), fb.blocks[0].id, fb.blocks[0].point.unwrap().0), (1, 20, Vec3::new(100.0, 6.0, 1.0)));
        assert_eq!(store.changed_count(), 0);
        assert_eq!(store.spawns[added].native.unwrap().id, 31);

        // undo the border move and save again: the spawner returns to zone 1 with its id
        assert!(store.undo());
        assert_eq!(store.changed_count(), 2);
        store.save(&[], |_, _| None).unwrap();
        let a = crate::zone_spawners::parse(&std::fs::read_to_string(dir.join(crate::zone_spawners::file_name("w", 1))).unwrap()).unwrap();
        let b = std::fs::read_to_string(dir.join(crate::zone_spawners::file_name("w", 2))).unwrap();
        assert_eq!(a.blocks.iter().map(|b| b.id).collect::<Vec<_>>(), vec![10, 30, 31, 20]);
        assert_eq!(a.blocks[0].point.unwrap().0, Vec3::new(5.0, 5.0, 1.0));
        assert!(b.is_empty());

        // a reload reads the saved overlay instead of the base file
        let mut again = SpawnStore::empty(&dir);
        again.load_native_with("w", &dir, WorldZones::parse(xml).unwrap(), templates(), |_| Ok(None)).unwrap();
        assert_eq!(again.spawns.len(), 3);
        assert!(again.add(Kind::Npc, Placement::new(500, origin, 0.0)).is_ok());
        assert_eq!(again.spawns.last().unwrap().native.unwrap().id, 32);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn native_spawner_border_move_converts_to_the_new_zone_origin() {
        let dir = std::env::temp_dir().join(format!("aa-native-origins-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let xml = r#"<World><ZoneList>
            <Zone name="a" id="1" originX="1" originY="0"><cellList><cell x="1" y="0"><sectorList><sector x="0" y="0"/></sectorList></cell></cellList></Zone>
            <Zone name="b" id="2" originX="0" originY="1"><cellList><cell x="1" y="0"><sectorList><sector x="1" y="0"/></sectorList></cell></cellList></Zone>
        </ZoneList></World>"#;
        let one = crate::zone_spawners::new_block(20, 9000, Vec3::new(6.0, 6.0, 1.0), 0.0, "\n");
        let mut templates = Templates::default();
        templates.by_npc.insert(500, 9000);
        templates.npc_of.insert(9000, 500);
        let read = |name: &str| Ok((name == crate::zone_spawners::file_name("w", 1)).then(|| one.clone()));
        let mut store = SpawnStore::empty(&dir);
        store.load_native_with("w", &dir, WorldZones::parse(xml).unwrap(), templates, read).unwrap();
        assert_eq!(store.spawns[0].now.pos, Vec3::new(1030.0, 6.0, 1.0));
        store.apply(vec![(0, Placement { pos: Vec3::new(1124.0, 6.0, 1.0), ..store.spawns[0].now })]);
        store.save(&[], |_, _| None).unwrap();
        let b = crate::zone_spawners::parse(&std::fs::read_to_string(dir.join(crate::zone_spawners::file_name("w", 2))).unwrap()).unwrap();
        assert_eq!((b.blocks[0].id, b.blocks[0].point.unwrap().0), (20, Vec3::new(1124.0, -1018.0, 1.0)));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn follow_path_is_read_set_and_cleared() {
        let dir = std::env::temp_dir().join(format!("aa-editor-follow-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let retail = r#"[
    { "UnitId": 1, "FollowPath": "walk_a", "Position": { "X": 1, "Y": 2, "Z": 3 } },
    { "UnitId": 2, "Position": { "X": 4, "Y": 5, "Z": 6 } }
]"#;
        std::fs::write(dir.join("npc_spawns.json"), retail).unwrap();
        let mut store = SpawnStore::load(&dir).unwrap();
        assert_eq!(store.path_of(&store.spawns[0].now), Some("walk_a"));
        assert_eq!(store.path_of(&store.spawns[1].now), None);
        let (mut a, mut b) = (store.spawns[0].now, store.spawns[1].now);
        a.path = 0;
        b.path = store.path_id("walk_b");
        store.apply(vec![(0, a), (1, b)]);
        assert_eq!(store.changed_count(), 2);
        store.save(&[], |_, _| None).unwrap();
        let text = std::fs::read_to_string(dir.join("npc_spawns.json")).unwrap();
        assert!(!text.contains("walk_a") && text.contains("\"FollowPath\": \"walk_b\""), "{text}");
        let again = SpawnStore::load(&dir).unwrap();
        assert_eq!(again.path_of(&again.spawns[1].now), Some("walk_b"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn untouched_entries_keep_their_text_and_edits_round_trip() {
        let dir = std::env::temp_dir().join(format!("aa-editor-spawns-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let retail = r#"[
    {
        "UnitId": 3111, // Gorgon
        "Position": { "X": 10.5, "Y": 20, "Z": 3, "Yaw": -45.0 }
    },
    {
        "UnitId": 502,
        "RespawnTime": 15,
        "Position": { "X": 1, "Y": 2, "Z": 3 }
    }
]"#;
        std::fs::write(dir.join("npc_spawns.json"), retail).unwrap();
        let mut store = SpawnStore::load(&dir).unwrap();
        assert_eq!(store.spawns.len(), 2);
        assert_eq!(store.spawns[0].now.yaw, -45.0);
        // move the second, add a doodad
        let mut p = store.spawns[1].now;
        p.pos.x = 100.25;
        store.apply(vec![(1, p)]);
        store.add(Kind::Doodad, Placement::new(1373, Vec3::new(5.0, 6.0, 7.0), 90.0)).unwrap();
        let written = store.save(&[], |_, id| (id == 1373).then(|| "Dead body".to_string())).unwrap();
        assert_eq!(written.len(), 2);
        let text = std::fs::read_to_string(dir.join("npc_spawns.json")).unwrap();
        assert!(text.contains("// Gorgon"), "{text}");
        assert!(text.contains("\"RespawnTime\": 15") && text.contains("100.25"), "{text}");
        assert!(dir.join("npc_spawns.json.editor-orig").exists());
        let doodads = std::fs::read_to_string(dir.join("doodad_spawns_editor_overlay.json")).unwrap();
        assert!(doodads.contains("\"UnitId\": 1373, // Dead body"), "{doodads}");
        // reload: same data; undo of the delete etc.
        let mut again = SpawnStore::load(&dir).unwrap();
        assert_eq!(again.spawns.len(), 3);
        let moved = again.spawns.iter().find(|s| s.now.unit_id == 502).unwrap();
        assert_eq!(moved.now.pos.x, 100.25);
        let gorgon = again.spawns.iter().position(|s| s.now.unit_id == 3111).unwrap();
        let mut del = again.spawns[gorgon].now;
        del.deleted = true;
        again.apply(vec![(gorgon, del)]);
        assert_eq!(again.changed_count(), 1);
        assert!(again.undo());
        assert_eq!(again.changed_count(), 0);
        assert!(again.redo());
        again.save(&[], |_, _| None).unwrap();
        let text = std::fs::read_to_string(dir.join("npc_spawns.json")).unwrap();
        assert!(!text.contains("3111"), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn retail_files_load_and_a_single_edit_changes_only_its_entry() {
        let source = Path::new("server/Data/Worlds/main_world");
        if !source.join("npc_spawns.json").exists() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("aa-editor-retail-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["npc_spawns.json", "doodad_spawns.json"] {
            std::fs::copy(source.join(name), dir.join(name)).unwrap();
        }
        let mut store = SpawnStore::load(&dir).unwrap();
        let npcs = store.spawns.iter().filter(|s| s.kind == Kind::Npc).count();
        let doodads = store.spawns.len() - npcs;
        assert!(npcs > 10_000 && doodads > 10_000, "{npcs} npcs, {doodads} doodads");
        let original = std::fs::read_to_string(dir.join("npc_spawns.json")).unwrap();
        let mut p = store.spawns[0].now;
        p.pos.z += 1.0;
        store.apply(vec![(0, p)]);
        store.save(&[], |_, _| None).unwrap();
        let saved = std::fs::read_to_string(dir.join("npc_spawns.json")).unwrap();
        let (a, b): (Vec<&str>, Vec<&str>) = (original.lines().collect(), saved.lines().collect());
        let differing = a.iter().zip(&b).filter(|(x, y)| x.trim_end() != y.trim_end()).count();
        assert!(differing <= 12, "{differing} lines differ");
        assert_eq!(SpawnStore::load(&dir).unwrap().spawns.len(), store.spawns.len());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
