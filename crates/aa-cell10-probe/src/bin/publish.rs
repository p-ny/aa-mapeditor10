use anyhow::{Result,ensure,Context};
use std::{fs,path::{Path,PathBuf},collections::HashMap};
use serde_json::json;
fn payloads(tx:&Path)->Result<Vec<(String,Vec<u8>)>> {
    let inputs:serde_json::Value=serde_json::from_slice(&fs::read(tx.join("inputs.json"))?)?;
    inputs["patch_files"].as_array().context("missing patch file list")?.iter().map(|n| {
        let name=n.as_str().context("invalid patch name")?;
        aa_cell10_probe::cells::parse_name(name)?;
        let bytes=fs::read(tx.join("overlay").join(name))?;
        aa_cell10_probe::cells::validate(name,&bytes)?;
        ensure!(aa_cell10_probe::cells::digest(&bytes)==inputs["effective_assets"][name].as_str().unwrap_or(""),"snapshot changed after validation: {name}");
        Ok((format!("game/{name}"),bytes))
    }).collect()
}
fn apply(target:&Path,files:&[(String,Vec<u8>)],undo:&Path)->Result<serde_json::Value>{
    ensure!(!undo.exists(),"undo already exists");
    if files.is_empty(){return Ok(json!({"entries":0,"readback_verified":true,"no_pak_changes":true}));}
    let before=aa_pak::write::read_table(target)?;
    let result=aa_pak::write::patch_with_undo(target,files,Some(undo))?;
    let after=aa_pak::write::read_table(target)?;
    let index:HashMap<_,_>=after.files.iter().map(|f|(f.name.as_str(),f)).collect();
    let mut unchanged=0;
    for entry in &before.files {
        if files.iter().any(|(n,_)|n==&entry.name){continue}
        ensure!(index.get(entry.name.as_str()).is_some_and(|e|*e==entry),"unrelated entry changed: {}",entry.name);
        unchanged+=1;
    }
    let pak=aa_pak::Pak::open(target)?;
    for (name,bytes) in files {ensure!(pak.read(name)?==*bytes,"readback failed: {name}")}
    Ok(json!({"entries":files.len(),"unrelated_entries_unchanged":unchanged,"readback_verified":true,"old_size":result.old_len,"new_size":result.new_len}))
}
/// The paks of a publication target from `targets.json` (the transaction's `target.txt`, default `lab`).
struct Target{baseline:PathBuf,client:PathBuf,server:Option<PathBuf>}
fn target(root:&Path,tx:&Path)->Result<Target>{
    let name=fs::read_to_string(tx.join("target.txt")).map(|s|s.trim().to_string()).unwrap_or_else(|_|"lab".into());
    let all:serde_json::Value=serde_json::from_slice(&fs::read(root.join("targets.json"))?)?;
    let t=all.get(&name).with_context(||format!("unknown publication target {name}"))?;
    let path=|k:&str|t[k].as_str().map(PathBuf::from);
    let client=path("client_pak").context("target without client_pak")?;
    let server=path("server_pak").filter(|s|s.canonicalize().ok()!=client.canonicalize().ok());
    Ok(Target{baseline:path("baseline_pak").context("target without baseline_pak")?,client,server})
}
fn table_digest(pak:&Path)->Result<String>{Ok(aa_cell10_probe::cells::digest(&aa_pak::write::read_table(pak)?.tail()?))}
/// Every effective asset whose bytes differ in the server pak, so it matches the client after publishing.
fn server_payloads(tx:&Path,server:&Path)->Result<Vec<(String,Vec<u8>)>>{
    let inputs:serde_json::Value=serde_json::from_slice(&fs::read(tx.join("inputs.json"))?)?;
    let pak=aa_pak::Pak::open(server)?;let mut out=Vec::new();
    for (name,hash) in inputs["effective_assets"].as_object().context("missing effective assets")? {
        let bytes=fs::read(tx.join("effective-overlay").join(name))?;
        ensure!(aa_cell10_probe::cells::digest(&bytes)==hash.as_str().unwrap_or(""),"snapshot changed after validation: {name}");
        let pak_name=format!("game/{name}");
        ensure!(pak.entry(&pak_name).is_some(),"server pak lacks {name}");
        if pak.read(&pak_name)?!=bytes{out.push((pak_name,bytes));}
    }
    Ok(out)
}
fn main()->Result<()> {
    // Single-threaded startup: inspection/readback must read physical pak bytes.
    unsafe { std::env::remove_var("AA_PAK_OVERLAY"); }
    let args:Vec<_>=std::env::args().collect();
    ensure!(args.len()==3,"publish --prepare|--apply|--undo <transaction directory>");
    let root=PathBuf::from("C:/Games/aadev");
    let tx=PathBuf::from(&args[2]).canonicalize()?;
    let home=root.join("publishes").canonicalize()?;
    ensure!(tx.parent()==Some(home.as_path()),"transaction must be directly below C:/Games/aadev/publishes");
    let t=target(&root,&tx)?;
    let (undo,server_undo)=(tx.join("pak.undo"),tx.join("pak-server.undo"));
    match args[1].as_str(){
        "--prepare"=>{
            let mut report=aa_cell10_probe::cells::prepare(&t.baseline,&t.client,&tx)?;
            if let Some(server)=&t.server{
                report["server_target_table_sha256"]=table_digest(server)?.into();
                fs::write(tx.join("inputs.json"),serde_json::to_vec_pretty(&report)?)?;
                report["server_patch_files"]=server_payloads(&tx,server)?.len().into();
            }
            println!("{report}");
        }
        "--apply"=>{
            let inputs:serde_json::Value=serde_json::from_slice(&fs::read(tx.join("inputs.json"))?)?;
            ensure!(table_digest(&t.client)?==inputs["target_table_sha256"].as_str().unwrap_or(""),"client pak changed since preflight; preview/publish again from a fresh snapshot");
            let server_files=match &t.server{
                Some(server)=>{
                    ensure!(table_digest(server)?==inputs["server_target_table_sha256"].as_str().unwrap_or(""),"server pak changed since preflight; preview/publish again from a fresh snapshot");
                    Some((server,server_payloads(&tx,server)?))
                }
                None=>None,
            };
            let mut report=json!({"client":apply(&t.client,&payloads(&tx)?,&undo)?});
            if let Some((server,files))=server_files{
                match apply(server,&files,&server_undo){
                    Ok(r)=>report["server"]=r,
                    Err(e)=>{
                        if undo.exists(){aa_pak::write::undo(&t.client,&undo)?;fs::remove_file(&undo)?;}
                        return Err(e.context("server pak publish failed; client pak restored"));
                    }
                }
            }
            fs::write(tx.join("pak-report.json"),serde_json::to_vec_pretty(&report)?)?;
            println!("{report}");
        }
        "--undo"=>{
            if let Some(server)=&t.server && server_undo.exists(){aa_pak::write::undo(server,&server_undo)?;println!("Server pak restored to the previous publication");}
            if undo.exists(){aa_pak::write::undo(&t.client,&undo)?;}
            println!("Client pak restored to the previous publication");
        }
        _=>anyhow::bail!("expected --prepare, --apply or --undo")
    }
    Ok(())
}
#[cfg(test)] mod tests {
    use super::*;
    #[test] fn publish_readback_and_undo_preserve_unrelated_payloads()->Result<()> {
        let dir=std::env::temp_dir().join(format!("editor10-publish-{}",std::process::id()));
        fs::create_dir_all(&dir)?;
        let target=dir.join("game_pak");let undo=dir.join("pak.undo");
        let mut w=aa_pak::write::PakWriter::new();
        w.add("game/test.dat",vec![1;16]).add("game/untouched.dat",vec![7;120]);w.write(&target)?;
        let original=fs::read(&target)?;
        let report=apply(&target,&[("game/test.dat".into(),vec![2;1500])],&undo)?;
        ensure!(report["readback_verified"]==true);
        ensure!(report["unrelated_entries_unchanged"]==1);
        ensure!(aa_pak::Pak::open(&target)?.read("game/untouched.dat")?==vec![7;120]);
        ensure!(apply(&target,&[("game/test.dat".into(),vec![3])],&undo).is_err(),"must not overwrite recovery data");
        aa_pak::write::undo(&target,&undo)?;
        ensure!(fs::read(&target)?==original,"rollback must be byte-identical");
        fs::remove_file(undo)?;fs::remove_file(target)?;fs::remove_dir(dir)?;
        Ok(())
    }
    #[test] fn multiple_cells_preserve_previously_published_cells_and_undo_together()->Result<()> {
        use aa_world::placement_write::{SectorFile,Record};
        let root=std::env::temp_dir().join(format!("editor10-multi-{}",std::process::id()));fs::create_dir_all(&root)?;
        let source=root.join("baseline.pak");let target=root.join("current.pak");let tx=root.join("tx");fs::create_dir_all(&tx)?;
        let names=["worlds/main_world/cells/001_001/client/brush.dat","worlds/main_world/cells/002_001/client/brush.dat","worlds/instance_prologue/cells/000_000/client/brush.dat"];
        let empty=SectorFile::parse_brushes(&{let mut b=vec![0;2052];b[..4].copy_from_slice(&1u32.to_le_bytes());b}).unwrap();
        let mut changed=empty.clone();changed.records.push(Record{sector:0,bytes:vec![0;128],deleted:false});let edited=changed.write();
        let mut w=aa_pak::write::PakWriter::new();
        for n in names {
            w.add(&format!("game/{n}"),empty.write());
            w.add(&format!("game/{}terrain/heightmap.dat",n.strip_suffix("brush.dat").unwrap()),b"unchanged-height-placeholder".to_vec());
        }
        w.add("game/unrelated.dat",b"keep me".to_vec());w.write(&source)?;fs::copy(&source,&target)?;
        aa_pak::write::patch(&target,&[(format!("game/{}",names[0]),edited.clone())])?;
        let before=fs::read(&target)?;
        for n in &names[1..]{let p=tx.join("overlay").join(n);fs::create_dir_all(p.parent().unwrap())?;fs::write(p,&edited)?;}
        let inputs=aa_cell10_probe::cells::prepare(&source,&target,&tx)?;
        ensure!(inputs["saved_cells"].as_array().unwrap().len()==2);
        ensure!(inputs["patch_files"].as_array().unwrap().len()==2);
        ensure!(fs::read(tx.join("effective-overlay").join(names[0]))?==edited,"existing published cell lost");
        let files=payloads(&tx)?;apply(&target,&files,&tx.join("pak.undo"))?;
        let pak=aa_pak::Pak::open(&target)?;for name in names{ensure!(pak.read(&format!("game/{name}"))?==edited);}
        drop(pak);aa_pak::write::undo(&target,tx.join("pak.undo"))?;ensure!(fs::read(&target)?==before);
        ensure!(root.canonicalize()?.starts_with(std::env::temp_dir().canonicalize()?));fs::remove_dir_all(&root)?;
        Ok(())
    }

    #[test]fn inconsistent_native_object_pairs_and_missing_model_references_are_rejected()->Result<()> {
        use aa_world::placement_write::{SectorFile,Record,ObjectTree,Node,Item};
        let root=std::env::temp_dir().join(format!("editor10-invalid-pairs-{}",std::process::id()));fs::create_dir_all(&root)?;
        let mut rec=vec![0u8;128];rec[115..119].copy_from_slice(&(-1i32).to_le_bytes());
        for i in [0,5,10]{rec[67+i*4..71+i*4].copy_from_slice(&1f32.to_le_bytes())}
        let mut sector=SectorFile::parse_brushes(&vec![0;2052]).unwrap();sector.version=1;sector.records.push(Record{sector:0,bytes:rec.clone(),deleted:false});
        let mut model=vec![0;256];model[..8].copy_from_slice(b"rock.cgf");
        let mut head=1u32.to_le_bytes().to_vec();head.extend(0u32.to_le_bytes());head.extend(&model);head.extend(0u32.to_le_bytes());
        let mut header=2u32.to_le_bytes().to_vec();for v in [0f32,0.,0.,8192.,8192.,8192.]{header.extend(v.to_le_bytes());}
        let tree=ObjectTree{head,nodes:vec![Node{header,mask:0,items:vec![0],rest:vec![],children:vec![]}],items:vec![Item{kind:1,bytes:rec.clone(),deleted:false}],tail:vec![]};
        let prefix="worlds/main_world/cells/003_004/client/";let target=root.join("current.pak");let source=root.join("source.pak");
        let mut models=1u32.to_le_bytes().to_vec();models.extend(&model);
        let mut w=aa_pak::write::PakWriter::new();for (name,bytes) in [("brush.dat",sector.write()),("object.dat",tree.write()),("statobjs.dat",models),("materials.dat",0u32.to_le_bytes().to_vec()),("terrain/heightmap.dat",vec![0])]{w.add(&format!("game/{prefix}{name}"),bytes);}w.write(&source)?;fs::copy(&source,&target)?;
        let before=fs::read(&target)?;
        for case in ["bad-twin","bad-model","bad-material","bad-flat-material","bad-table"]{
            let bad_model=case=="bad-model";
            let tx=root.join(case);let dir=tx.join("overlay").join(prefix);fs::create_dir_all(&dir)?;
            let mut edit=sector.clone();edit.records[0].bytes[79..83].copy_from_slice(&70f32.to_le_bytes());
            if bad_model{edit.records[0].bytes[123..127].copy_from_slice(&9u32.to_le_bytes());}
            let mut saved_tree=tree.clone();
            if case=="bad-material"{edit.records[0].bytes[115..119].copy_from_slice(&9i32.to_le_bytes());saved_tree.items[0].bytes[115..119].copy_from_slice(&9i32.to_le_bytes());}
            if case=="bad-flat-material"{
                edit.records[0].bytes[115..119].copy_from_slice(&0i32.to_le_bytes());
                saved_tree.items[0].bytes[115..119].copy_from_slice(&0i32.to_le_bytes());
                let at=saved_tree.head.len()-4;saved_tree.head[at..].copy_from_slice(&1u32.to_le_bytes());
                let mut material=vec![0;260];material[4..12].copy_from_slice(b"rock.mtl");saved_tree.head.extend(material);
            }
            if case=="bad-table"{let mut bad=1u32.to_le_bytes().to_vec();bad.extend(&model);bad[4]=b'X';fs::write(dir.join("statobjs.dat"),bad)?;}
            fs::write(dir.join("brush.dat"),edit.write())?;fs::write(dir.join("object.dat"),saved_tree.write())?;
            let err=format!("{:#}",aa_cell10_probe::cells::prepare(&source,&target,&tx).unwrap_err());
            ensure!(match case{"bad-model"=>err.contains("missing model"),"bad-material"|"bad-flat-material"=>err.contains("missing material"),"bad-table"=>err.contains("must append"),_=>err.contains("transform copies disagree")},"{err}");
            ensure!(fs::read(&target)?==before,"failed validation must not touch the pak");
        }
        ensure!(root.canonicalize()?.starts_with(std::env::temp_dir().canonicalize()?));fs::remove_dir_all(root)?;Ok(())
    }

}
