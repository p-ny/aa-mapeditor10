//! `zone-map <world> <x> <y> <w> <h> <out.dds> [width height]`: a shaded top-down world-map image of the rectangle
//! (south-west corner x,y and size in world metres, north up), read from the cells' heightmaps through AA_PAK and
//! AA_PAK_OVERLAY. Writes an uncompressed 32-bit DDS (default 928x556, the size of the client's zone maps).
use anyhow::{Context, Result, ensure};
use aa_world::heightmap::CellHeights;
use std::collections::HashMap;
struct Terrain { world: String, cells: HashMap<(i32, i32), Option<CellHeights>> }
impl Terrain {
    /// Height and sea level at a world position; None outside existing cells.
    fn at(&mut self, x: f32, y: f32) -> Option<(f32, f32)> {
        let c = ((x / 1024.0).floor() as i32, (y / 1024.0).floor() as i32);
        let world = self.world.clone();
        let h = self.cells.entry(c).or_insert_with(|| {
            let path = format!("game/worlds/{world}/cells/{:03}_{:03}/client/terrain/heightmap.dat", c.0, c.1);
            aa_pak::Pak::global().read(&path).ok().and_then(|b| CellHeights::parse(&b).ok())
        }).as_ref()?;
        Some((h.sample(x - c.0 as f32 * 1024.0, y - c.1 as f32 * 1024.0), h.ocean_level))
    }
}
fn colour(z: f32, sea: f32, shade: f32) -> [f32; 3] {
    if z < sea {
        let d = ((sea - z) / 60.0).clamp(0.0, 1.0);
        return [0.33 - 0.15 * d, 0.55 - 0.2 * d, 0.68 - 0.18 * d];
    }
    let t = ((z - sea) / 600.0).clamp(0.0, 1.0);
    let low = [0.47, 0.58, 0.33];
    let high = [0.62, 0.55, 0.42];
    let base: Vec<f32> = (0..3).map(|i| low[i] + (high[i] - low[i]) * t).collect();
    [base[0] * shade, base[1] * shade, base[2] * shade]
}
fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().skip(1).collect();
    ensure!(a.len() == 6 || a.len() == 8, "zone-map <world> <x> <y> <w> <h> <out.dds> [width height]");
    let n = |i: usize| a[i].parse::<f32>().with_context(|| format!("argument {i}"));
    let (x0, y0, w, h) = (n(1)?, n(2)?, n(3)?, n(4)?);
    let (width, height) = if a.len() == 8 { (a[6].parse::<u32>()?, a[7].parse::<u32>()?) } else { (928, 556) };
    ensure!(w > 0.0 && h > 0.0 && width > 0 && height > 0, "empty map rectangle");
    let mut terrain = Terrain { world: a[0].clone(), cells: HashMap::new() };
    let step = (w / width as f32).max(2.0);
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for j in 0..height {
        for i in 0..width {
            let x = x0 + (i as f32 + 0.5) / width as f32 * w;
            let y = y0 + h - (j as f32 + 0.5) / height as f32 * h;
            let rgb = match terrain.at(x, y) {
                Some((z, sea)) => {
                    // light from the north-west
                    let zx = terrain.at(x + step, y).map_or(z, |v| v.0) - terrain.at(x - step, y).map_or(z, |v| v.0);
                    let zy = terrain.at(x, y + step).map_or(z, |v| v.0) - terrain.at(x, y - step).map_or(z, |v| v.0);
                    let shade = (1.0 + (zx - zy) / (4.0 * step) * -0.6).clamp(0.55, 1.25);
                    colour(z, sea, shade)
                }
                None => [0.2, 0.22, 0.25],
            };
            let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0) as u8;
            pixels.extend([b(rgb[2]), b(rgb[1]), b(rgb[0]), 255]);
        }
    }
    let mut dds = b"DDS ".to_vec();
    let mut header = [0u32; 31];
    header[0] = 124;
    header[1] = 0x1 | 0x2 | 0x4 | 0x8 | 0x1000; // caps, height, width, pitch, pixel format
    header[2] = height;
    header[3] = width;
    header[4] = width * 4;
    header[18] = 32; // pixel format size
    header[19] = 0x1 | 0x40; // alpha pixels, RGB
    header[21] = 32;
    header[22] = 0x00ff_0000;
    header[23] = 0x0000_ff00;
    header[24] = 0x0000_00ff;
    header[25] = 0xff00_0000;
    header[26] = 0x1000; // texture
    for v in header {
        dds.extend(v.to_le_bytes());
    }
    dds.extend(pixels);
    std::fs::write(&a[5], dds)?;
    println!("wrote {} ({width}x{height})", a[5]);
    Ok(())
}
