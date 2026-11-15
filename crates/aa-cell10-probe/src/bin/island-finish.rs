//! Makes generated terrain look like the game's own land. AA_PAK selects the pak (read without the overlay).
//!
//! * `island-finish catalog <out.json>`: for every surface material of main_world, 64 m sectors of real cells that
//!   are (almost) only that material, and the material's mean cover colour. The ground colour of such a sector is
//!   what the artists painted for that material.
//! * `island-finish erode <cells/XXX_YYY/client>...`: water erosion, slumping of over-steep slopes and a light
//!   smoothing, on the samples that differ from the pak (the generated land); the rest keeps its heights.
//! * `island-finish bake <catalog.json> <cells/XXX_YYY/client>...`: rebuilds the cover of generated ground from the
//!   catalog's donor sectors, with soft, irregular blends between materials and darker hollows.
//!
//! Files are backed up next to themselves once (`.before-erode`, `.before-bake`).
use aa_world::cover::{Cover, TILE_PX, bc1_decode};
use aa_world::heightmap::{CellHeights, GRID};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
#[path = "../../../aa-editor/src/heightmap_write.rs"]
mod heightmap_write;
#[path = "shared/cover_out.rs"]
mod cover_out;
use cover_out::{FINE, Sampler, hash, noise};

const WORLD: &str = "main_world";
const SEA: f32 = 100.0;

/// `DetailMaterial` as a key: lower case, forward slashes, without a leading `game/`.
fn material_key(m: &str) -> String {
    let m = m.to_lowercase().replace('\\', "/");
    m.strip_prefix("game/").unwrap_or(&m).to_string()
}

/// The material key of each slot of a `leveldata.xml`.
fn slot_materials(xml: &str) -> Vec<String> {
    xml.lines()
        .filter(|l| l.contains("<SurfaceType "))
        .map(|l| material_key(l.split("DetailMaterial=\"").nth(1).and_then(|r| r.split('"').next()).unwrap_or("")))
        .collect()
}

fn backup(p: &Path, tag: &str) -> Result<()> {
    let b = p.with_file_name(format!("{}.{tag}", p.file_name().unwrap().to_string_lossy()));
    if !b.exists() && p.exists() {
        std::fs::copy(p, b)?;
    }
    Ok(())
}

fn cell_of(dir: &Path) -> Result<(i32, i32)> {
    let name = dir.parent().and_then(|p| p.file_name()).context("cell folder")?.to_string_lossy().to_string();
    let (x, y) = name.split_once('_').context("XXX_YYY")?;
    Ok((x.parse()?, y.parse()?))
}

fn pak_path(c: (i32, i32), file: &str) -> String {
    format!("game/worlds/{WORLD}/cells/{:03}_{:03}/client/{file}", c.0, c.1)
}

fn parse_heights(b: &[u8]) -> Result<CellHeights> {
    CellHeights::parse(b).map_err(|e| anyhow::anyhow!("{e:?}"))
}

#[derive(Serialize, Deserialize, Default)]
struct Entry {
    /// (cell x, cell y, sector x, sector y, share of the sector's samples)
    donors: Vec<(i32, i32, usize, usize, f32)>,
    /// Mean encoded cover texel of the donors (0..255).
    mean: [f32; 3],
}

fn catalog(out: &Path) -> Result<()> {
    let pak = aa_pak::Pak::global();
    let prefix = format!("game/worlds/{WORLD}/cells/");
    let cells: Vec<(i32, i32)> = pak
        .names()
        .filter(|n| n.starts_with(&prefix) && n.ends_with("/client/leveldata.xml"))
        .filter_map(|n| {
            let (x, y) = n[prefix.len()..].split('/').next()?.split_once('_')?;
            Some((x.parse().ok()?, y.parse().ok()?))
        })
        .collect();
    let mut found: HashMap<String, Vec<(i32, i32, usize, usize, f32)>> = HashMap::new();
    for (k, &c) in cells.iter().enumerate() {
        let (Ok(xml), Ok(hm), Ok(cov)) = (pak.read_string(&pak_path(c, "leveldata.xml")), pak.read(&pak_path(c, "terrain/heightmap.dat")), pak.read(&pak_path(c, "terrain/cover.ctc"))) else { continue };
        let Ok(cover) = Cover::parse(&cov) else { continue };
        if cover.max_level < 4 {
            continue;
        }
        let Ok(h) = parse_heights(&hm) else { continue };
        let mats = slot_materials(&xml);
        for sy in 0..16 {
            for sx in 0..16 {
                if cover.tile(0, 4, sx, sy).is_none() {
                    continue;
                }
                let mut count = [0u32; 32];
                let mut wet = 0;
                for iy in sy * 32..sy * 32 + 32 {
                    for ix in sx * 32..sx * 32 + 32 {
                        let i = iy * GRID + ix;
                        count[h.surface[i] as usize & 31] += 1;
                        if h.heights[i] < h.ocean_level {
                            wet += 1;
                        }
                    }
                }
                let (slot, n) = count.iter().enumerate().take(31).max_by_key(|e| e.1).map(|(s, n)| (s, *n)).unwrap();
                let share = n as f32 / 1024.0;
                if share < 0.85 || wet > 900 {
                    continue;
                }
                let Some(m) = mats.get(slot).filter(|m| !m.is_empty()) else { continue };
                found.entry(m.clone()).or_default().push((c.0, c.1, sx, sy, share));
            }
        }
        if k % 500 == 0 {
            eprintln!("catalog: {k} / {} cells", cells.len());
        }
    }
    let mut table: BTreeMap<String, Entry> = BTreeMap::new();
    for (m, mut list) in found {
        // the purest sectors, from different cells
        list.sort_by(|a, b| b.4.total_cmp(&a.4).then((a.0, a.1, a.2, a.3).cmp(&(b.0, b.1, b.2, b.3))));
        let mut donors: Vec<(i32, i32, usize, usize, f32)> = Vec::new();
        for d in list {
            if donors.len() >= 60 {
                break;
            }
            if donors.iter().filter(|e| (e.0, e.1) == (d.0, d.1)).count() < 2 {
                donors.push(d);
            }
        }
        let mut sum = [0f64; 3];
        let mut n = 0f64;
        for d in &donors {
            if let Some(t) = donor_tile(pak, d) {
                for px in &t {
                    for k in 0..3 {
                        sum[k] += px[k] as f64;
                    }
                }
                n += t.len() as f64;
            }
        }
        if n > 0.0 {
            table.insert(m, Entry { donors, mean: sum.map(|s| (s / n) as f32) });
        }
    }
    std::fs::write(out, serde_json::to_string_pretty(&table)?)?;
    println!("catalog: {} materials with donor ground, from {} cells -> {}", table.len(), cells.len(), out.display());
    Ok(())
}

/// The finest cover tile (128 x 128 encoded texels, rows along X) of a donor sector.
fn donor_tile(pak: &aa_pak::Pak, d: &(i32, i32, usize, usize, f32)) -> Option<Vec<[u8; 3]>> {
    let cov = pak.read(&pak_path((d.0, d.1), "terrain/cover.ctc")).ok()?;
    let cover = Cover::parse(&cov).ok()?;
    Some(bc1_decode(cover.tile(0, 4, d.2, d.3)?))
}

/// The heights of a block of cells as one grid (2 m), the pak's and the current (overlay) ones.
struct Mosaic {
    x0: i32,
    y0: i32,
    w: usize,
    now: Vec<f32>,
    pak: Vec<f32>,
}

impl Mosaic {
    fn at(&self, gx: usize, gy: usize) -> usize {
        gy * self.w + gx
    }
}

fn box_blur(v: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    let mut tmp = vec![0f32; v.len()];
    let mut out = vec![0f32; v.len()];
    for y in 0..h {
        let mut acc = 0f32;
        let row = &v[y * w..(y + 1) * w];
        for x in 0..=r.min(w - 1) {
            acc += row[x];
        }
        for x in 0..w {
            let lo = x.saturating_sub(r);
            let hi = (x + r).min(w - 1);
            tmp[y * w + x] = acc / (hi - lo + 1) as f32;
            if x + r + 1 < w {
                acc += row[x + r + 1];
            }
            if x >= r {
                acc -= row[x - r];
            }
        }
    }
    for x in 0..w {
        let mut acc = 0f32;
        for y in 0..=r.min(h - 1) {
            acc += tmp[y * w + x];
        }
        for y in 0..h {
            let lo = y.saturating_sub(r);
            let hi = (y + r).min(h - 1);
            out[y * w + x] = acc / (hi - lo + 1) as f32;
            if y + r + 1 < h {
                acc += tmp[(y + r + 1) * w + x];
            }
            if y >= r {
                acc -= tmp[(y - r) * w + x];
            }
        }
    }
    out
}

fn erode(dirs: &[PathBuf]) -> Result<()> {
    let pak = aa_pak::Pak::global();
    let cells: Vec<(i32, i32)> = dirs.iter().map(|d| cell_of(d)).collect::<Result<_>>()?;
    let (x0, x1) = (cells.iter().map(|c| c.0).min().unwrap(), cells.iter().map(|c| c.0).max().unwrap());
    let (y0, y1) = (cells.iter().map(|c| c.1).min().unwrap(), cells.iter().map(|c| c.1).max().unwrap());
    let (w, h) = (((x1 - x0 + 1) * 512 + 1) as usize, ((y1 - y0 + 1) * 512 + 1) as usize);
    let mut m = Mosaic { x0, y0, w, now: vec![0.0; w * h], pak: vec![0.0; w * h] };
    let mut originals: HashMap<(i32, i32), (PathBuf, Vec<u8>, CellHeights)> = HashMap::new();
    for cy in y0..=y1 {
        for cx in x0..=x1 {
            let dir = dirs.iter().find(|d| cell_of(d).ok() == Some((cx, cy)));
            let base = parse_heights(&pak.read(&pak_path((cx, cy), "terrain/heightmap.dat"))?)?;
            let now = match dir {
                Some(d) => {
                    let b = std::fs::read(d.join("terrain/heightmap.dat"))?;
                    let h = parse_heights(&b)?;
                    originals.insert((cx, cy), (d.clone(), b, h.clone()));
                    h
                }
                None => base.clone(),
            };
            for iy in 0..GRID {
                for ix in 0..GRID {
                    let i = m.at((cx - x0) as usize * 512 + ix, (cy - y0) as usize * 512 + iy);
                    m.now[i] = now.heights[iy * GRID + ix];
                    m.pak[i] = base.heights[iy * GRID + ix];
                }
            }
        }
    }
    // where the land is generated (0..1, soft at the edges)
    let generated: Vec<f32> = m.now.iter().zip(&m.pak).map(|(a, b)| if (a - b).abs() > 0.05 { 1.0 } else { 0.0 }).collect();
    let weight: Vec<f32> = box_blur(&box_blur(&generated, w, h, 6), w, h, 6).iter().zip(&generated).map(|(b, g)| b.min(*g)).collect();
    let before = m.now.clone();
    // 1. water: droplets that pick up and drop sediment (heights in grid units: 1 = 2 m)
    let mut z: Vec<f32> = m.now.iter().map(|v| v / 2.0).collect();
    let land: Vec<usize> = (0..w * h).filter(|&i| weight[i] > 0.5 && m.now[i] > SEA + 2.0).collect();
    let drops = (land.len() / 6).min(1_500_000);
    let grad = |z: &[f32], x: f32, y: f32| -> (f32, f32, f32) {
        let (ix, iy) = (x as usize, y as usize);
        let (u, v) = (x - ix as f32, y - iy as f32);
        let i = iy * w + ix;
        let (a, b, c, d) = (z[i], z[i + 1], z[i + w], z[i + w + 1]);
        ((b - a) * (1.0 - v) + (d - c) * v, (c - a) * (1.0 - u) + (d - b) * u, a * (1.0 - u) * (1.0 - v) + b * u * (1.0 - v) + c * (1.0 - u) * v + d * u * v)
    };
    for n in 0..drops {
        let start = land[(hash(n as i32, 7, 101) * land.len() as f32) as usize % land.len()];
        let (mut x, mut y) = ((start % w) as f32 + hash(n as i32, 1, 102), (start / w) as f32 + hash(n as i32, 2, 103));
        let (mut dx, mut dy, mut speed, mut water, mut sediment) = (0f32, 0f32, 1f32, 1f32, 0f32);
        for _ in 0..90 {
            if x < 1.0 || y < 1.0 || x >= (w - 2) as f32 || y >= (h - 2) as f32 {
                break;
            }
            let (gx, gy, hz) = grad(&z, x, y);
            dx = dx * 0.05 - gx * 0.95;
            dy = dy * 0.05 - gy * 0.95;
            let len = (dx * dx + dy * dy).sqrt();
            if len < 1e-6 {
                break;
            }
            dx /= len;
            dy /= len;
            let (ox, oy) = (x, y);
            x += dx;
            y += dy;
            if x < 1.0 || y < 1.0 || x >= (w - 2) as f32 || y >= (h - 2) as f32 {
                break;
            }
            let (_, _, nz) = grad(&z, x, y);
            let dh = nz - hz;
            let capacity = (-dh).max(0.01) * speed * water * 4.0;
            let (ix, iy) = (ox as usize, oy as usize);
            let (u, v) = (ox - ix as f32, oy - iy as f32);
            let i = iy * w + ix;
            if sediment > capacity || dh > 0.0 {
                let drop = if dh > 0.0 { dh.min(sediment) } else { (sediment - capacity) * 0.3 };
                sediment -= drop;
                for (k, f) in [(i, (1.0 - u) * (1.0 - v)), (i + 1, u * (1.0 - v)), (i + w, (1.0 - u) * v), (i + w + 1, u * v)] {
                    z[k] += drop * f * weight[k];
                }
            } else {
                let take = ((capacity - sediment) * 0.3).min(-dh);
                // spread the pick-up over a small disc
                let mut wsum = 0f32;
                let mut cells = [(0usize, 0f32); 25];
                let mut nc = 0;
                for ry in -2i32..=2 {
                    for rx in -2i32..=2 {
                        let (cx, cy) = (ix as i32 + rx, iy as i32 + ry);
                        if cx < 0 || cy < 0 || cx >= w as i32 || cy >= h as i32 {
                            continue;
                        }
                        let d = ((rx as f32 - u).powi(2) + (ry as f32 - v).powi(2)).sqrt();
                        let f = (2.5 - d).max(0.0);
                        if f > 0.0 {
                            cells[nc] = (cy as usize * w + cx as usize, f);
                            nc += 1;
                            wsum += f;
                        }
                    }
                }
                for &(k, f) in &cells[..nc] {
                    let t = take * f / wsum * weight[k];
                    z[k] -= t;
                    sediment += t;
                }
            }
            speed = (speed * speed + dh.abs() * 4.0).sqrt().min(8.0);
            water *= 0.98;
        }
    }
    // 2. slumping: slopes steeper than ~40 degrees give material to their lowest neighbour
    let talus = 0.85f32;
    for _ in 0..40 {
        let mut delta = vec![0f32; w * h];
        for y in 1..h - 1 {
            for x in 1..w - 1 {
                let i = y * w + x;
                if weight[i] <= 0.0 {
                    continue;
                }
                let mut low = (0f32, 0usize);
                for (dx, dy, dist) in [(-1i32, 0i32, 1.0f32), (1, 0, 1.0), (0, -1, 1.0), (0, 1, 1.0), (-1, -1, 1.414), (1, -1, 1.414), (-1, 1, 1.414), (1, 1, 1.414)] {
                    let j = (y as i32 + dy) as usize * w + (x as i32 + dx) as usize;
                    let d = (z[i] - z[j]) / dist;
                    if d > low.0 {
                        low = (d, j);
                    }
                }
                if low.0 > talus {
                    let mv = (low.0 - talus) * 0.25 * weight[i].min(weight[low.1].max(0.2));
                    delta[i] -= mv;
                    delta[low.1] += mv;
                }
            }
        }
        for (a, d) in z.iter_mut().zip(&delta) {
            *a += d;
        }
    }
    // 3. a light smoothing of the land (keeps sharp water lines from the sea floor)
    let smooth = box_blur(&z, w, h, 1);
    for i in 0..w * h {
        let k = 0.35 * weight[i];
        z[i] += (smooth[i] - z[i]) * k;
    }
    for i in 0..w * h {
        m.now[i] = z[i] * 2.0;
    }
    let mut most = 0f32;
    for (c, (dir, bytes, cell)) in &originals {
        let mut edited = cell.clone();
        for iy in 0..GRID {
            for ix in 0..GRID {
                let i = m.at((c.0 - m.x0) as usize * 512 + ix, (c.1 - m.y0) as usize * 512 + iy);
                most = most.max((m.now[i] - before[i]).abs());
                edited.heights[iy * GRID + ix] = m.now[i];
            }
        }
        let written = heightmap_write::write(bytes, &edited)?;
        let p = dir.join("terrain/heightmap.dat");
        backup(&p, "before-erode")?;
        std::fs::write(&p, written)?;
    }
    println!("erode: {} cells, {drops} droplets, largest change {most:.1} m", originals.len());
    Ok(())
}

fn bake(catalog_path: &Path, dirs: &[PathBuf]) -> Result<()> {
    let pak = aa_pak::Pak::global();
    let table: BTreeMap<String, Entry> = serde_json::from_str(&std::fs::read_to_string(catalog_path)?)?;
    let mut tiles: HashMap<(i32, i32, usize, usize), Vec<[u8; 3]>> = HashMap::new();
    let mut missing: BTreeMap<String, usize> = BTreeMap::new();
    for dir in dirs {
        let c = cell_of(dir)?;
        let h = parse_heights(&std::fs::read(dir.join("terrain/heightmap.dat"))?)?;
        let h0 = parse_heights(&pak.read(&pak_path(c, "terrain/heightmap.dat"))?)?;
        let mats = slot_materials(&std::fs::read_to_string(dir.join("leveldata.xml"))?);
        let mats0 = slot_materials(&pak.read_string(&pak_path(c, "leveldata.xml"))?);
        let pristine_bytes = pak.read(&pak_path(c, "terrain/cover.ctc"))?;
        let cover_path = dir.join("terrain/cover.ctc");
        let current_bytes = if cover_path.exists() { std::fs::read(&cover_path)? } else { pristine_bytes.clone() };
        let current = Sampler::new(&current_bytes)?;
        let pristine = Sampler::new(&pristine_bytes)?;
        // generated samples: other height or other material than the pak's
        let generated: Vec<f32> = (0..GRID * GRID)
            .map(|i| {
                let changed = (h.heights[i] - h0.heights[i]).abs() > 0.05 || mats.get(h.surface[i] as usize) != mats0.get(h0.surface[i] as usize);
                if changed { 1.0 } else { 0.0 }
            })
            .collect();
        let gen_w = box_blur(&generated, GRID, GRID, 3);
        // material weights: one grid per material, softened
        let mut keys: Vec<String> = Vec::new();
        for i in 0..GRID * GRID {
            if generated[i] > 0.0
                && let Some(m) = mats.get(h.surface[i] as usize)
                && !keys.contains(m)
            {
                keys.push(m.clone());
            }
        }
        let weights: Vec<Vec<f32>> = keys
            .iter()
            .map(|k| {
                let one: Vec<f32> = (0..GRID * GRID).map(|i| if mats.get(h.surface[i] as usize) == Some(k) { 1.0 } else { 0.0 }).collect();
                box_blur(&box_blur(&one, GRID, GRID, 1), GRID, GRID, 1)
            })
            .collect();
        let chosen: Vec<Option<Chosen>> = keys
            .iter()
            .map(|k| match table.get(k) {
                Some(e) => Some(choose(e, &mut tiles, c)),
                None => {
                    *missing.entry(k.clone()).or_default() += 1;
                    None
                }
            })
            .collect();
        // hollows: how far a sample lies below its surroundings (about 16 m around)
        let around = box_blur(&box_blur(&h.heights, GRID, GRID, 4), GRID, GRID, 4);
        let (wx0, wy0) = (c.0 as f32 * 1024.0, c.1 as f32 * 1024.0);
        let mut fine = vec![[0f32; 3]; FINE * FINE];
        let bilinear = |g: &[f32], sx: f32, sy: f32| -> f32 {
            let (sx, sy) = (sx.clamp(0.0, (GRID - 1) as f32 - 0.001), sy.clamp(0.0, (GRID - 1) as f32 - 0.001));
            let (ix, iy) = (sx as usize, sy as usize);
            let (u, v) = (sx - ix as f32, sy - iy as f32);
            let i = iy * GRID + ix;
            (g[i] * (1.0 - u) + g[i + 1] * u) * (1.0 - v) + (g[i + GRID] * (1.0 - u) + g[i + GRID + 1] * u) * v
        };
        for row in 0..FINE {
            for col in 0..FINE {
                let (lx, ly) = ((row as f32 + 0.5) * 0.5, (col as f32 + 0.5) * 0.5);
                let (sx, sy) = (lx / 2.0, ly / 2.0);
                let g = bilinear(&gen_w, sx, sy);
                let old = current.at(lx, ly);
                if g <= 0.0 {
                    fine[row * FINE + col] = old;
                    continue;
                }
                let (x, y) = (wx0 + lx, wy0 + ly);
                // irregular borders between materials
                let (wx, wy) = (sx + 1.6 * noise(x / 7.0, y / 7.0, 11) + 0.6 * noise(x / 2.0, y / 2.0, 12), sy + 1.6 * noise(x / 7.0, y / 7.0, 13) + 0.6 * noise(x / 2.0, y / 2.0, 14));
                let mut acc = [0f32; 3];
                let mut wsum = 0f32;
                for (k, ch) in chosen.iter().enumerate() {
                    let wgt = bilinear(&weights[k], wx, wy);
                    if wgt < 0.02 {
                        continue;
                    }
                    // a material without donor ground (under water: coral) gets the original sea floor colour
                    let col_m = match ch {
                        Some(ch) => ground(ch, x, y, k as u32),
                        None => pristine.at(lx, ly),
                    };
                    for j in 0..3 {
                        acc[j] += col_m[j] * wgt;
                    }
                    wsum += wgt;
                }
                if wsum <= 0.0 {
                    fine[row * FINE + col] = old;
                    continue;
                }
                let mut px = acc.map(|v| v / wsum);
                let z = bilinear(&h.heights, sx, sy);
                let hollow = bilinear(&around, sx, sy) - z;
                px[2] *= 1.0 - (hollow / 14.0).clamp(0.0, 0.22) + (-hollow / 30.0).clamp(0.0, 0.05);
                // ground gets darker toward the water line (wet) and below it fades into the sea floor colour
                let wet = 1.0 - 0.35 * (1.0 - ((z - SEA) / 3.0).clamp(0.0, 1.0));
                px[2] *= wet;
                let k = g * ((z - (SEA - 12.0)) / 10.0).clamp(0.0, 1.0);
                fine[row * FINE + col] = [0, 1, 2].map(|j| old[j] + (px[j] - old[j]) * k);
            }
        }
        let out = cover_out::build(&pristine_bytes, fine)?;
        backup(&cover_path, "before-bake")?;
        std::fs::write(&cover_path, out)?;
        println!("{:03}_{:03}: baked {} material(s)", c.0, c.1, keys.len());
    }
    for (m, n) in missing {
        println!("no donor ground for {m} ({n} cell(s)): the original sea floor colour there");
    }
    Ok(())
}

/// The donor sectors a cell uses for one material: the nearest ones whose colour agrees, and their common colour.
struct Chosen {
    tiles: Vec<(Vec<[u8; 3]>, [f32; 3])>,
    target: [f32; 3],
}

fn choose(e: &Entry, tiles: &mut HashMap<(i32, i32, usize, usize), Vec<[u8; 3]>>, cell: (i32, i32)) -> Chosen {
    let pak = aa_pak::Pak::global();
    let mut near: Vec<&(i32, i32, usize, usize, f32)> = e.donors.iter().collect();
    near.sort_by_key(|d| (d.0 - cell.0).pow(2) + (d.1 - cell.1).pow(2));
    let mut picked: Vec<(Vec<[u8; 3]>, [f32; 3])> = Vec::new();
    for d in near.into_iter().take(10) {
        let key = (d.0, d.1, d.2, d.3);
        if !tiles.contains_key(&key)
            && let Some(t) = donor_tile(pak, d)
        {
            tiles.insert(key, t);
        }
        if let Some(t) = tiles.get(&key) {
            let n = t.len() as f32;
            let mean = [0, 1, 2].map(|j| t.iter().map(|p| p[j] as f32).sum::<f32>() / n);
            picked.push((t.clone(), mean));
        }
    }
    if picked.is_empty() {
        return Chosen { tiles: Vec::new(), target: e.mean };
    }
    // drop donors whose brightness or tint is far from the median of the nearest ones
    let median = |j: usize| {
        let mut v: Vec<f32> = picked.iter().map(|p| p.1[j]).collect();
        v.sort_by(f32::total_cmp);
        v[v.len() / 2]
    };
    let med = [median(0), median(1), median(2)];
    picked.retain(|p| (p.1[2] - med[2]).abs() <= med[2] * 0.3 + 3.0 && (p.1[0] - med[0]).abs() + (p.1[1] - med[1]).abs() <= 14.0);
    // the calmest ones: busy sectors (flower spots, paths) repeat visibly
    let busy = |t: &(Vec<[u8; 3]>, [f32; 3])| t.0.iter().map(|p| (p[2] as f32 - t.1[2]).powi(2) + (p[0] as f32 - t.1[0]).powi(2) + (p[1] as f32 - t.1[1]).powi(2)).sum::<f32>();
    picked.sort_by(|a, b| busy(a).total_cmp(&busy(b)));
    picked.truncate(4);
    Chosen { tiles: picked, target: med }
}

/// A material's ground colour at world `(x, y)`: donor sectors laid out on a 64 m grid, a second grid offset by half a
/// sector blended in so the sector edges do not show, each sector picked, flipped and turned by a hash.
fn ground(chosen: &Chosen, x: f32, y: f32, seed: u32) -> [f32; 3] {
    let donors = &chosen.tiles;
    if donors.is_empty() {
        return chosen.target;
    }
    let mut acc = [0f32; 3];
    let mut wsum = 0f32;
    for (layer, shift) in [(0u32, 0.0f32), (1, 32.0)] {
        let (gx, gy) = ((x + shift) / 64.0, (y + shift) / 64.0);
        let (bx, by) = (gx.floor() as i32, gy.floor() as i32);
        let (fx, fy) = (gx - bx as f32, gy - by as f32);
        let edge = fx.min(1.0 - fx).min(fy).min(1.0 - fy).max(0.001);
        let r = hash(bx, by, 300 + seed * 7 + layer);
        let (t, mean) = &donors[(r * donors.len() as f32) as usize % donors.len()];
        let bits = (hash(bx, by, 400 + seed * 7 + layer) * 8.0) as u32;
        let (mut u, mut v) = (fx, fy);
        if bits & 1 != 0 {
            u = 1.0 - u;
        }
        if bits & 2 != 0 {
            v = 1.0 - v;
        }
        if bits & 4 != 0 {
            std::mem::swap(&mut u, &mut v);
        }
        let (r_, c_) = (((u * TILE_PX as f32) as usize).min(TILE_PX - 1), ((v * TILE_PX as f32) as usize).min(TILE_PX - 1));
        // the donor's own detail, softened, around the material's colour near here
        let px = t[r_ * TILE_PX + c_];
        for j in 0..3 {
            acc[j] += (chosen.target[j] + (px[j] as f32 - mean[j]) * 0.55) * edge;
        }
        wsum += edge;
    }
    acc.map(|v| v / wsum)
}

fn main() -> Result<()> {
    unsafe { std::env::remove_var("AA_PAK_OVERLAY") };
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("catalog") => catalog(Path::new(args.get(1).context("catalog <out.json>")?)),
        Some("erode") => erode(&args[1..].iter().map(PathBuf::from).collect::<Vec<_>>()),
        Some("bake") => bake(Path::new(args.get(1).context("bake <catalog.json> <cells>...")?), &args[2..].iter().map(PathBuf::from).collect::<Vec<_>>()),
        _ => bail!("usage: island-finish catalog <out.json> | erode <cells>... | bake <catalog.json> <cells>..."),
    }
}
