//! The vegetation brush (Objects mode, "More objects" > "New vegetation" > "Paint with a brush"): holding the left
//! mouse button scatters plants of the chosen groups over the ground under the circle, Shift + drag removes them.
//!
//! Each new plant is a `vegetation.dat` record made from a real record of the same group (any cell of the world),
//! so cells that had no plants at all (sea cells, new islands) can take them. Plants keep a spacing that follows
//! the density, stay off water and steep slopes, and get a random turn and size. A stroke is one undo step.
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use aa_world::TerrainQuery;
use aa_world::coords::cell_origin;
use aa_world::objects::ObjectState;
use aa_world::placement::Vegetation;
use aa_world::placement_write::SectorFile;
use bevy::prelude::*;
use bevy_egui::egui;

use crate::terrain_edit::Heights;
use crate::tools::Cursor;
use crate::world_objects::WorldObjects;
use crate::{Editor, Mode};

#[derive(Resource)]
pub struct VegBrush {
    pub on: bool,
    /// Groups to paint (`vegetation.xml` ids); clicking a list entry adds or removes it.
    pub groups: Vec<u16>,
    pub radius: f32,
    /// Plants per 100 m² when the area is full.
    pub density: f32,
    pub scale: (f32, f32),
    /// Steepest ground (degrees) that gets plants.
    pub max_slope: f32,
    next: f32,
    counter: u32,
    stroke_started: bool,
    pub status: String,
}

impl Default for VegBrush {
    fn default() -> Self {
        VegBrush { on: false, groups: Vec::new(), radius: 10.0, density: 8.0, scale: (0.8, 1.25), max_slope: 35.0, next: 0.0, counter: 0, stroke_started: false, status: String::new() }
    }
}

pub fn plugin(app: &mut App) {
    app.init_resource::<VegBrush>().add_systems(Update, (paint, draw, scripted).after(crate::tools::update_cursor));
}

/// One real 64-byte `vegetation.dat` record and its parsed placement per group, from the world's cells, and the
/// group with the most records.
struct Templates {
    by_group: HashMap<u16, (Vec<u8>, Vegetation)>,
    common: Option<u16>,
}

fn templates(world: &str) -> Arc<Templates> {
    static CACHE: OnceLock<std::sync::Mutex<HashMap<String, Arc<Templates>>>> = OnceLock::new();
    let mut cache = CACHE.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    cache
        .entry(world.to_string())
        .or_insert_with(|| {
            let pak = aa_pak::Pak::global();
            let prefix = format!("game/worlds/{world}/cells/");
            let names: Vec<String> = pak.names().filter(|n| n.starts_with(&prefix) && n.ends_with("/client/vegetation.dat")).map(String::from).collect();
            let mut out: HashMap<u16, (Vec<u8>, Vegetation)> = HashMap::new();
            let mut count: HashMap<u16, usize> = HashMap::new();
            for n in names {
                let Ok(d) = pak.read(&n) else { continue };
                let (Some(file), Ok(parsed)) = (SectorFile::parse_vegetation(&d), aa_world::placement::parse_vegetation_dat(&d)) else { continue };
                if file.records.len() != parsed.len() {
                    continue;
                }
                for (r, v) in file.records.iter().zip(parsed) {
                    *count.entry(v.group).or_default() += 1;
                    out.entry(v.group).or_insert_with(|| (r.bytes.clone(), v));
                }
            }
            let common = count.into_iter().max_by_key(|e| e.1).map(|e| e.0);
            Arc::new(Templates { by_group: out, common })
        })
        .clone()
}

/// A real `vegetation.dat` record of `group` from any cell of `world`, and its placement. A group no cell uses (many
/// of `vegetation.xml`'s) gets the record of the world's most common plant with the group set.
pub fn template(world: &str, group: u16) -> Option<(Vec<u8>, Vegetation)> {
    let t = templates(world);
    let (mut bytes, mut v) = t.by_group.get(&group).or_else(|| t.by_group.get(&t.common?))?.clone();
    bytes[55..57].copy_from_slice(&group.to_le_bytes());
    v.group = group;
    Some((bytes, v))
}

fn hash(a: u32, b: u32) -> f32 {
    let mut h = a.wrapping_mul(374_761_393) ^ b.wrapping_mul(668_265_263);
    h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
    h ^= h >> 16;
    h as f32 / u32::MAX as f32
}

#[allow(clippy::too_many_arguments)]
fn paint(
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    editor: Res<Editor>,
    cursor: Res<Cursor>,
    state: Res<ObjectState>,
    heights: Res<Heights>,
    query: Res<TerrainQuery>,
    mut objs: ResMut<WorldObjects>,
    mut brush: ResMut<VegBrush>,
) {
    if !brush.on || editor.mode != Mode::Objects || editor.playing {
        return;
    }
    if !mouse.pressed(MouseButton::Left) {
        brush.stroke_started = false;
        return;
    }
    if mouse.just_pressed(MouseButton::Left) && cursor.over_ui {
        return;
    }
    let Some(hit) = cursor.ground.filter(|_| !cursor.over_ui) else { return };
    let now = time.elapsed_secs();
    if now < brush.next {
        return;
    }
    brush.next = now + 0.08;
    let world = editor.world.clone();
    let centre = hit.truncate();
    let merge = brush.stroke_started;
    if keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]) {
        let groups = brush.groups.clone();
        let n = objs.remove_plants(&world, &state, centre, brush.radius, &groups, merge);
        if n > 0 {
            brush.stroke_started = true;
            brush.status = format!("removed {n} plant(s)");
        }
        return;
    }
    scatter(&mut brush, &world, centre, &state, &heights, &query, &mut objs, merge);
}

/// One dab of plants around world `centre`.
#[allow(clippy::too_many_arguments)]
fn scatter(brush: &mut VegBrush, world: &str, centre: Vec2, state: &ObjectState, heights: &Heights, query: &TerrainQuery, objs: &mut WorldObjects, merge: bool) {
    if brush.groups.is_empty() {
        brush.status = "pick one or more plants in the list first".into();
        return;
    }
    let Some(groups) = state.vegetation_groups().cloned() else { return };
    // spacing that gives the density when the area is full
    let spacing = (100.0 / brush.density.max(0.1)).sqrt() * 0.75;
    let mut taken = objs.plants_near(world, state, centre, brush.radius + spacing);
    let area = std::f32::consts::PI * brush.radius * brush.radius;
    let tries = ((brush.density / 100.0 * area * 0.2).ceil() as usize).clamp(1, 200);
    let mut plants = Vec::new();
    let mut missing = None;
    for _ in 0..tries {
        brush.counter = brush.counter.wrapping_add(1);
        let k = brush.counter;
        let (a, r) = (hash(k, 1) * std::f32::consts::TAU, brush.radius * hash(k, 2).sqrt());
        let p = centre + Vec2::new(a.cos(), a.sin()) * r;
        if taken.iter().any(|q| q.distance(p) < spacing) {
            continue;
        }
        let (Some(z), Some(zx), Some(zy)) = (heights.height_at(query, p.x, p.y), heights.height_at(query, p.x + 1.0, p.y), heights.height_at(query, p.x, p.y + 1.0)) else { continue };
        let slope = ((zx - z).powi(2) + (zy - z).powi(2)).sqrt().atan().to_degrees();
        if slope > brush.max_slope || z < 100.3 {
            continue;
        }
        let group = brush.groups[(hash(k, 3) * brush.groups.len() as f32) as usize % brush.groups.len()];
        let Some((template, base)) = template(world, group) else {
            missing = Some(group);
            continue;
        };
        let c = ((p.x / 1024.0).floor() as i32, (p.y / 1024.0).floor() as i32);
        let local = p.extend(z) - cell_origin(c.0, c.1);
        let mut v = base;
        v.pos = local;
        v.scale = brush.scale.0 + (brush.scale.1 - brush.scale.0) * hash(k, 4);
        v.angle_x = 0;
        v.angle_y = 0;
        v.angle_z = (hash(k, 5) * 256.0) as u8;
        let model = groups.groups.get(&group).map(|g| g.model.clone()).unwrap_or_default();
        let (lo, hi) = crate::objects_extra::model_bounds(&model).unwrap_or((Vec3::new(-1.0, -1.0, 0.0), Vec3::new(1.0, 1.0, 2.0)));
        let m = v.mat4();
        let (mut blo, mut bhi) = (Vec3::MAX, Vec3::MIN);
        for i in 0..8 {
            let corner = Vec3::new(if i & 1 == 0 { lo.x } else { hi.x }, if i & 2 == 0 { lo.y } else { hi.y }, if i & 4 == 0 { lo.z } else { hi.z });
            let q = m.transform_point3(corner);
            blo = blo.min(q);
            bhi = bhi.max(q);
        }
        plants.push((c, template, v, [blo.x, blo.y, blo.z, bhi.x, bhi.y, bhi.z]));
        taken.push(p);
    }
    let n = objs.add_plants(world, state, plants, merge);
    if n > 0 {
        brush.stroke_started = true;
        brush.status = format!("painted {n} plant(s) (Ctrl+S saves, Ctrl+Z undoes the stroke)");
    } else if let Some(g) = missing {
        brush.status = format!("group {g} has no record anywhere in this world to copy");
    }
}

/// `--veg-paint x,y --veg-groups a,b[,..] [--veg-radius r] [--veg-density d] [--objects-save]`: one stroke of 20
/// dabs once the cell's objects are loaded (unattended checks).
#[allow(clippy::too_many_arguments)]
fn scripted(
    mut editor: ResMut<Editor>,
    state: Res<ObjectState>,
    heights: Res<Heights>,
    query: Res<TerrainQuery>,
    mut objs: ResMut<WorldObjects>,
    mut brush: ResMut<VegBrush>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    let Some(at) = crate::arg("--veg-paint").and_then(|v| {
        let v: Vec<f32> = v.split(',').filter_map(|s| s.trim().parse().ok()).collect();
        (v.len() == 2).then(|| Vec2::new(v[0], v[1]))
    }) else {
        *done = true;
        return;
    };
    let c = ((at.x / 1024.0).floor() as i32, (at.y / 1024.0).floor() as i32);
    if state.cell_data(c).is_none() || state.vegetation_groups().is_none() || heights.height_at(&query, at.x, at.y).is_none() {
        return;
    }
    *done = true;
    brush.groups = crate::arg("--veg-groups").map(|v| v.split(',').filter_map(|s| s.trim().parse().ok()).collect()).unwrap_or_default();
    brush.radius = crate::arg("--veg-radius").and_then(|v| v.parse().ok()).unwrap_or(brush.radius);
    brush.density = crate::arg("--veg-density").and_then(|v| v.parse().ok()).unwrap_or(brush.density);
    let world = editor.world.clone();
    for i in 0..20 {
        scatter(&mut brush, &world, at, &state, &heights, &query, &mut objs, i > 0);
    }
    let added = objs.plants_near(&world, &state, at, brush.radius).len();
    info!("veg_brush: scripted stroke at {at:?}: {} ({added} plants within the radius now)", brush.status);
    if crate::flag("--objects-save") {
        editor.requests.push(crate::Request::SaveObjects);
    }
}

fn draw(editor: Res<Editor>, cursor: Res<Cursor>, brush: Res<VegBrush>, mut gizmos: Gizmos) {
    if !brush.on || editor.mode != Mode::Objects || editor.playing || cursor.over_ui {
        return;
    }
    let Some(hit) = cursor.ground else { return };
    let iso = Isometry3d::new(aa_world::cry_to_bevy(hit + Vec3::Z * 0.3), Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2));
    gizmos.circle(iso, brush.radius, Color::srgb(0.3, 1.0, 0.4));
}

/// The brush part of the "New vegetation" tab. `clicked`: a group picked in the list below.
pub fn panel(ui: &mut egui::Ui, brush: &mut VegBrush) {
    ui.checkbox(&mut brush.on, egui::RichText::new("Paint with a brush").strong());
    if !brush.on {
        return;
    }
    ui.add(egui::Slider::new(&mut brush.radius, 1.0..=60.0).logarithmic(true).text("radius (m)"));
    ui.add(egui::Slider::new(&mut brush.density, 0.5..=60.0).logarithmic(true).text("plants per 100 m²"));
    ui.horizontal(|ui| {
        ui.label("size");
        ui.add(egui::DragValue::new(&mut brush.scale.0).speed(0.01).range(0.2..=3.0));
        ui.label("to");
        ui.add(egui::DragValue::new(&mut brush.scale.1).speed(0.01).range(0.2..=3.0));
    });
    if brush.scale.1 < brush.scale.0 {
        brush.scale.1 = brush.scale.0;
    }
    ui.add(egui::Slider::new(&mut brush.max_slope, 5.0..=80.0).text("steepest ground (°)"));
    if brush.groups.is_empty() {
        ui.colored_label(egui::Color32::from_rgb(255, 190, 60), "No plant chosen yet: click one or more entries in the list below (they turn highlighted).");
    } else {
        ui.label(egui::RichText::new(format!("{} plant type(s) chosen (highlighted below; click again to remove)", brush.groups.len())).strong());
    }
    ui.small("Hold the left mouse button on the ground to paint; Shift + drag removes the chosen types (all types when none is chosen).");
    if !brush.status.is_empty() {
        ui.small(&brush.status);
    }
}
