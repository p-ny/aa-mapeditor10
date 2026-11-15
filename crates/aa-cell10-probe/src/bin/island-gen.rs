//! `island-gen <mask.txt> <overlay> [preview.ppm]`: raises the island of the Haranya map art north-east of the
//! continent (Saltswept Atoll waters) into the main_world cells of a pak overlay: heights, painted surfaces (new
//! surface types appended to each cell's `leveldata.xml`) and cover colour. Reads the original cells from AA_PAK
//! (without overlay) and keeps every original height that is above the generated one, so existing land is untouched.
//!
//! The terrain: the map-art outline with a fractal coast, sea cliffs on the north side and beaches and dunes on the
//! south, a ridge spine, a terraced mesa in the west, a breached volcano in the east with a crater, a gorge from the
//! crater down to a sea inlet, a hidden cove behind the north cliffs, sea stacks and an islet.
use aa_world::cover::{Cover, CoverFormat, TILE_PX, bc1_decode, bc1_encode};
use aa_world::heightmap::{CellHeights, GRID};
use anyhow::{Context, Result};
use std::{collections::HashMap, fs, path::PathBuf};
#[path = "../../../aa-editor/src/heightmap_write.rs"]
mod heightmap_write;

const WORLD: &str = "main_world";
// map art pixel -> world metres (fitted against the terrain), plus the shift that keeps the island in zone 261
const SX: f32 = 27.7;
const SY: f32 = 27.5;
const OX: f32 = 10500.0 + 350.0;
const OY: f32 = 17400.0;
// field grid
const FX0: f32 = 24.0 * 1024.0;
const FY0: f32 = 13.0 * 1024.0;
const STEP: f32 = 4.0;
const NX: usize = 6 * 256;
const NY: usize = 4 * 256;
const SEA: f32 = 100.0;

// surface palette: (name, donor cell, cover colour)
const PALETTE: [(&str, &str, [u8; 3]); 10] = [
    ("beach_white_sand_wet", "021_018", [176, 164, 128]),
    ("tr_wt_sand_800a", "014_009", [205, 190, 150]),
    ("tr_field_grass_001a_veg", "", [74, 100, 46]),
    ("tr_field_grass_004a", "", [100, 122, 62]),
    ("tr_mix_rc_gr_211a", "", [112, 112, 90]),
    ("tr_canyon_rock3d_302a", "024_011", [118, 112, 104]),
    ("tr_field_dirt_001a", "", [125, 104, 78]),
    ("tr_wasteland_dirt_801a", "", [92, 86, 80]),
    ("tr_canyon_rock3d_808a", "", [58, 52, 50]),
    ("tr_beach_coral_201b", "020_016", [160, 160, 125]),
];
const WET: usize = 0;
const DUNE: usize = 1;
const LUSH: usize = 2;
const GRASS: usize = 3;
const ROCKGRASS: usize = 4;
const ROCK: usize = 5;
const DIRT: usize = 6;
const ASH: usize = 7;
const BASALT: usize = 8;
const CORAL: usize = 9;

fn hash(x: i32, y: i32, seed: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(374_761_393) ^ (y as u32).wrapping_mul(668_265_263) ^ seed.wrapping_mul(2_246_822_519);
    h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
    h ^= h >> 16;
    h as f32 / u32::MAX as f32
}

fn smooth(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

fn sstep(a: f32, b: f32, x: f32) -> f32 {
    smooth(((x - a) / (b - a)).clamp(0.0, 1.0))
}

/// Value noise in -1..1.
fn noise(x: f32, y: f32, seed: u32) -> f32 {
    let (ix, iy) = (x.floor() as i32, y.floor() as i32);
    let (fx, fy) = (smooth(x - x.floor()), smooth(y - y.floor()));
    let a = hash(ix, iy, seed) + (hash(ix + 1, iy, seed) - hash(ix, iy, seed)) * fx;
    let b = hash(ix, iy + 1, seed) + (hash(ix + 1, iy + 1, seed) - hash(ix, iy + 1, seed)) * fx;
    (a + (b - a) * fy) * 2.0 - 1.0
}

fn fbm(x: f32, y: f32, octaves: u32, seed: u32) -> f32 {
    let (mut sum, mut amp, mut f, mut norm) = (0.0, 1.0, 1.0, 0.0);
    for o in 0..octaves {
        sum += noise(x * f, y * f, seed + o) * amp;
        norm += amp;
        amp *= 0.5;
        f *= 2.03;
    }
    sum / norm
}

/// Sharp crests, 0..1.
fn ridged(x: f32, y: f32, octaves: u32, seed: u32) -> f32 {
    let (mut sum, mut amp, mut f, mut norm) = (0.0, 1.0, 1.0, 0.0);
    for o in 0..octaves {
        let n = 1.0 - noise(x * f, y * f, seed + o).abs();
        sum += n * n * amp;
        norm += amp;
        amp *= 0.5;
        f *= 2.1;
    }
    sum / norm
}

fn seg_dist(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> (f32, f32) {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let t = (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
    (((p.0 - a.0 - t * dx).powi(2) + (p.1 - a.1 - t * dy).powi(2)).sqrt(), t)
}

const VOLCANO: (f32, f32) = (28150.0, 15850.0);
const MESA: (f32, f32) = (26650.0, 15950.0);
const COVE: (f32, f32) = (27330.0, 16170.0);
const ISLET: (f32, f32) = (29330.0, 14820.0);
/// Rock needles along the ridge: (x, y, radius, height above the ground).
const NEEDLES: [(f32, f32, f32, f32); 6] = [
    (27080.0, 15760.0, 22.0, 85.0),
    (27190.0, 15830.0, 15.0, 60.0),
    (27300.0, 15720.0, 18.0, 95.0),
    (27420.0, 15900.0, 13.0, 55.0),
    (25980.0, 15880.0, 16.0, 50.0),
    (26100.0, 15820.0, 11.0, 40.0),
];
const SPINE: [(f32, f32); 3] = [(25750.0, 15950.0), (26650.0, 15950.0), (28150.0, 15850.0)];

/// The gorge's centre line from the sea inlet (t = 0) up to the crater breach (t = 1).
fn gorge_point(t: f32) -> (f32, f32) {
    let (a, b) = ((27930.0, 14650.0), (VOLCANO.0, VOLCANO.1 - 150.0));
    (a.0 + (b.0 - a.0) * t + 140.0 * (t * 9.0).sin() * (1.0 - t), a.1 + (b.1 - a.1) * t)
}

struct Gorge(Vec<(f32, f32)>);
impl Gorge {
    fn new() -> Self {
        Gorge((0..=200).map(|i| gorge_point(i as f32 / 200.0)).collect())
    }
    /// (distance to the centre line, position along it 0..1)
    fn at(&self, p: (f32, f32)) -> (f32, f32) {
        let mut best = (f32::MAX, 0.0);
        for (i, w) in self.0.windows(2).enumerate() {
            let (d, t) = seg_dist(p, w[0], w[1]);
            if d < best.0 {
                best = (d, (i as f32 + t) / (self.0.len() - 1) as f32);
            }
        }
        best
    }
}

struct Field {
    /// Signed distance to the coast in metres, positive on land.
    dist: Vec<f32>,
    stacks: Vec<(f32, f32, f32, f32)>,
    gorge: Gorge,
}

fn load_mask(path: &str) -> Result<(Vec<f32>, usize, usize)> {
    let (w, h) = (928usize, 556usize);
    let mut m = vec![0f32; w * h];
    for l in fs::read_to_string(path)?.lines() {
        let mut it = l.split_whitespace().map(|v| v.parse::<usize>());
        if let (Some(Ok(x)), Some(Ok(y))) = (it.next(), it.next()) {
            m[y * w + x] = 1.0;
        }
    }
    // fill pin holes (the art's texture), then soften the outline
    for _ in 0..2 {
        let src = m.clone();
        for y in 1..h - 1 {
            for x in 1..w - 1 {
                let mut s = 0.0;
                for (dx, dy) in [(-1i32, -1i32), (0, -1), (1, -1), (-1, 0), (0, 0), (1, 0), (-1, 1), (0, 1), (1, 1)] {
                    s += src[(y as i32 + dy) as usize * w + (x as i32 + dx) as usize];
                }
                m[y * w + x] = s / 9.0;
            }
        }
    }
    Ok((m, w, h))
}

fn mask_at(m: &(Vec<f32>, usize, usize), x: f32, y: f32) -> f32 {
    let (px, py) = ((x - OX) / SX - 0.5, (OY - y) / SY - 0.5);
    let (ix, iy) = (px.floor() as i32, py.floor() as i32);
    let get = |a: i32, b: i32| if a < 0 || b < 0 || a >= m.1 as i32 || b >= m.2 as i32 { 0.0 } else { m.0[b as usize * m.1 + a as usize] };
    let (fx, fy) = (px - ix as f32, py - iy as f32);
    let top = get(ix, iy) + (get(ix + 1, iy) - get(ix, iy)) * fx;
    let bottom = get(ix, iy + 1) + (get(ix + 1, iy + 1) - get(ix, iy + 1)) * fx;
    top + (bottom - top) * fy
}

/// Distance (metres) of every grid point to the nearest point where `inside` is false (0 outside).
fn distance(inside: &[bool]) -> Vec<f32> {
    let big = 1e9;
    let mut d: Vec<f32> = inside.iter().map(|&i| if i { big } else { 0.0 }).collect();
    let (a, b) = (STEP, STEP * std::f32::consts::SQRT_2);
    for y in 0..NY {
        for x in 0..NX {
            let i = y * NX + x;
            let mut v = d[i];
            if x > 0 { v = v.min(d[i - 1] + a); }
            if y > 0 {
                v = v.min(d[i - NX] + a);
                if x > 0 { v = v.min(d[i - NX - 1] + b); }
                if x + 1 < NX { v = v.min(d[i - NX + 1] + b); }
            }
            d[i] = v;
        }
    }
    for y in (0..NY).rev() {
        for x in (0..NX).rev() {
            let i = y * NX + x;
            let mut v = d[i];
            if x + 1 < NX { v = v.min(d[i + 1] + a); }
            if y + 1 < NY {
                v = v.min(d[i + NX] + a);
                if x + 1 < NX { v = v.min(d[i + NX + 1] + b); }
                if x > 0 { v = v.min(d[i + NX - 1] + b); }
            }
            d[i] = v;
        }
    }
    d
}

fn build_field(mask: &(Vec<f32>, usize, usize)) -> Field {
    let mut land = vec![false; NX * NY];
    for y in 0..NY {
        for x in 0..NX {
            let (wx, wy) = (FX0 + x as f32 * STEP, FY0 + y as f32 * STEP);
            let mut s = mask_at(mask, wx, wy) - 0.5 + 0.30 * fbm(wx / 420.0, wy / 420.0, 3, 11) + 0.12 * fbm(wx / 70.0, wy / 70.0, 3, 23);
            // the hidden cove behind the north cliffs and its narrow mouth
            let rc = ((wx - COVE.0).powi(2) + (wy - COVE.1).powi(2)).sqrt() + 25.0 * fbm(wx / 40.0, wy / 40.0, 2, 5);
            s -= 1.5 * sstep(135.0, 95.0, rc);
            let (dm, t) = seg_dist((wx, wy), COVE, (COVE.0 - 40.0, COVE.1 + 500.0));
            if t > 0.05 {
                s -= 1.5 * sstep(22.0, 12.0, dm + 6.0 * fbm(wx / 25.0, wy / 25.0, 2, 6));
            }
            // the islet
            let ri = ((wx - ISLET.0).powi(2) + ((wy - ISLET.1) * 1.4).powi(2)).sqrt();
            s += 1.2 * sstep(170.0, 90.0, ri + 40.0 * fbm(wx / 60.0, wy / 60.0, 3, 9));
            land[y * NX + x] = s > 0.0;
        }
    }
    let to_sea = distance(&land);
    let sea: Vec<bool> = land.iter().map(|l| !l).collect();
    let to_land = distance(&sea);
    let dist: Vec<f32> = (0..NX * NY).map(|i| if land[i] { to_sea[i] - STEP * 0.5 } else { -(to_land[i] - STEP * 0.5) }).collect();
    // sea stacks off the cliff coast, at least 140 m apart
    let mut stacks: Vec<(f32, f32, f32, f32)> = Vec::new();
    for y in (0..NY).step_by(5) {
        for x in (0..NX).step_by(5) {
            let d = dist[y * NX + x];
            let (wx, wy) = (FX0 + x as f32 * STEP, FY0 + y as f32 * STEP);
            if !(-190.0..-70.0).contains(&d) || cliff(wx, wy) < 0.7 || hash(x as i32, y as i32, 77) > 0.02 {
                continue;
            }
            if stacks.iter().all(|s| (s.0 - wx).powi(2) + (s.1 - wy).powi(2) > 260.0f32.powi(2)) {
                let r = 12.0 + 16.0 * hash(x as i32, y as i32, 78);
                let top = SEA + 35.0 + 75.0 * hash(x as i32, y as i32, 79);
                stacks.push((wx, wy, r, top));
            }
        }
    }
    Field { dist, stacks, gorge: Gorge::new() }
}

/// 1 on the cliff coast (north and the volcano's east flank), 0 on the beach coast.
fn cliff(x: f32, y: f32) -> f32 {
    let north = (y - 15650.0) / 500.0 + 0.9 * fbm(x / 900.0, y / 900.0, 2, 31);
    let east = (x - 28700.0) / 400.0 + 0.6 * fbm(x / 700.0, y / 700.0, 2, 32);
    sstep(-0.4, 0.6, north.max(east * 0.8))
}

impl Field {
    fn dist_at(&self, x: f32, y: f32) -> f32 {
        let (gx, gy) = (((x - FX0) / STEP).clamp(0.0, (NX - 2) as f32), ((y - FY0) / STEP).clamp(0.0, (NY - 2) as f32));
        let (ix, iy) = (gx.floor() as usize, gy.floor() as usize);
        let (fx, fy) = (gx - ix as f32, gy - iy as f32);
        let d = |a: usize, b: usize| self.dist[b * NX + a];
        let top = d(ix, iy) + (d(ix + 1, iy) - d(ix, iy)) * fx;
        let bottom = d(ix, iy + 1) + (d(ix + 1, iy + 1) - d(ix, iy + 1)) * fx;
        top + (bottom - top) * fy
    }

    /// Generated height and surface (palette index) at world `(x, y)`.
    fn sample(&self, x: f32, y: f32) -> (f32, usize) {
        let d = self.dist_at(x, y);
        let c = cliff(x, y);
        let detail = fbm(x / 23.0, y / 23.0, 3, 41);
        let (mut z, mut surf);
        if d < 0.0 {
            let t = -d;
            let shelf = 140.0 + (25.0 - 140.0) * c;
            z = if t < shelf { SEA - 2.0 - 12.0 * c - 6.0 * (t / shelf).powf(0.8) } else { (SEA - 8.0 - 12.0 * c) * (1.0 - sstep(0.0, 1.0, (t - shelf) / 650.0)) };
            z += 2.5 * fbm(x / 35.0, y / 35.0, 3, 42) * sstep(900.0, 200.0, t);
            surf = if z > SEA - 12.0 { if c > 0.5 { ROCK } else if fbm(x / 60.0, y / 60.0, 2, 43) > 0.0 { CORAL } else { WET } } else { WET };
        } else {
            let wall = 45.0 + 35.0 * fbm(x / 300.0, y / 300.0, 3, 44);
            let cliff_rise = SEA - 14.0 + (wall + 14.0) * sstep(0.0, 9.0, d) + 15.0 * sstep(9.0, 220.0, d);
            let beach_rise = SEA + 3.5 * sstep(0.0, 70.0, d) + 4.0 * sstep(30.0, 90.0, d) * (0.5 + 0.5 * fbm(x / 45.0, y / 45.0, 2, 45)) + 22.0 * sstep(90.0, 450.0, d);
            z = beach_rise + (cliff_rise - beach_rise) * c;
            let inland = sstep(40.0, 550.0, d);
            let hills = 70.0 * (0.5 + 0.5 * fbm(x / 650.0, y / 650.0, 4, 46)) + 55.0 * ridged(x / 380.0, y / 380.0, 4, 47);
            let spine = SPINE.windows(2).map(|w| seg_dist((x, y), w[0], w[1]).0).fold(f32::MAX, f32::min);
            z += inland * (hills + 150.0 * (-(spine / 450.0).powi(2)).exp());
            // the volcano: cone, rim, crater, breached to the south where the gorge starts
            let (vx, vy) = (x - VOLCANO.0, y - VOLCANO.1);
            let r = (vx * vx + vy * vy).sqrt() + 30.0 * fbm(x / 120.0, y / 120.0, 3, 48);
            let ang = vy.atan2(vx);
            let breach = 1.0 - 0.9 * (-((ang + std::f32::consts::FRAC_PI_2) / 0.35).powi(2)).exp();
            let cone = 470.0 * (1.0 - r / 1150.0).max(0.0).powf(1.6) + 45.0 * (-((r - 210.0) / 70.0).powi(2)).exp() * breach;
            let crater = 150.0 * sstep(215.0, 110.0, r);
            z = z.max(SEA + 6.0 + cone * inland.max(0.35) - crater + 25.0 * ridged(x / 160.0, y / 160.0, 3, 49) * sstep(1150.0, 300.0, r));
            surf = GRASS;
            // the mesa: a flat top ringed by terraces
            let rm = ((x - MESA.0).powi(2) + ((y - MESA.1) * 1.3).powi(2)).sqrt() + 60.0 * fbm(x / 200.0, y / 200.0, 3, 50);
            let wm = sstep(560.0, 380.0, rm);
            if wm > 0.0 {
                let top = 265.0 + 4.0 * detail;
                let flank = z.max(SEA + (top - SEA) * sstep(560.0, 380.0, rm));
                let step = 24.0;
                let k = flank / step;
                let terraced = (k.floor() + sstep(0.72, 1.0, k.fract())) * step;
                z = z + (if rm < 380.0 { top } else { terraced.min(top) } - z) * wm;
            }
            z += 3.0 * detail * inland;
            for &(nx, ny, nr, nh) in &NEEDLES {
                let rn = ((x - nx).powi(2) + (y - ny).powi(2)).sqrt() + 3.0 * fbm(x / 8.0, y / 8.0, 2, 56);
                if rn < nr * 1.5 {
                    z += nh * sstep(nr * 1.5, nr * 0.9, rn) * (1.0 + 0.08 * detail);
                }
            }
            // the gorge from the crater down to an inlet below sea level
            let (dg, t) = self.gorge.at((x, y));
            if dg < 160.0 {
                let floor = SEA - 6.0 + (300.0 - SEA) * t.powf(1.4) + 2.0 * detail;
                let wall = floor + (dg - 14.0 - 6.0 * fbm(x / 30.0, y / 30.0, 2, 51)).max(0.0) * 2.8;
                if wall < z {
                    z = wall;
                    if dg < 22.0 {
                        surf = DIRT;
                    }
                }
            }
            if c < 0.5 && z < SEA + 4.0 && d < 90.0 {
                surf = WET;
            } else if c < 0.5 && z < SEA + 14.0 && d < 160.0 {
                surf = DUNE;
            } else if surf != DIRT {
                surf = if r < 230.0 {
                    BASALT
                } else if r < 750.0 && z > SEA + 160.0 {
                    if fbm(x / 80.0, y / 80.0, 2, 52) > -0.3 { ASH } else { BASALT }
                } else if wm > 0.9 && rm < 380.0 {
                    if fbm(x / 90.0, y / 90.0, 2, 53) > 0.35 { DIRT } else { GRASS }
                } else if fbm(x / 150.0, y / 150.0, 3, 54) > 0.05 {
                    LUSH
                } else {
                    GRASS
                };
            }
        }
        // sea stacks
        for &(sx, sy, sr, top) in &self.stacks {
            let rs = ((x - sx).powi(2) + (y - sy).powi(2)).sqrt() + 4.0 * fbm(x / 9.0, y / 9.0, 2, 55);
            if rs < sr * 1.4 {
                let h = top + 6.0 * detail - 25.0 * sstep(sr * 0.5, sr, rs) * 0.0;
                let w = sstep(sr * 1.4, sr * 0.95, rs);
                let zz = z + (h - z) * w;
                if zz > z {
                    z = zz;
                    surf = if w > 0.98 && rs < sr * 0.6 { GRASS } else { ROCK };
                }
            }
        }
        (z, surf)
    }
}

fn element(pak: &aa_pak::Pak, name: &str, donor: &str, cache: &mut HashMap<String, String>) -> Result<String> {
    let key = format!("{name} {donor}");
    if let Some(e) = cache.get(&key) {
        return Ok(e.clone());
    }
    let mut cells: Vec<String> = if donor.is_empty() { vec![] } else { vec![format!("game/worlds/{WORLD}/cells/{donor}/client/leveldata.xml")] };
    if cells.is_empty() {
        cells = pak.names().filter(|n| n.starts_with(&format!("game/worlds/{WORLD}/cells/")) && n.ends_with("/client/leveldata.xml")).map(String::from).collect();
    }
    for c in cells {
        let Ok(s) = pak.read_string(&c) else { continue };
        if let Some(l) = s.lines().find(|l| l.contains("<SurfaceType ") && l.contains(&format!("Name=\"{name}\""))) {
            // some materials carry vegetation children over several lines; only the element itself is copied
            let mut e = l.trim().to_string();
            if !e.ends_with("/>") {
                e = format!("{}/>", e.trim_end_matches('>'));
            }
            cache.insert(key, e.clone());
            return Ok(e);
        }
    }
    anyhow::bail!("no donor for {name}")
}

/// The cell's slot for each palette entry, appending the missing ones to its `<SurfaceTypes>`.
fn slots(xml: &str, elements: &[String]) -> Result<(String, Vec<u8>)> {
    let mut lines: Vec<String> = xml.lines().map(String::from).collect();
    let end = lines.iter().position(|l| l.contains("</SurfaceTypes>")).context("no </SurfaceTypes>")?;
    let mut existing: Vec<String> = lines[..end].iter().filter(|l| l.contains("<SurfaceType ")).map(|l| l.trim().to_string()).collect();
    let mut out = Vec::new();
    let mut insert = end;
    for e in elements {
        let name = e.split("Name=\"").nth(1).and_then(|r| r.split('"').next()).unwrap_or("");
        if let Some(i) = existing.iter().position(|l| l.contains(&format!("Name=\"{name}\""))) {
            out.push(i as u8);
            continue;
        }
        anyhow::ensure!(existing.len() < 31, "cell has no free surface slot");
        out.push(existing.len() as u8);
        existing.push(e.clone());
        lines.insert(insert, format!("  {e}"));
        insert += 1;
    }
    Ok((lines.join("\n") + "\n", out))
}

fn encode(c: [u8; 3]) -> [u8; 3] {
    let col = c.map(|v| v as f32 / 255.0);
    let sum = col[0] + col[1] + col[2];
    let (r, g) = (col[0] / sum, col[1] / sum);
    [(r * 30.0 / 31.0 - 0.001012).clamp(0.0, 1.0), g.clamp(0.0, 1.0), (sum / 3.0).clamp(0.0, 1.0)].map(|v| (v * 255.0).round() as u8)
}

/// Repaints the cover texels of every level where `colour` gives one.
fn paint_cover(bytes: &[u8], colour: impl Fn(f32, f32) -> Option<[u8; 3]>) -> Option<Vec<u8>> {
    let cover = Cover::parse(bytes).ok()?;
    if cover.layers.first()?.format != CoverFormat::Bc1 {
        return None;
    }
    let base = bytes.as_ptr() as usize;
    let mut out = bytes.to_vec();
    let blocks = TILE_PX / 4;
    let len = blocks * blocks * 8;
    let mut changed = false;
    for level in 0..=cover.max_level {
        let tile_m = 1024.0 / (1usize << level) as f32;
        let px = tile_m / TILE_PX as f32;
        for tx in 0..1usize << level {
            for ty in 0..1usize << level {
                let Some(t) = cover.tile(0, level, tx, ty) else { continue };
                let off = t.as_ptr() as usize - base;
                let mut rgb = bc1_decode(&bytes[off..off + len]);
                let mut any = false;
                for row in 0..TILE_PX {
                    for col in 0..TILE_PX {
                        let (lx, ly) = (tx as f32 * tile_m + (row as f32 + 0.5) * px, ty as f32 * tile_m + (col as f32 + 0.5) * px);
                        if let Some(c) = colour(lx, ly) {
                            rgb[row * TILE_PX + col] = encode(c);
                            any = true;
                        }
                    }
                }
                if any {
                    out[off..off + len].copy_from_slice(&bc1_encode(&rgb));
                    changed = true;
                }
            }
        }
    }
    changed.then_some(out)
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    anyhow::ensure!(args.len() >= 2, "usage: island-gen <mask.txt> <overlay> [preview.ppm]");
    unsafe { std::env::remove_var("AA_PAK_OVERLAY") };
    let pak = aa_pak::Pak::global();
    let overlay = PathBuf::from(&args[1]);
    let mask = load_mask(&args[0])?;
    let field = build_field(&mask);
    println!("{} sea stacks", field.stacks.len());
    let mut cache = HashMap::new();
    let elements: Vec<String> = PALETTE.iter().map(|(n, d, _)| element(pak, n, d, &mut cache)).collect::<Result<_>>()?;
    // preview: 4 m per pixel over the field
    let (pw, ph) = (NX, NY);
    let mut preview = vec![[20u8, 40, 70]; pw * ph];
    for cy in 13..17 {
        for cx in 24..30 {
            let dir = format!("worlds/{WORLD}/cells/{cx:03}_{cy:03}/client");
            let bytes = pak.read(&format!("game/{dir}/terrain/heightmap.dat"))?;
            let original = CellHeights::parse(&bytes).map_err(|e| anyhow::anyhow!("{dir}: {e:?}"))?;
            let mut h = original.clone();
            let mut gen_s = vec![usize::MAX; GRID * GRID];
            let mut raised = 0usize;
            for iy in 0..GRID {
                for ix in 0..GRID {
                    let (wx, wy) = (cx as f32 * 1024.0 + ix as f32 * 2.0, cy as f32 * 1024.0 + iy as f32 * 2.0);
                    let (z, s) = field.sample(wx, wy);
                    let i = iy * GRID + ix;
                    if z > original.heights[i] + 0.05 {
                        h.heights[i] = z;
                        gen_s[i] = s;
                        raised += 1;
                    }
                }
            }
            // steep ground is rock whatever the theme says
            for iy in 1..GRID - 1 {
                for ix in 1..GRID - 1 {
                    let i = iy * GRID + ix;
                    if gen_s[i] == usize::MAX || h.heights[i] < SEA - 1.0 {
                        continue;
                    }
                    let gx = (h.heights[i + 1] - h.heights[i - 1]) / 4.0;
                    let gy = (h.heights[i + GRID] - h.heights[i - GRID]) / 4.0;
                    let slope = (gx * gx + gy * gy).sqrt();
                    if slope > 1.1 {
                        gen_s[i] = if gen_s[i] == BASALT || gen_s[i] == ASH { BASALT } else { ROCK };
                    } else if slope > 0.6 && matches!(gen_s[i], GRASS | LUSH | DUNE) {
                        gen_s[i] = ROCKGRASS;
                    }
                }
            }
            for iy in 0..GRID / 2 {
                for ix in 0..GRID / 2 {
                    let i = iy * 2 * GRID + ix * 2;
                    let (gx, gy) = ((cx - 24) * 256 + ix, (cy - 13) * 256 + iy);
                    let z = h.heights[i];
                    let shade = if ix > 0 && iy > 0 { ((h.heights[i] - h.heights[i - 2 - 2 * GRID]) * 0.35 + 1.0).clamp(0.35, 1.6) } else { 1.0 };
                    let contour = z > SEA && ix > 0 && ((z / 50.0).floor() != (h.heights[i - 2] / 50.0).floor() || (iy > 0 && (z / 50.0).floor() != (h.heights[i - 2 * GRID] / 50.0).floor()));
                    let base = if gen_s[i] != usize::MAX && z >= SEA - 12.0 { PALETTE[gen_s[i]].2 } else if z >= SEA { [120, 120, 110] } else { let k = (z / SEA).clamp(0.0, 1.0); [(20.0 + 60.0 * k) as u8, (50.0 + 90.0 * k) as u8, (90.0 + 90.0 * k) as u8] };
                    let water = z < SEA;
                    preview[(ph - 1 - gy) * pw + gx] = if contour { [30, 30, 30] } else { base.map(|v| if water && gen_s[i] != usize::MAX { ((v as f32 * 0.6 + 40.0) * shade.min(1.1)) as u8 } else { (v as f32 * shade).clamp(0.0, 255.0) as u8 }) };
                }
            }
            if raised == 0 {
                continue;
            }
            let ld_path = format!("game/{dir}/leveldata.xml");
            let (xml, slot) = slots(&pak.read_string(&ld_path)?, &elements)?;
            for i in 0..GRID * GRID {
                if gen_s[i] != usize::MAX {
                    h.surface[i] = slot[gen_s[i]];
                }
            }
            let written = heightmap_write::write(&bytes, &h)?;
            let check = CellHeights::parse(&written).map_err(|e| anyhow::anyhow!("{e:?}"))?;
            let err = check.heights.iter().zip(&h.heights).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
            let out_dir = overlay.join(&dir);
            fs::create_dir_all(out_dir.join("terrain"))?;
            fs::write(out_dir.join("terrain/heightmap.dat"), &written)?;
            fs::write(out_dir.join("leveldata.xml"), xml)?;
            let cover = pak.read(&format!("game/{dir}/terrain/cover.ctc"))?;
            let colour = |lx: f32, ly: f32| {
                let (ix, iy) = (((lx / 2.0).round() as usize).min(GRID - 1), ((ly / 2.0).round() as usize).min(GRID - 1));
                let i = iy * GRID + ix;
                (gen_s[i] != usize::MAX && h.heights[i] > SEA - 14.0).then(|| PALETTE[gen_s[i]].2)
            };
            let covered = match paint_cover(&cover, colour) {
                Some(c) => {
                    fs::write(out_dir.join("terrain/cover.ctc"), c)?;
                    "cover painted"
                }
                None => "cover unchanged",
            };
            println!("{cx:03}_{cy:03}: {raised} samples raised, max {:.0} m, write error {err:.2} m, {covered}", h.heights.iter().cloned().fold(0.0, f32::max));
        }
    }
    if let Some(p) = args.get(2) {
        let mut ppm = format!("P6 {pw} {ph} 255\n").into_bytes();
        for c in &preview {
            ppm.extend_from_slice(c);
        }
        fs::write(p, ppm)?;
    }
    Ok(())
}
