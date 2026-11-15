use anyhow::{Result,ensure,Context};
use std::{fs,path::PathBuf};
use aa_world::heightmap::{CellHeights,GRID};
#[path="../../../aa-editor/src/heightmap_write.rs"]mod heightmap_write;
fn main()->Result<()> {
    let root=PathBuf::from("C:/Games/aadev");let out=root.join("reports/multi-cell-trial");fs::create_dir_all(&out)?;
    if std::env::args().any(|a|a=="--covered-zone-case") {
        unsafe{std::env::remove_var("AA_PAK_OVERLAY");}
        let pak=aa_pak::Pak::open("D:/dev/clients/AA-CN/game_pak")?;
        let verts=pak.read("worlds/instance_prologue/zone/300/vertsmission0.bai")?;
        let n=i32::from_le_bytes(verts[4..8].try_into()?) as usize;
        let f=|i:usize,k:usize|f32::from_le_bytes(verts[8+i*32+k*4..12+i*32+k*4].try_into().unwrap());
        let mut cache=std::collections::HashMap::new();let mut candidates=Vec::new();
        for i in 0..n {
            let (x,y,z)=(f(i,0),f(i,1),f(i,2));if !x.is_finite()||!y.is_finite()||x<0.||y<0.{continue}
            let (cx,cy)=((x/1024.).floor() as i32,(y/1024.).floor() as i32);
            if cx>4||cy>4{continue}
            let px=(x/4.).round()*4.;let py=(y/4.).round()*4.;let (lx,ly)=(px-cx as f32*1024.,py-cy as f32*1024.);
            if !(64.0..960.0).contains(&lx)||!(64.0..960.0).contains(&ly){continue}
            if !cache.contains_key(&(cx,cy)){
                let name=format!("worlds/instance_prologue/cells/{cx:03}_{cy:03}/client/terrain/heightmap.dat");
                let Ok(bytes)=pak.read(&name)else{continue};cache.insert((cx,cy),(bytes.clone(),CellHeights::parse(&bytes)?));
            }
            let h=&cache[&(cx,cy)].1;
            if (h.mesh_height(x-cx as f32*1024.,y-cy as f32*1024.)-z).abs()<0.15 {
                candidates.push((((px-1500.).powi(2)+(py-1500.).powi(2)),i,cx,cy,px,py,x,y,z));
            }
        }
        candidates.sort_by(|a,b|a.0.total_cmp(&b.0));let &(_,vertex,cx,cy,px,py,vx,vy,vz)=candidates.first().context("no ground-aligned zone BAI vertex")?;
        let (bytes,mut h)=cache.remove(&(cx,cy)).unwrap();let center=(px-cx as f32*1024.,py-cy as f32*1024.);
        for y in 0..GRID{for x in 0..GRID{let d=((x as f32*2.-center.0).powi(2)+(y as f32*2.-center.1).powi(2)).sqrt();if d<12.{h.heights[y*GRID+x]+=1.25*(1.-d/12.);}}}
        let edited=heightmap_write::write(&bytes,&h)?;
        let base=out.join("multi-world-overlay/worlds/instance_prologue/cells");
        for cell in ["000_000","000_001"]{let p=base.join(cell).join("client/terrain/heightmap.dat");if p.is_file(){fs::remove_file(p)?;}}
        let name=format!("worlds/instance_prologue/cells/{cx:03}_{cy:03}/client/terrain/heightmap.dat");let dest=out.join("multi-world-overlay").join(name);fs::create_dir_all(dest.parent().unwrap())?;fs::write(&dest,&edited)?;
        let data=serde_json::json!({"world":"instance_prologue","cell":format!("{cx:03}_{cy:03}"),"zone":300,"vertex":vertex,"vertex_world":[vx,vy,vz],"center_world":[px,py],"expected_vertex_delta":1.25*(1.-((vx-px).powi(2)+(vy-py).powi(2)).sqrt()/12.),"output":dest});
        fs::write(out.join("covered-zone-case.json"),serde_json::to_vec_pretty(&data)?)?;println!("{data}");return Ok(())
    }
    if std::env::args().any(|a|a=="--prologue-nav-case") {
        unsafe{std::env::remove_var("AA_PAK_OVERLAY");}
        let pak=aa_pak::Pak::open(root.join("client/game_pak"))?;
        let rel="worlds/instance_prologue/cells/000_001/client/terrain/heightmap.dat";
        let bytes=pak.read(rel)?;let mut h=CellHeights::parse(&bytes)?;let before=h.sample(256.,256.);
        for y in 0..GRID{for x in 0..GRID{let d=((x as f32*2.-256.).powi(2)+(y as f32*2.-256.).powi(2)).sqrt();if d<12.{h.heights[y*GRID+x]+=1.25*(1.-d/12.);}}}
        let edited=heightmap_write::write(&bytes,&h)?;let dest=out.join("multi-world-overlay").join(rel);fs::create_dir_all(dest.parent().unwrap())?;fs::write(&dest,&edited)?;
        let prior=out.join("multi-world-overlay/worlds/instance_prologue/cells/000_000/client/terrain/heightmap.dat");
        if prior.is_file(){fs::remove_file(prior)?;}
        let data=serde_json::json!({"world":"instance_prologue","cell":"000_001","before":before,"after":CellHeights::parse(&edited)?.sample(256.,256.),"output":dest});
        fs::write(out.join("prologue-navigation-case.json"),serde_json::to_vec_pretty(&data)?)?;println!("{data}");return Ok(())
    }
    ensure!(!out.join("inputs-created.json").exists(),"trial already staged; do not overwrite it");
    // Physical copied-pak reads; the trial overlay never replaces the user's saved overlay.
    unsafe{std::env::remove_var("AA_PAK_OVERLAY");}
    let pak=aa_pak::Pak::open(root.join("client/game_pak"))?;
    let mut cells=Vec::new();let mut checks=Vec::new();
    for (x,y) in [(13,29),(14,29)] {
        let name=format!("worlds/main_world/cells/{x:03}_{y:03}/client/terrain/heightmap.dat");let bytes=pak.read(&name)?;
        let mut h=CellHeights::parse(&bytes)?;
        let local_x=14336.-x as f32*1024.;let local_y=256.;
        let old=h.sample(local_x,local_y);
        for iy in 0..GRID{for ix in 0..GRID{
            let wx=x as f32*1024.+ix as f32*2.;let wy=y as f32*1024.+iy as f32*2.;
            let d=((wx-14336.).powi(2)+(wy-29952.).powi(2)).sqrt();if d<12.{h.heights[iy*GRID+ix]+=1.25*(1.-d/12.);}
        }}
        let edited=heightmap_write::write(&bytes,&h)?;let parsed=CellHeights::parse(&edited)?;
        ensure!((parsed.sample(local_x,local_y)-old-1.25).abs()<0.03,"border delta failed");
        let dest=out.join("overlay").join(&name);fs::create_dir_all(dest.parent().unwrap())?;fs::write(dest,&edited)?;
        checks.push(serde_json::json!({"cell":format!("{x:03}_{y:03}"),"edge_before":old,"edge_after":parsed.sample(local_x,local_y)}));
        cells.push(serde_json::json!({"world":"main_world","x":x,"y":y}));
    }
    // A second world verifies the cumulative plan can group separate world navigation.
    let name="worlds/instance_prologue/cells/000_000/client/terrain/heightmap.dat";
    if let Ok(bytes)=pak.read(name){
        let mut h=CellHeights::parse(&bytes)?;
        for iy in 120..=136 {for ix in 120..=136{let d=((ix as f32*2.-256.).powi(2)+(iy as f32*2.-256.).powi(2)).sqrt();if d<12.{h.heights[iy*GRID+ix]+=1.25*(1.-d/12.);}}}
        let edited=heightmap_write::write(&bytes,&h)?;let dest=out.join("multi-world-overlay").join(name);fs::create_dir_all(dest.parent().unwrap())?;fs::write(dest,edited)?;
    }
    // Pick a small matched rock in the new cell for the real editor's move/save check.
    let prefix="worlds/main_world/cells/014_029/client/";
    let brushes=aa_world::placement::parse_brush_dat(&pak.read(&format!("{prefix}brush.dat"))?)?;
    let models=aa_world::placement::parse_statobjs(&pak.read(&format!("{prefix}statobjs.dat"))?);
    let rock=brushes.iter().find(|b|models.get(b.model as usize).is_some_and(|m|m.to_ascii_lowercase().contains("rock"))&&b.translation().x>32.&&b.translation().x<850.&&b.translation().y>32.&&b.translation().y<990.&&(0..3).all(|i|b.bbox[i+3]-b.bbox[i]<20.)).context("no small rock for real editor move test")?;
    let pos=rock.translation();
    let report=serde_json::json!({"cells":cells,"border_world":[14336.,29952.],"checks":checks,"rock_world":[14336.+pos.x,29696.+pos.y,pos.z],"rock_model":models[rock.model as usize]});
    fs::write(out.join("inputs-created.json"),serde_json::to_vec_pretty(&report)?)?;println!("{report}");Ok(())
}
