//! `cell-tidy <cell client dir>...`: makes generated sea cells editable like land cells.
//!
//! * Surface slots: drops the `__default__` placeholder slots no sample uses from `leveldata.xml` and renumbers the
//!   samples, so the editor can add materials again (a cell holds at most 31). Node surface lists are rebuilt.
//! * Cover: rewrites `cover.ctc` with all five levels (0.5 m per pixel; ocean cells ship only one or two). Texels
//!   on generated ground (`--palette` materials) get that material's colour with some variation, unless the old cover
//!   there was tinted to something else; everything else keeps the old colour.
//!
//! Each file is backed up next to itself as `<name>.before-tidy` once.
use aa_world::cover::{Cover, CoverFormat};
use aa_world::heightmap::{CellHeights, GRID};
use anyhow::{Context, Result, bail, ensure};
use std::path::Path;
#[path = "../../../aa-editor/src/heightmap_write.rs"]
mod heightmap_write;
#[path = "shared/cover_out.rs"]
mod cover_out;

use cover_out::{FINE, LEVELS, Sampler, encode, noise};

/// Generated-ground materials and their cover colours (island-gen's palette).
const PALETTE: [(&str, [u8; 3]); 10] = [
    ("beach_white_sand_wet", [176, 164, 128]),
    ("tr_wt_sand_800a", [205, 190, 150]),
    ("tr_field_grass_001a_veg", [74, 100, 46]),
    ("tr_field_grass_004a", [100, 122, 62]),
    ("tr_mix_rc_gr_211a", [112, 112, 90]),
    ("tr_canyon_rock3d_302a", [118, 112, 104]),
    ("tr_field_dirt_001a", [125, 104, 78]),
    ("tr_wasteland_dirt_801a", [92, 86, 80]),
    ("tr_canyon_rock3d_808a", [58, 52, 50]),
    ("tr_beach_coral_201b", [160, 160, 125]),
];

fn backup(p: &Path) -> Result<()> {
    let b = p.with_file_name(format!("{}.before-tidy", p.file_name().unwrap().to_string_lossy()));
    if !b.exists() {
        std::fs::copy(p, b)?;
    }
    Ok(())
}

fn name_of(l: &str) -> &str {
    l.split("Name=\"").nth(1).and_then(|r| r.split('"').next()).unwrap_or("")
}

/// Rewrites every node's surface list: a sector lists its used surfaces (sorted, without 31), a parent the union
/// of the sectors inside it in order of first appearance.
fn rebuild_lists(b: &[u8]) -> Result<Vec<u8>> {
    struct N { start: usize, list_at: usize, count: usize, end: usize, size: usize, min: [f32; 2], max: [f32; 2], used: Vec<u8> }
    let f = |at: usize| f32::from_le_bytes(b[at..at + 4].try_into().unwrap());
    let i = |at: usize| i32::from_le_bytes(b[at..at + 4].try_into().unwrap());
    let header = 32 + if b[0] >= 24 { 128 } else { 0 };
    let chunk = u32::from_le_bytes(b[4..8].try_into()?) as usize;
    let mut nodes = Vec::new();
    let mut pos = header;
    while pos < chunk {
        let size = i(pos + 37) as usize;
        let count = i(pos + 41).max(0) as usize;
        let data = pos + 45;
        let list_at = data + size * size * 2 + 20;
        let mut used: Vec<u8> = (0..size * size).map(|k| (u16::from_le_bytes([b[data + 2 * k], b[data + 2 * k + 1]]) & 31) as u8).filter(|&v| v != 31).collect();
        used.sort_unstable();
        used.dedup();
        nodes.push(N { start: pos, list_at, count, end: list_at + count + 36, size, min: [f(pos + 4), f(pos + 8)], max: [f(pos + 16), f(pos + 20)], used });
        pos = list_at + count + 36;
    }
    ensure!(pos == chunk, "node walk does not end at the chunk size");
    let mut out = b[..header].to_vec();
    for n in &nodes {
        let list: Vec<u8> = if n.size > 0 {
            n.used.clone()
        } else {
            let mut l = Vec::new();
            for m in nodes.iter().filter(|m| m.size > 0) {
                let (cx, cy) = ((m.min[0] + m.max[0]) * 0.5, (m.min[1] + m.max[1]) * 0.5);
                if cx >= n.min[0] && cx <= n.max[0] && cy >= n.min[1] && cy <= n.max[1] {
                    for &v in &m.used {
                        if !l.contains(&v) {
                            l.push(v);
                        }
                    }
                }
            }
            l
        };
        let mut head = b[n.start..n.list_at].to_vec();
        head[41..45].copy_from_slice(&(list.len() as i32).to_le_bytes());
        out.extend_from_slice(&head);
        out.extend_from_slice(&list);
        out.extend_from_slice(&b[n.list_at + n.count..n.end]);
    }
    let len = out.len() as u32;
    out[4..8].copy_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&b[chunk..]);
    Ok(out)
}

/// Drops unused placeholder slots; returns the new heightmap, leveldata and slot -> material name.
fn compact(hm: &[u8], xml: &str) -> Result<(Vec<u8>, String, Vec<String>)> {
    let mut h = CellHeights::parse(hm).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let lines: Vec<&str> = xml.lines().collect();
    let slots: Vec<usize> = lines.iter().enumerate().filter(|(_, l)| l.contains("<SurfaceType ")).map(|(i, _)| i).collect();
    let mut used = [false; 32];
    for &s in &h.surface {
        used[s as usize] = true;
    }
    let mut remap = [31u8; 32];
    let mut keep = Vec::new();
    for (slot, &line) in slots.iter().enumerate() {
        if used[slot] || name_of(lines[line]) != "__default__" {
            remap[slot] = keep.len() as u8;
            keep.push(line);
        }
    }
    for (slot, u) in used.iter().enumerate().take(31) {
        ensure!(!u || slot < slots.len(), "samples use slot {slot}, which leveldata.xml does not define");
    }
    let names: Vec<String> = keep.iter().map(|&l| name_of(lines[l]).to_string()).collect();
    if keep.len() == slots.len() {
        return Ok((hm.to_vec(), xml.to_string(), names));
    }
    for s in h.surface.iter_mut() {
        *s = remap[*s as usize];
    }
    let dropped: Vec<usize> = slots.iter().copied().filter(|l| !keep.contains(l)).collect();
    let new_xml = lines.iter().enumerate().filter(|(i, _)| !dropped.contains(i)).map(|(_, l)| *l).collect::<Vec<_>>().join("\n") + "\n";
    let written = heightmap_write::write(hm, &h)?;
    let written = rebuild_lists(&written)?;
    let back = CellHeights::parse(&written).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    ensure!(back.surface == h.surface, "surface round trip failed");
    let err = back.heights.iter().zip(&h.heights).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
    ensure!(err < 0.1, "height round trip error {err}");
    Ok((written, new_xml, names))
}

fn differs(a: [f32; 3], b: [f32; 3]) -> bool {
    (a[0] - b[0]).abs() + (a[1] - b[1]).abs() + (a[2] - b[2]).abs() > 24.0
}

/// `pristine`: the cell's cover before any edit (pak), to tell user tints from generated or seabed colour.
fn rebuild_cover(old: &[u8], pristine: Option<&[u8]>, h: &CellHeights, names: &[String], cell: (i32, i32)) -> Result<Vec<u8>> {
    let cover = Cover::parse(old).map_err(|e| anyhow::anyhow!("{e}"))?;
    let old_s = Sampler::new(old)?;
    let pristine = pristine.map(Sampler::new).transpose()?;
    ensure!(cover.layers.len() == 1 && cover.layers[0].format == CoverFormat::Bc1, "only one-layer BC1 covers");
    let colour_of: Vec<Option<[u8; 3]>> = names.iter().map(|n| PALETTE.iter().find(|p| p.0 == n).map(|p| p.1)).collect();
    // finest level, image rows along world X
    let mut fine = vec![[0f32; 3]; FINE * FINE];
    let (wx0, wy0) = (cell.0 as f32 * 1024.0, cell.1 as f32 * 1024.0);
    for row in 0..FINE {
        for col in 0..FINE {
            let (lx, ly) = ((row as f32 + 0.5) * 0.5, (col as f32 + 0.5) * 0.5);
            let old = old_s.at(lx, ly);
            let (ix, iy) = (((lx / 2.0).round() as usize).min(GRID - 1), ((ly / 2.0).round() as usize).min(GRID - 1));
            let s = h.surface[iy * GRID + ix] as usize;
            let mut px = old;
            if let Some(Some(c)) = colour_of.get(s) {
                let flat = encode(c.map(|v| v as f32 / 255.0));
                // a tint: neither the generated colour nor the untouched original
                let tinted = differs(old, flat) && pristine.as_ref().is_none_or(|p| differs(old, p.at(lx, ly)));
                if !tinted {
                    let (x, y) = (wx0 + lx, wy0 + ly);
                    let v = 1.0 + 0.10 * noise(x / 9.0, y / 9.0, 1) + 0.06 * noise(x / 2.5, y / 2.5, 2) + 0.08 * noise(x / 40.0, y / 40.0, 3);
                    let warm = 0.05 * noise(x / 25.0, y / 25.0, 4);
                    px = encode([c[0] as f32 / 255.0 * v * (1.0 + warm), c[1] as f32 / 255.0 * v, c[2] as f32 / 255.0 * v * (1.0 - warm)]);
                }
            }
            fine[row * FINE + col] = px;
        }
    }
    cover_out::build(old, fine)
}

fn main() -> Result<()> {
    unsafe { std::env::remove_var("AA_PAK_OVERLAY") };
    let dirs: Vec<String> = std::env::args().skip(1).collect();
    if dirs.is_empty() {
        bail!("usage: cell-tidy <.../cells/XXX_YYY/client>...");
    }
    for d in dirs {
        let dir = Path::new(&d);
        let cell_name = dir.parent().and_then(|p| p.file_name()).context("cell folder")?.to_string_lossy().to_string();
        let (x, y) = cell_name.split_once('_').context("XXX_YYY")?;
        let cell = (x.parse::<i32>()?, y.parse::<i32>()?);
        let (hm_p, ld_p, cv_p) = (dir.join("terrain/heightmap.dat"), dir.join("leveldata.xml"), dir.join("terrain/cover.ctc"));
        let hm = std::fs::read(&hm_p)?;
        let ld = std::fs::read_to_string(&ld_p)?;
        let (new_hm, new_ld, names) = compact(&hm, &ld)?;
        let before = ld.matches("<SurfaceType ").count();
        if new_hm != hm {
            backup(&hm_p)?;
            backup(&ld_p)?;
            std::fs::write(&hm_p, &new_hm)?;
            std::fs::write(&ld_p, &new_ld)?;
        }
        let mut cover_note = "no cover".to_string();
        // a cell without its own cover in the overlay uses the pak's (AA_PAK, without the overlay)
        let rel = format!("game/worlds/main_world/cells/{cell_name}/client/terrain/cover.ctc");
        let pristine = aa_pak::Pak::global().read(&rel).ok();
        let old = if cv_p.exists() { Some(std::fs::read(&cv_p)?) } else { pristine.clone() };
        if let Some(old) = old {
            let parsed = Cover::parse(&old).ok();
            let levels = parsed.as_ref().map(|c| c.max_level + 1).unwrap_or(0);
            // complete: every tile of every level (1 + 4 + 16 + 64 + 256)
            let tiles = parsed.as_ref().map_or(0, |c| (0..LEVELS).map(|l| (0..1usize << l).flat_map(|x| (0..1usize << l).map(move |y| (x, y))).filter(|&(x, y)| c.tile(0, l, x, y).is_some()).count()).sum::<usize>());
            if tiles < 341 {
                let h = CellHeights::parse(&new_hm).map_err(|e| anyhow::anyhow!("{e:?}"))?;
                let rebuilt = rebuild_cover(&old, pristine.as_deref(), &h, &names, cell)?;
                if cv_p.exists() {
                    backup(&cv_p)?;
                }
                std::fs::write(&cv_p, rebuilt)?;
                cover_note = format!("cover {levels} levels / {tiles} tiles -> complete");
            } else {
                cover_note = "cover already complete".to_string();
            }
        }
        println!("{cell_name}: surface slots {before} -> {} ({} free), {cover_note}", names.len(), 31 - names.len());
    }
    Ok(())
}
