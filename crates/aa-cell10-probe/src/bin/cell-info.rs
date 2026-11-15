//! `cell-info <world> [XXX_YYY ...]`: height range, sea level and land share of cells (read-only; AA_PAK selects the pak).
//! Without cell names it reads cell names from stdin, one per line.
use anyhow::Result;
use aa_world::heightmap::CellHeights;
use std::io::BufRead;
fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let world = args.first().cloned().unwrap_or_else(|| "main_world".into());
    let cells: Vec<String> = if args.len() > 1 { args[1..].to_vec() } else { std::io::stdin().lock().lines().map_while(Result::ok).collect() };
    let pak = aa_pak::Pak::global();
    println!("cell\tmin_z\tmax_z\tocean\tland_pct");
    for cell in cells.iter().map(|c| c.trim()).filter(|c| !c.is_empty()) {
        let path = format!("game/worlds/{world}/cells/{cell}/client/terrain/heightmap.dat");
        match pak.read(&path).map_err(anyhow::Error::from).and_then(|b| CellHeights::parse(&b).map_err(|e| anyhow::anyhow!("{e:?}"))) {
            Ok(h) => {
                let land = h.heights.iter().filter(|&&z| z > h.ocean_level).count() as f32 / h.heights.len() as f32 * 100.0;
                println!("{cell}\t{:.0}\t{:.0}\t{:.0}\t{land:.0}", h.min_z, h.max_z, h.ocean_level);
            }
            Err(e) => println!("{cell}\terror {e}"),
        }
    }
    Ok(())
}
