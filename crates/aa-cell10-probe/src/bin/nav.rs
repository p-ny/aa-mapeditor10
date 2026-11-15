use anyhow::{Result,ensure};
use std::{path::PathBuf,collections::HashMap,sync::Arc,fs};
use serde_json::json;
#[path="../../../aa-editor/src/navmesh.rs"] mod navmesh;
#[path="../../../aa-editor/src/navmesh_add.rs"] mod navmesh_add;
mod server_terrain {
    pub fn heightmap_rel(world:&str,c:(i32,i32),server:bool)->String {
        format!("{}worlds/{world}/cells/{:03}_{:03}/client/terrain/heightmap.dat",if server {"game/"}else{""},c.0,c.1)
    }
    pub fn backup(path:&std::path::Path)->anyhow::Result<bool> {
        anyhow::ensure!(path.starts_with("data/local"),"navigation target outside test root");
        let old=path.with_extension("bai.editor-orig");
        if path.exists() && !old.exists(){std::fs::copy(path,old)?;return Ok(true)}
        Ok(false)
    }
}
fn main()->Result<()> {
    let root=PathBuf::from("data/local");
    let args:Vec<_>=std::env::args().collect();
    ensure!(args.len()==3,"nav <effective-overlay> <new-staging-directory>; use the publish workflow for runtime updates");
    let overlay=PathBuf::from(&args[1]);
    let output=PathBuf::from(&args[2]);
    ensure!(overlay.canonicalize()?.starts_with(root.canonicalize()?),"overlay outside test root");
    fs::create_dir_all(&output)?;
    ensure!(fs::read_dir(&output)?.next().is_none(),"navigation staging directory must be empty");
    ensure!(output.canonicalize()?.starts_with(root.canonicalize()?),"navigation output outside test root");
    let mut worlds:std::collections::BTreeMap<String,HashMap<(i32,i32),Arc<aa_world::heightmap::CellHeights>>>=std::collections::BTreeMap::new();
    let mut cells=Vec::new();
    for (name,path) in aa_cell10_probe::cells::discover(&overlay)? {
        let (cell,file)=aa_cell10_probe::cells::parse_name(&name)?;
        if file!="terrain/heightmap.dat"{continue}
        let edited=aa_world::heightmap::CellHeights::parse(&fs::read(path)?)?;
        worlds.entry(cell.world.clone()).or_default().insert((cell.x,cell.y),Arc::new(edited));cells.push(cell);
    }
    let mut total=navmesh::Report::default();let mut reports=Vec::new();let mut zone_targets=Vec::new();
    for (world,changes) in worlds {
        let report=navmesh::export(&world,&changes,&output,None)?;
        for id in &report.zone_targets{zone_targets.push(json!({"world":world,"zone_id":id}));}
        reports.push(json!({"world":world,"terrain_cells":changes.len(),"files":report.files,"split":report.split,"children":report.children}));
        total.files+=report.files;total.moved+=report.moved;total.kept+=report.kept;
        total.blocked_links+=report.blocked_links;total.walls+=report.walls;total.opened+=report.opened;total.split+=report.split;total.children+=report.children;
    }
    let report=total;
    let result=json!({"schema":2,"zone_targets":zone_targets,"terrain_cells":cells,"worlds":reports,"files":report.files,"moved":report.moved,"kept":report.kept,"blocked_links":report.blocked_links,"walls":report.walls,"opened":report.opened,"split":report.split,"children":report.children,"original_pak_read_only":true,"server_overlay":output});
    fs::write(output.join("navmesh-export.json"),serde_json::to_vec_pretty(&result)?)?;
    println!("{}",serde_json::to_string_pretty(&result)?);
    Ok(())
}
