use anyhow::{Result,ensure};
use aa_world::heightmap::{CellHeights,GRID};
use std::{path::PathBuf,fs};
#[path="../../../aa-editor/src/heightmap_write.rs"] mod heightmap_write;
fn main()->Result<()> {
    let root=PathBuf::from("C:/Games/aadev");
    let dir=root.join("reports/live-roundtrip");fs::create_dir_all(&dir)?;
    let source=root.join("overlay/worlds/main_world/cells/013_029/client/terrain/heightmap.dat");
    let before=fs::read(&source)?;
    let backup=dir.join("saved-heightmap.dat");
    ensure!(!backup.exists(),"roundtrip backup exists; do not replace saved edits");
    fs::write(&backup,&before)?;
    let mut h=CellHeights::parse(&before)?;let initial=h.sample(256.,256.);
    for y in 0..GRID {for x in 0..GRID {
        let d=((x as f32*2.-256.).powi(2)+(y as f32*2.-256.).powi(2)).sqrt();
        if d<12. {h.heights[y*GRID+x]+=0.5*(1.-d/12.);}
    }}
    let bytes=heightmap_write::write(&before,&h)?;let after=CellHeights::parse(&bytes)?;
    ensure!((after.sample(256.,256.)-initial-0.5).abs()<0.02,"unexpected terrain delta");
    ensure!(after.surface==h.surface,"surface bits changed");
    fs::write(&source,&bytes)?;
    let report=serde_json::json!({"before":initial,"temporary_published_height":after.sample(256.,256.),"saved_overlay_backup":backup});
    fs::write(dir.join("terrain-change.json"),serde_json::to_vec_pretty(&report)?)?;
    println!("{report}");Ok(())
}
