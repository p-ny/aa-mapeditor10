//! Zone borders (10.x): a half transparent wall along every 64 m sector edge where the owning `world.xml` zone
//! changes, hugging the terrain near the camera, plus the zone under the camera. Key B or the window toggles it.
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use aa_world::{TerrainQuery, bevy_to_cry, cry_to_bevy};
use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy_egui::input::EguiWantsInput;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};

use crate::camera::EditorCamera;
use crate::terrain_edit::Heights;
use crate::zone_spawners::WorldZones;
use crate::Editor;

const SECTOR: f32 = 64.0;
/// Sectors around the camera that get walls (4 km: large sea zones have borders far apart).
const REACH: i32 = 64;
/// Rebuild after the camera moved this many sectors.
const REBUILD: i32 = 16;
const BELOW: f32 = 5.0;
const ABOVE: f32 = 40.0;
/// Points along one sector edge.
const STEPS: usize = 4;

#[derive(Resource)]
struct Borders {
    show: bool,
    world: String,
    zones: Option<WorldZones>,
    /// The world's water level (`oceanLevel` in world.xml): walls at sea reach above it.
    ocean: f32,
    built_at: Option<(i32, i32)>,
    built_edits: usize,
    entity: Option<Entity>,
    material: Option<Handle<StandardMaterial>>,
    here: String,
}

impl Default for Borders {
    fn default() -> Self {
        Borders { show: true, world: String::new(), zones: None, ocean: 100.0, built_at: None, built_edits: 0, entity: None, material: None, here: String::new() }
    }
}

pub fn plugin(app: &mut App) {
    if !crate::flag("--tenx") {
        return;
    }
    app.init_resource::<Borders>().add_systems(Update, (keys, rebuild).chain()).add_systems(EguiPrimaryContextPass, window.after(crate::ui::panels));
}

fn keys(keys: Res<ButtonInput<KeyCode>>, egui: Res<EguiWantsInput>, mut borders: ResMut<Borders>) {
    if keys.just_pressed(KeyCode::KeyB) && !egui.wants_any_keyboard_input() {
        borders.show = !borders.show;
        borders.built_at = None;
    }
}

/// A steady colour per zone pair, so one border keeps its colour along its length.
fn pair_color(a: Option<u32>, b: Option<u32>) -> [f32; 4] {
    let mut h = DefaultHasher::new();
    (a.min(b), a.max(b)).hash(&mut h);
    let hue = (h.finish() % 360) as f32;
    let c = Color::hsla(hue, 0.85, 0.55, 0.28).to_linear();
    [c.red, c.green, c.blue, c.alpha]
}

#[allow(clippy::too_many_arguments)]
fn rebuild(
    mut commands: Commands,
    editor: Res<Editor>,
    heights: Res<Heights>,
    query: Res<TerrainQuery>,
    cam: Query<&GlobalTransform, With<EditorCamera>>,
    mut borders: ResMut<Borders>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    if borders.world != editor.world || borders.zones.is_none() {
        borders.world = editor.world.clone();
        let xml = aa_pak::Pak::global().read_string(&format!("worlds/{}/world.xml", editor.world)).ok();
        borders.ocean = xml
            .as_deref()
            .and_then(|x| x.split("oceanLevel=\"").nth(1))
            .and_then(|v| v.split('"').next())
            .and_then(|v| v.parse().ok())
            .unwrap_or(100.0);
        borders.zones = xml.and_then(|xml| WorldZones::parse(&xml).ok());
        borders.built_at = None;
    }
    let Ok(ct) = cam.single() else { return };
    let b = &mut *borders;
    let camera = bevy_to_cry(ct.translation());
    let at = ((camera.x / SECTOR).floor() as i32, (camera.y / SECTOR).floor() as i32);
    let Some(zones) = b.zones.as_ref() else { return };
    b.here = match zones.sector_owner(at) {
        Some(z) => format!("{} {}", z.id, z.name),
        None => "no zone".into(),
    };
    let edits = heights.edited.len() + heights.unsaved.len();
    let stale = b.built_at.is_none_or(|b| (b.0 - at.0).abs() > REBUILD || (b.1 - at.1).abs() > REBUILD) || edits != b.built_edits;
    if !stale {
        return;
    }
    if let Some(e) = b.entity.take() {
        commands.entity(e).despawn();
    }
    b.built_at = Some(at);
    b.built_edits = edits;
    if !b.show {
        return;
    }
    let ground = |x: f32, y: f32| heights.height_at(&query, x, y).unwrap_or(camera.z.min(200.0));
    let ocean = b.ocean;
    let (mut pos, mut col, mut idx) = (Vec::new(), Vec::new(), Vec::<u32>::new());
    let mut wall = |a: Vec2, b: Vec2, color: [f32; 4]| {
        for s in 0..STEPS {
            let (p, q) = (a.lerp(b, s as f32 / STEPS as f32), a.lerp(b, (s + 1) as f32 / STEPS as f32));
            let (zp, zq) = (ground(p.x, p.y), ground(q.x, q.y));
            let base = pos.len() as u32;
            // at sea the wall rises from the seabed to above the water surface
            let (tp, tq) = (zp.max(ocean) + ABOVE, zq.max(ocean) + ABOVE);
            for v in [Vec3::new(p.x, p.y, zp - BELOW), Vec3::new(q.x, q.y, zq - BELOW), Vec3::new(p.x, p.y, tp), Vec3::new(q.x, q.y, tq)] {
                pos.push(cry_to_bevy(v).to_array());
                col.push(color);
            }
            idx.extend([base, base + 1, base + 2, base + 1, base + 3, base + 2]);
        }
    };
    let owner = |s: (i32, i32)| zones.sector_owner(s).map(|z| z.id);
    for gx in at.0 - REACH..=at.0 + REACH {
        for gy in at.1 - REACH..=at.1 + REACH {
            let here = owner((gx, gy));
            let (east, north) = (owner((gx + 1, gy)), owner((gx, gy + 1)));
            let (x0, y0, x1, y1) = (gx as f32 * SECTOR, gy as f32 * SECTOR, (gx + 1) as f32 * SECTOR, (gy + 1) as f32 * SECTOR);
            if here != east {
                wall(Vec2::new(x1, y0), Vec2::new(x1, y1), pair_color(here, east));
            }
            if here != north {
                wall(Vec2::new(x0, y1), Vec2::new(x1, y1), pair_color(here, north));
            }
        }
    }
    if pos.is_empty() {
        return;
    }
    let mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, col)
        .with_inserted_indices(Indices::U32(idx));
    let material = b
        .material
        .get_or_insert_with(|| materials.add(StandardMaterial { base_color: Color::WHITE, unlit: true, alpha_mode: AlphaMode::Blend, cull_mode: None, ..default() }))
        .clone();
    let e = commands.spawn((Name::new("zone borders"), Mesh3d(meshes.add(mesh)), MeshMaterial3d(material), Transform::default())).id();
    b.entity = Some(e);
}

fn window(mut contexts: EguiContexts, mut borders: ResMut<Borders>) -> Result {
    let ctx = contexts.ctx_mut()?;
    egui::Window::new("Zone borders").default_pos([1300.0, 520.0]).default_width(240.0).show(ctx, |ui| {
        if ui.checkbox(&mut borders.show, "Show zone borders (B)").changed() {
            borders.built_at = None;
        }
        ui.small(format!("Camera in zone: {}", borders.here));
    });
    Ok(())
}
