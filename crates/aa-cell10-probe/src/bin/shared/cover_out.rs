//! Cover (`cover.ctc`) helpers shared by the terrain tools: sampling a cover, building a complete five-level cover
//! from a 2048 x 2048 texel image, value noise. Texels are in the cover's encoded space (chromaticity r, g and
//! brightness b, 0..255; see `aa_editor::cover_paint`).
use aa_world::cover::{Cover, TILE_PX, bc1_decode, bc1_encode};
use anyhow::{Result, ensure};
use std::collections::HashMap;

pub const LEVELS: usize = 5;
/// Texels per side of the finest level: 0.5 m each over the 1024 m cell.
pub const FINE: usize = TILE_PX << (LEVELS - 1);

/// The colour of a cover at cell-local metres, finest tile available, bilinear (raw encoded values).
pub struct Sampler(Vec<(usize, HashMap<(usize, usize), Vec<[u8; 3]>>)>);
impl Sampler {
    pub fn new(bytes: &[u8]) -> Result<Self> {
        let cover = Cover::parse(bytes).map_err(|e| anyhow::anyhow!("{e}"))?;
        let mut levels = Vec::new();
        for level in (0..=cover.max_level).rev() {
            let mut tiles = HashMap::new();
            for x in 0..1usize << level {
                for y in 0..1usize << level {
                    if let Some(t) = cover.tile(0, level, x, y) {
                        tiles.insert((x, y), bc1_decode(t));
                    }
                }
            }
            levels.push((level, tiles));
        }
        Ok(Sampler(levels))
    }
    pub fn at(&self, lx: f32, ly: f32) -> [f32; 3] {
        for (level, tiles) in &self.0 {
            let tile_m = 1024.0 / (1usize << level) as f32;
            let (tx, ty) = (((lx / tile_m) as usize).min((1 << level) - 1), ((ly / tile_m) as usize).min((1 << level) - 1));
            let Some(px) = tiles.get(&(tx, ty)) else { continue };
            let p = tile_m / TILE_PX as f32;
            let (fr, fc) = (((lx - tx as f32 * tile_m) / p - 0.5).clamp(0.0, (TILE_PX - 1) as f32), ((ly - ty as f32 * tile_m) / p - 0.5).clamp(0.0, (TILE_PX - 1) as f32));
            let (r0, c0) = (fr.floor() as usize, fc.floor() as usize);
            let (r1, c1) = ((r0 + 1).min(TILE_PX - 1), (c0 + 1).min(TILE_PX - 1));
            let (a, b) = (fr - r0 as f32, fc - c0 as f32);
            let g = |r: usize, c: usize, k: usize| px[r * TILE_PX + c][k] as f32;
            return [0, 1, 2].map(|k| (g(r0, c0, k) * (1.0 - b) + g(r0, c1, k) * b) * (1.0 - a) + (g(r1, c0, k) * (1.0 - b) + g(r1, c1, k) * b) * a);
        }
        [0.0; 3]
    }
}

pub fn hash(x: i32, y: i32, seed: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(374_761_393) ^ (y as u32).wrapping_mul(668_265_263) ^ seed.wrapping_mul(2_246_822_519);
    h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
    h ^= h >> 16;
    h as f32 / u32::MAX as f32
}

pub fn noise(x: f32, y: f32, seed: u32) -> f32 {
    let (ix, iy) = (x.floor() as i32, y.floor() as i32);
    let s = |t: f32| t * t * (3.0 - 2.0 * t);
    let (fx, fy) = (s(x - x.floor()), s(y - y.floor()));
    let a = hash(ix, iy, seed) + (hash(ix + 1, iy, seed) - hash(ix, iy, seed)) * fx;
    let b = hash(ix, iy + 1, seed) + (hash(ix + 1, iy + 1, seed) - hash(ix, iy + 1, seed)) * fx;
    (a + (b - a) * fy) * 2.0 - 1.0
}

/// The encoded texel (0..255) of a gamma-space colour (0..1).
#[allow(dead_code)]
pub fn encode(c: [f32; 3]) -> [f32; 3] {
    let sum = (c[0] + c[1] + c[2]).max(1e-4);
    [((c[0] / sum) * 30.0 / 31.0 - 0.001012).clamp(0.0, 1.0), (c[1] / sum).clamp(0.0, 1.0), (sum / 3.0).clamp(0.0, 1.0)].map(|v| v * 255.0)
}

/// Rebuilds the cover with all levels. `cell` gives the world origin for the variation noise.
/// A complete cover from the finest level `fine` (rows along world X), with the file header (format, flags) of `like`.
pub fn build(like: &[u8], fine: Vec<[f32; 3]>) -> Result<Vec<u8>> {
    // mip chain: level 4 = fine, each coarser level a 2x2 average
    let mut levels = vec![fine];
    for _ in 1..LEVELS {
        let prev = levels.last().unwrap();
        let n = (prev.len() as f32).sqrt() as usize / 2;
        let mut next = vec![[0f32; 3]; n * n];
        for r in 0..n {
            for c in 0..n {
                let s = |dr: usize, dc: usize| prev[(2 * r + dr) * 2 * n + 2 * c + dc];
                next[r * n + c] = [0, 1, 2].map(|k| (s(0, 0)[k] + s(0, 1)[k] + s(1, 0)[k] + s(1, 1)[k]) * 0.25);
            }
        }
        levels.push(next);
    }
    levels.reverse(); // index = level
    // table (depth first) and tiles
    let mut table: Vec<u16> = Vec::new();
    let mut tiles: Vec<Vec<u8>> = Vec::new();
    fn walk(level: usize, x: usize, y: usize, levels: &[Vec<[f32; 3]>], table: &mut Vec<u16>, tiles: &mut Vec<Vec<u8>>) {
        let n = TILE_PX << level;
        let img = &levels[level];
        let mut px = Vec::with_capacity(TILE_PX * TILE_PX);
        for r in 0..TILE_PX {
            for c in 0..TILE_PX {
                px.push(img[(x * TILE_PX + r) * n + y * TILE_PX + c].map(|v| v.round().clamp(0.0, 255.0) as u8));
            }
        }
        table.push(tiles.len() as u16);
        tiles.push(bc1_encode(&px));
        if level + 1 < LEVELS {
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                walk(level + 1, 2 * x + dx, 2 * y + dy, levels, table, tiles);
            }
        } else {
            for _ in 0..4 {
                table.push(0xFFFF);
            }
        }
    }
    walk(0, 0, 0, &levels, &mut table, &mut tiles);
    // header up to and including the layer record, from the old file
    let header_len = 16 + 12;
    let mut out = like[..header_len].to_vec();
    out.extend_from_slice(&(table.len() as u16).to_le_bytes());
    for t in &table {
        out.extend_from_slice(&t.to_le_bytes());
    }
    for t in &tiles {
        out.extend_from_slice(t);
    }
    let check = Cover::parse(&out).map_err(|e| anyhow::anyhow!("rebuilt cover: {e}"))?;
    ensure!(check.max_level == LEVELS - 1, "rebuilt cover has {} levels", check.max_level + 1);
    Ok(out)
}
