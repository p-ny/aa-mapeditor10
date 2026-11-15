//! Shared discovery and validation for existing native 10.x cell assets.
use anyhow::{Result,ensure,Context};
use std::{path::{Path,PathBuf},fs,collections::{BTreeMap,BTreeSet,HashMap}};
use serde::{Serialize,Deserialize};
use sha2::{Digest,Sha256};
use aa_world::{heightmap::CellHeights,placement_write::{SectorFile,ObjectTree}};
#[derive(Debug,Clone,PartialEq,Eq,PartialOrd,Ord,Hash,Serialize,Deserialize)]
pub struct Cell {pub world:String,pub x:i32,pub y:i32}
impl Cell {pub fn prefix(&self)->String{format!("worlds/{}/cells/{:03}_{:03}/client/",self.world,self.x,self.y)}}
pub fn parse_name(name:&str)->Result<(Cell,String)> {
    ensure!(!name.contains('\\')&&!name.starts_with('/')&&!name.contains(':')&&name==name.to_ascii_lowercase(),"noncanonical cell asset path: {name}");
    let p:Vec<_>=name.split('/').collect();
    ensure!(p.len()>=6&&p[0]=="worlds"&&p[2]=="cells"&&p[4]=="client","not a native cell asset: {name}");
    ensure!(!p[1].is_empty()&&p[1].bytes().all(|c|c.is_ascii_lowercase()||c.is_ascii_digit()||c==b'_'),"invalid world name");
    let (x,y)=p[3].split_once('_').context("invalid cell coordinates")?;
    let (x,y):(i32,i32)=(x.parse()?,y.parse()?);
    ensure!((0..=9999).contains(&x)&&(0..=9999).contains(&y)&&p[3]==format!("{x:03}_{y:03}"),"invalid or noncanonical cell coordinates");
    let file=p[5..].join("/");
    let supported=matches!(file.as_str(),"brush.dat"|"object.dat"|"statobjs.dat"|"materials.dat"|"material_list.dat"|"vegetation.dat"|"entities.xml"|"leveldata.xml"|"terrain/heightmap.dat"|"terrain/cover.ctc")||veg_sector(&file).is_some();
    ensure!(supported,"unsupported cell asset: {name}; only terrain, cover, static objects, vegetation, model/material tables and cell XML are publishable");
    Ok((Cell{world:p[1].into(),x,y},file))
}
fn veg_sector(name:&str)->Option<(usize,usize)> {
    let n=name.strip_prefix("vegetation/")?.strip_suffix(".dat")?;let (x,y)=n.split_once('_')?;
    let (x,y):(usize,usize)=(x.parse().ok()?,y.parse().ok()?);
    (x<4&&y<4&&n==format!("{x:02}_{y:02}")).then_some((x,y))
}
pub fn discover(dir:&Path)->Result<Vec<(String,PathBuf)>> {
    let root=dir.canonicalize()?;let mut pending=vec![root.clone()];let mut files=Vec::new();
    while let Some(folder)=pending.pop(){for e in fs::read_dir(folder)?{let e=e?;let p=e.path();
        ensure!(p.canonicalize()?.starts_with(&root),"overlay link escapes its root");
        ensure!(!e.file_type()?.is_symlink(),"overlay symlinks are not supported");
        if e.file_type()?.is_dir(){pending.push(p)}else if e.file_type()?.is_file(){let name=p.strip_prefix(&root)?.to_string_lossy().replace('\\',"/");parse_name(&name)?;files.push((name,p));}
    }} files.sort();Ok(files)
}
pub fn validate(name:&str,bytes:&[u8])->Result<()> {
    let (_,file)=parse_name(name)?;
    match file.as_str(){
        "terrain/heightmap.dat"=>{let h=CellHeights::parse(bytes)?;ensure!(h.heights.iter().all(|x|x.is_finite()),"nonfinite terrain height");}
        "brush.dat"=>{let f=SectorFile::parse_brushes(bytes).context("invalid brush.dat")?;ensure!(f.write()==bytes,"brush layout cannot roundtrip without loss");aa_world::placement::parse_brush_dat(bytes)?;}
        "object.dat"=>{let f=ObjectTree::parse(bytes).context("invalid object.dat")?;ensure!(f.write()==bytes,"object layout cannot roundtrip without loss");aa_world::placement::parse_object_dat(bytes)?;}
        "vegetation.dat"=>{let f=SectorFile::parse_vegetation(bytes).context("invalid vegetation.dat")?;ensure!(f.write()==bytes,"vegetation layout cannot roundtrip without loss");}
        "terrain/cover.ctc"=>{aa_world::cover::Cover::parse(bytes).map_err(|e|anyhow::anyhow!("invalid cover: {e:?}"))?;}
        "entities.xml"|"leveldata.xml"=>{roxmltree::Document::parse(std::str::from_utf8(bytes)?.trim_start_matches('\u{feff}'))?;}
        "statobjs.dat"|"materials.dat"|"material_list.dat"=>{
            ensure!(bytes.len()>=4,"truncated model/material table");let n=u32::from_le_bytes(bytes[..4].try_into()?) as usize;
            let width=if file=="material_list.dat"{260}else{256};ensure!(4usize.checked_add(n.checked_mul(width).context("table overflow")?).is_some_and(|size|size<=bytes.len()),"truncated model/material table entries");
        }
        _=>{let (x,y)=veg_sector(&file).context("unsupported file")?;let f=SectorFile::parse_vegetation_sector(bytes,x,y).context("invalid vegetation sector")?;ensure!(f.write()==bytes,"vegetation sector cannot roundtrip");}
    }Ok(())
}
fn brush_counts(bytes:&[u8],models:&[String],materials:&[String],object:bool)->Result<BTreeMap<(String,Option<String>,[i64;12]),i64>> {
    let brushes=if object{aa_world::placement::parse_object_dat(bytes)?.brushes}else{aa_world::placement::parse_brush_dat(bytes)?};
    let mut out=BTreeMap::new();
    for b in brushes {
        let name=crate::native_tables::normalized(models.get(b.model as usize).context("brush references missing model table entry")?);
        let material=if b.material==-1{None}else{
            ensure!(b.material>=0,"brush references invalid material index");
            Some(crate::native_tables::normalized(materials.get(b.material as usize).context("brush references missing material table entry")?))
        };
        ensure!(b.matrix.iter().all(|v|v.is_finite()),"nonfinite object transform");
        let key=std::array::from_fn(|i|(b.matrix[i] as f64*if i%4==3{100.0}else{1000.0}).round() as i64);
        *out.entry((name,material,key)).or_default()+=1;
    }Ok(out)
}
fn validate_brush_copies(current:&aa_pak::Pak,effective:&BTreeMap<String,Vec<u8>>,cell:&Cell)->Result<()> {
    let p=cell.prefix();let brush=format!("{p}brush.dat");let object=format!("{p}object.dat");let models=format!("{p}statobjs.dat");
    let read=|name:&str|->Result<Vec<u8>>{Ok(if let Some(b)=effective.get(name){b.clone()}else{current.read(&format!("game/{name}"))?})};
    for (name,width) in [(&models,256),(&format!("{p}materials.dat"),256),( &format!("{p}material_list.dat"),260)] {
        if let Some(new)=effective.get(name){crate::native_tables::validate_append(&current.read(&format!("game/{name}"))?,new,false,width)?;}
    }
    if current.entry(&format!("game/{object}")).is_none(){return Ok(())}
    let original_object=current.read(&format!("game/{object}"))?;let new_object=read(&object)?;
    let old_layout=ObjectTree::parse(&original_object).context("invalid original object.dat")?;
    let new_layout=ObjectTree::parse(&new_object).context("invalid saved object.dat")?;
    crate::native_tables::validate_append(&old_layout.head,&new_layout.head,true,260)?;
    let old_tree=aa_world::placement::parse_object_dat(&original_object)?;let new_tree=aa_world::placement::parse_object_dat(&new_object)?;
    let old_copies=brush_counts(&original_object,&old_tree.models,&old_tree.materials,true)?;
    let new_copies=brush_counts(&new_object,&new_tree.models,&new_tree.materials,true)?;
    if current.entry(&format!("game/{brush}")).is_none(){return Ok(())}
    let original_brush=current.read(&format!("game/{brush}"))?;let new_brush=read(&brush)?;
    let old_models=aa_world::placement::parse_statobjs(&current.read(&format!("game/{models}"))?);
    let new_models=aa_world::placement::parse_statobjs(&read(&models)?);
    let flat_mats=format!("{p}materials.dat");
    // Native brush.dat uses materials.dat, independently of object.dat's table.
    // Legacy fixtures without that file retain the older octree-table fallback.
    let (old_materials,new_materials)=if current.entry(&format!("game/{flat_mats}")).is_some(){
        (aa_world::placement::parse_statobjs(&current.read(&format!("game/{flat_mats}"))?),aa_world::placement::parse_statobjs(&read(&flat_mats)?))
    }else{(old_tree.materials.clone(),new_tree.materials.clone())};
    let old=brush_counts(&original_brush,&old_models,&old_materials,false)?;
    let new=brush_counts(&new_brush,&new_models,&new_materials,false)?;
    let keys:BTreeSet<_>=old.keys().chain(new.keys()).collect();
    for key in keys {
        let delta=new.get(key).copied().unwrap_or(0)-old.get(key).copied().unwrap_or(0);
        if delta==0{continue}
        if delta>0||old_copies.contains_key(key){
            let twin_delta=new_copies.get(key).copied().unwrap_or(0)-old_copies.get(key).copied().unwrap_or(0);
            ensure!(delta==twin_delta,"{}: brush.dat/object.dat transform copies disagree (model/material) for {}",cell.prefix(),key.0);
        }
    }Ok(())
}
pub fn digest(b:&[u8])->String{format!("{:x}",Sha256::digest(b))}
/// Derive the cumulative terrain state from the actual current pak, not a stale
/// cache, so publishing cell B cannot erase a previously published cell A.
pub fn prepare(source:&Path,target:&Path,tx:&Path)->Result<serde_json::Value>{
    let base_table=aa_pak::write::read_table(source)?;let current_table=aa_pak::write::read_table(target)?;
    let base:HashMap<_,_>=base_table.files.iter().map(|f|(f.name.as_str(),f)).collect();
    let original=aa_pak::Pak::open(source)?;let current=aa_pak::Pak::open(target)?;
    let saved=discover(&tx.join("overlay"))?;ensure!(!saved.is_empty(),"no saved cell assets in overlay");
    let mut effective:BTreeMap<String,Vec<u8>>=BTreeMap::new();let mut previously_published=0;
    for entry in &current_table.files {
        let rel=entry.name.strip_prefix("game/").unwrap_or(&entry.name);
        if parse_name(rel).is_err(){continue}
        let differs=base.get(entry.name.as_str()).is_none_or(|old|*old!=entry);
        if !differs{continue}
        let bytes=current.read(&entry.name)?;
        if original.entry(&entry.name).is_some_and(|_|original.read(&entry.name).is_ok_and(|old|old==bytes)){continue}
        validate(rel,&bytes).with_context(||format!("existing published asset {rel}"))?;
        effective.insert(rel.into(),bytes);previously_published+=1;
    }
    let mut changed=BTreeSet::new();let mut saved_cells=BTreeSet::new();
    for (name,path) in saved {
        let (cell,_)=parse_name(&name)?;saved_cells.insert(cell.clone());
        let pak_name=format!("game/{name}");
        ensure!(original.entry(&format!("game/{}terrain/heightmap.dat",cell.prefix())).is_some(),"cell does not exist in baseline: {}",cell.prefix());
        ensure!(current.entry(&pak_name).is_some(),"new native cell asset creation is not supported: {name}");
        let bytes=fs::read(path)?;validate(&name,&bytes).with_context(||format!("saved asset {name}"))?;
        if current.read(&pak_name)?!=bytes{changed.insert(name.clone());}
        effective.insert(name,bytes);
    }
    // brush.dat and object.dat are two native copies, not independent exports.
    for cell in &saved_cells {
        validate_brush_copies(&current,&effective,cell)?;
        let prefix=cell.prefix();let brush=format!("{prefix}brush.dat");let object=format!("{prefix}object.dat");
        let saved_names:BTreeSet<_>=discover(&tx.join("overlay"))?.into_iter().map(|(n,_)|n).collect();
        if changed.contains(&brush)&&current.entry(&format!("game/{object}")).is_some(){
            ensure!(saved_names.contains(&object),"{}: brush edit must include the matching object.dat save",cell.prefix());
        }
        if changed.contains(&object) {
            let before=ObjectTree::parse(&current.read(&format!("game/{object}"))?).context("invalid existing object tree")?;
            let tree=ObjectTree::parse(effective.get(&object).context("missing edited object tree")?).context("invalid saved object tree")?;
            let height=format!("{prefix}terrain/heightmap.dat");
            let bytes=if let Some(b)=effective.get(&height){b.clone()}else{current.read(&format!("game/{height}"))?};
            crate::spatial::validate(&tree,&before,crate::spatial::origin(&bytes)?)?;
        }
    }
    let effective_dir=tx.join("effective-overlay");fs::create_dir_all(&effective_dir)?;
    let mut terrain_cells=BTreeSet::new();let mut hashes=BTreeMap::new();
    for (name,bytes) in &effective {
        let (cell,file)=parse_name(name)?;
        if file=="terrain/heightmap.dat"{terrain_cells.insert(cell);}
        let dest=effective_dir.join(name);fs::create_dir_all(dest.parent().unwrap())?;fs::write(dest,bytes)?;hashes.insert(name.clone(),digest(bytes));
    }
    let report=serde_json::json!({"schema":2,"saved_cells":saved_cells,"terrain_cells":terrain_cells,"effective_assets":hashes,"patch_files":changed,"previously_published_assets_preserved":previously_published,"baseline_table_sha256":digest(&base_table.tail()?),"target_table_sha256":digest(&current_table.tail()?)});
    fs::write(tx.join("inputs.json"),serde_json::to_vec_pretty(&report)?)?;Ok(report)
}
#[cfg(test)] mod tests {
    use super::*;
    #[test] fn paths_support_other_cells_and_worlds_but_reject_escape_and_unknown_files(){
        for p in ["worlds/main_world/cells/014_030/client/terrain/heightmap.dat","worlds/instance_prologue/cells/001_000/client/brush.dat","worlds/main_world/cells/014_030/client/vegetation/03_02.dat"]{assert!(parse_name(p).is_ok(),"{p}");}
        for p in ["worlds/../cells/014_030/client/brush.dat","worlds/main_world/cells/14_030/client/brush.dat","worlds/main_world/cells/014_030/client/../../../config.json","worlds/main_world/cells/014_030/client/vegetation/04_00.dat","worlds/main_world/cells/014_030/client/bogus.dat"]{assert!(parse_name(p).is_err(),"{p}");}
    }
}
