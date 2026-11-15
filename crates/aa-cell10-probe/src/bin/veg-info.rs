//! `veg-info <XXX_YYY>...`: where a main_world cell keeps its vegetation (AA_PAK selects the pak).
use aa_world::placement::{parse_vegetation_dat, parse_vegetation_sector};
fn main() {
    let pak = aa_pak::Pak::global();
    for c in std::env::args().skip(1) {
        let base = format!("game/worlds/main_world/cells/{c}/client");
        let dat = pak.read(&format!("{base}/vegetation.dat")).ok();
        let n_dat = dat.as_deref().map(|d| parse_vegetation_dat(d).map(|v| v.len()).unwrap_or(0));
        let mut sectors = 0;
        let mut n_sec = 0;
        for i in 0..16 {
            if let Ok(d) = pak.read(&format!("{base}/vegetation/{:02}_{:02}.dat", i / 4, i % 4)) {
                sectors += 1;
                n_sec += parse_vegetation_sector(&d, i / 4, i % 4).len();
            }
        }
        let objects = aa_world::placement::load_cell_objects("main_world", c[..3].parse().unwrap(), c[4..].parse().unwrap());
        let mut count = std::collections::BTreeMap::<u16, usize>::new();
        for v in &objects.vegetation {
            *count.entry(v.group).or_default() += 1;
        }
        let mut top: Vec<(u16, usize)> = count.into_iter().collect();
        top.sort_by(|a, b| b.1.cmp(&a.1));
        let groups = aa_world::vegetation::VegGroups::load("main_world");
        for (g, n) in top.iter().take(12) {
            let m = groups.as_ref().and_then(|gs| gs.groups.get(g)).map(|x| x.model.clone()).unwrap_or_default();
            println!("  group {g}: {n} x {m}");
        }
        println!("{c}: vegetation.dat {:?} bytes {} records {:?}; sector files {sectors} records {n_sec}; total loaded {}", dat.as_ref().map(|d| d.len()), "", n_dat, objects.vegetation.len());
    }
}
