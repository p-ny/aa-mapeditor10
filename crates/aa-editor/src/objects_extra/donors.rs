//! Roads and water volumes of the whole world, for placing a copy in a cell that has none (sea cells, new islands).
//! A copy keeps the original's shape, size and look; its material entry is copied into the destination cell's
//! `object.dat` material table (and `material_list.dat`) when that cell does not have it yet.
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use aa_world::placement_write::ObjectTree;
use bevy::prelude::*;

use super::{ExtraKind, bbox, f32_at, u32_at};

pub struct Donor {
    pub cell: (i32, i32),
    /// `object.dat` item type and payload.
    pub kind: u32,
    pub bytes: Vec<u8>,
    /// Material name, its octree table record and its `material_list.dat` record.
    pub material: Option<(String, Vec<u8>, Option<Vec<u8>>)>,
    pub label: String,
}

/// Byte offset of the material index in an item of `kind` (roads keep a sort priority in the top byte).
pub fn material_at(kind: u32) -> Option<usize> {
    match kind {
        13 => Some(59),
        11 => Some(51),
        _ => None,
    }
}

/// The roads or water volumes (`kind`) of every cell of `world` (lakes before rivers, largest first); read once.
pub fn donors(world: &str, kind: ExtraKind) -> Arc<Vec<Donor>> {
    static CACHE: OnceLock<Mutex<HashMap<(String, u32), Arc<Vec<Donor>>>>> = OnceLock::new();
    let mut cache = CACHE.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    cache.entry((world.to_string(), kind.item())).or_insert_with(|| Arc::new(scan(world, kind.item()))).clone()
}

fn scan(world: &str, kind: u32) -> Vec<Donor> {
    let pak = aa_pak::Pak::global();
    let prefix = format!("game/worlds/{world}/cells/");
    let names: Vec<String> = pak.names().filter(|n| n.starts_with(&prefix) && n.ends_with("/client/object.dat")).map(String::from).collect();
    let mut out = Vec::new();
    for n in names {
        let Some(cell) = n[prefix.len()..].split('/').next().and_then(|d| {
            let (x, y) = d.split_once('_')?;
            Some((x.parse().ok()?, y.parse().ok()?))
        }) else {
            continue;
        };
        let Ok(data) = pak.read(&n) else { continue };
        let Some(tree) = ObjectTree::parse(&data) else { continue };
        let items = tree.items_of(kind);
        if items.is_empty() {
            continue;
        }
        let external = pak.read(&n.replace("/object.dat", "/material_list.dat")).ok();
        for i in items {
            let item = &tree.items[i];
            if item.deleted || item.bytes.len() < 24 {
                continue;
            }
            let (lo, hi) = bbox(&item.bytes);
            if !(lo.is_finite() && hi.is_finite()) {
                continue;
            }
            let size = hi - lo;
            let material = material_at(kind).and_then(|at| {
                let index = (u32_at(&item.bytes, at) & 0x00ff_ffff) as usize;
                let (name, record) = crate::native_tables::tree_material(&tree.head, index).ok()?;
                let ext = external.as_deref().and_then(|t| crate::native_tables::external_material(t, &name).ok().flatten());
                Some((name, record, ext))
            });
            let look = material.as_ref().map(|m| m.0.rsplit('/').next().unwrap_or(&m.0).to_string()).unwrap_or_default();
            let label = match kind {
                11 => {
                    let what = if u32_at(&item.bytes, 39) & 3 == 3 { "river" } else { "lake" };
                    let surface = if f32_at(&item.bytes, 79) > 0.99 { -f32_at(&item.bytes, 83) } else { hi.z };
                    format!("{what} {:.0} x {:.0} m, surface {surface:.0} m  ·  {look}  ·  cell {:03}_{:03}", size.x, size.y, cell.0, cell.1)
                }
                _ => format!("road {:.0} x {:.0} m  ·  {look}  ·  cell {:03}_{:03}", size.x, size.y, cell.0, cell.1),
            };
            out.push(Donor { cell, kind, bytes: item.bytes.clone(), material, label });
        }
    }
    // lakes and ponds first (rivers are chains of small quads), each largest first
    let river = |d: &Donor| d.kind == 11 && u32_at(&d.bytes, 39) & 3 == 3;
    let area = |d: &Donor| {
        let (lo, hi) = bbox(&d.bytes);
        (hi.x - lo.x) * (hi.y - lo.y)
    };
    out.sort_by(|a, b| river(a).cmp(&river(b)).then(area(b).total_cmp(&area(a))));
    out
}

/// The donor's outline points (cell-local of the donor), for the placement ghost.
pub fn ghost_points(d: &Donor) -> Vec<Vec3> {
    super::outline(d.kind, &d.bytes)
}

/// Centre of the donor's box and the height the copy is placed by: the water surface, a road's lowest point.
pub fn anchor(d: &Donor) -> (Vec3, f32) {
    let (lo, hi) = bbox(&d.bytes);
    let centre = (lo + hi) * 0.5;
    let z = match d.kind {
        11 if f32_at(&d.bytes, 79) > 0.99 => -f32_at(&d.bytes, 83),
        11 => hi.z,
        _ => lo.z,
    };
    (centre, z)
}
