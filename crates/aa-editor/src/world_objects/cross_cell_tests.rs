//! Real native 10.x brush transfer, undo, redo, serialization and table remapping.
use super::*;
use aa_world::placement::{load_cell_objects, parse_brush_dat, parse_object_dat};
use aa_world::placement_write::{ObjectTree, SectorFile};
fn fixture() -> (WorldObjects, ObjectPart) {
    assert!(
        std::env::var("AA_PAK").is_ok(),
        "run native transfer regressions against the original 10.x pak"
    );
    let mut objs = WorldObjects::default();
    for c in [(13, 29), (14, 29)] {
        let loaded = load_cell_objects("main_world", c.0, c.1);
        let mut cell = EditCell::load("main_world", c, &loaded);
        cell.tenx_saved_octree = cell.octree.clone();
        objs.cells.insert(c, cell);
    }
    let source = &objs.cells[&(13, 29)];
    let dest = &objs.cells[&(14, 29)];
    let tree = parse_object_dat(&dest.octree.as_ref().unwrap().write()).unwrap();
    let index = (0..source.display.brushes.len() as u32)
        .find(|&i| {
            let Ok(b) = source.export_brush(i) else {
                return false;
            };
            b.model.to_ascii_lowercase().contains("rock")
                && (0..3).all(|axis| b.brush.bbox[axis + 3] - b.brush.bbox[axis] < 20.)
                && dest.display.models.iter().any(|m| m == &b.model)
                && tree.models.contains(&b.model)
                && b.material
                    .as_ref()
                    .is_none_or(|m| tree.materials.contains(m))
        })
        .expect("no shared paired brush fixture");
    let p = ObjectPart {
        cell: (13, 29),
        kind: ObjectKind::Brush,
        index,
    };
    let b = source.item(ObjectKind::Brush, index).unwrap();
    let source_world = b.position() + cell_origin(13, 29);
    let terrain = aa_world::TerrainQuery::new("main_world");
    let source_ground = terrain
        .height_at(source_world.x, source_world.y)
        .unwrap_or(b.position().z);
    let target_z = terrain.height_at(14848., 30208.).unwrap_or(b.position().z) + b.position().z
        - source_ground;
    println!(
        "CROSS_CELL_FIXTURE {}",
        serde_json::json!({"source_world":(b.position()+cell_origin(13,29)).to_array(),"target_world":[14848.,30208.,target_z],"source_index":index,"model":source.model(ObjectKind::Brush,index,None)})
    );
    objs.selected = Some(p);
    (objs, p)
}
fn bytes(objs: &WorldObjects) -> Vec<(Vec<u8>, Vec<u8>)> {
    [(13, 29), (14, 29)]
        .iter()
        .map(|c| {
            let cell = &objs.cells[c];
            (
                cell.brush_dat.as_ref().unwrap().write(),
                cell.octree.as_ref().unwrap().write(),
            )
        })
        .collect()
}
#[test]
fn cross_cell_transfer_remaps_native_tables_and_undo_redo_restores_both_cells() {
    let (mut objs, p) = fixture();
    let before = bytes(&objs);
    for (i, c) in [(13, 29), (14, 29)].iter().enumerate() {
        let dir = aa_world::world::cell_dir("main_world", c.0, c.1);
        assert_eq!(
            before[i].0,
            aa_pak::Pak::global()
                .read(&format!("{dir}/client/brush.dat"))
                .unwrap()
        );
        assert_eq!(
            before[i].1,
            aa_pak::Pak::global()
                .read(&format!("{dir}/client/object.dat"))
                .unwrap()
        );
    }

    let old = objs.cells[&p.cell].export_brush(p.index).unwrap();
    let model = old.model.clone();
    let material = old.material.clone();
    let target = Vec3::new(14848., 30208., old.brush.translation().z);
    objs.transfer_selected("main_world", target).unwrap();
    let q = objs.selected.unwrap();
    assert_eq!(q.cell, (14, 29));
    assert!(!objs.cells[&p.cell].alive(p.kind, p.index));
    assert!(objs.cells[&q.cell].alive(q.kind, q.index));
    assert_eq!(objs.undo.len(), 1);
    assert_eq!(objs.undo[0].len(), 2);
    let dest = &objs.cells[&q.cell];
    let new = dest.export_brush(q.index).unwrap();
    assert_eq!(new.model, model);
    assert_eq!(new.material, material);
    assert!((new.brush.translation() + cell_origin(14, 29) - target).length() < 0.001);
    let b = parse_brush_dat(&dest.brush_dat.as_ref().unwrap().write()).unwrap();
    let o = parse_object_dat(&dest.octree.as_ref().unwrap().write()).unwrap();
    let local = target - cell_origin(14, 29);
    let twin = o
        .brushes
        .iter()
        .find(|b| (b.translation() - local).length() < 0.001 && o.models[b.model as usize] == model)
        .unwrap();
    assert_eq!(twin.material, new.brush.material);
    assert_eq!(o.models[twin.model as usize], model);
    assert_eq!(b.len(), parse_brush_dat(&before[1].0).unwrap().len() + 1);
    // No object tables, opaque records, or source remaining records may change.
    let before_tree = ObjectTree::parse(&before[1].1).unwrap();
    let after_tree = dest.octree.as_ref().unwrap();
    assert_eq!(before_tree.head, after_tree.head);
    assert_eq!(before_tree.tail, after_tree.tail);
    assert_eq!(
        &after_tree.items[..before_tree.items.len()],
        before_tree.items.as_slice()
    );
    for i in 0..128 {
        if !(0..24).contains(&i)
            && !(67..115).contains(&i)
            && !(115..119).contains(&i)
            && !(123..127).contains(&i)
        {
            assert_eq!(new.record[i], old.record[i]);
            assert_eq!(new.twin[i], old.twin[i]);
        }
    }
    assert!(objs.undo_redo(false));
    assert_eq!(bytes(&objs), before);
    assert!(objs.cells[&p.cell].alive(p.kind, p.index));
    assert!(!objs.cells[&q.cell].alive(q.kind, q.index));
    assert!(objs.undo_redo(true));
    assert!(!objs.cells[&p.cell].alive(p.kind, p.index));
    assert!(objs.cells[&q.cell].alive(q.kind, q.index));
    let root = std::env::temp_dir().join(format!("editor10-cross-cell-{}", std::process::id()));
    let saved = objs.save_all("main_world", &root).unwrap();
    assert_eq!(saved.len(), 4);
    assert_eq!(objs.unsaved(), 0);
    for c in [(13, 29), (14, 29)] {
        let cell = &objs.cells[&c];
        let dir = root
            .join(aa_world::world::cell_dir("main_world", c.0, c.1))
            .join("client");
        let tree = ObjectTree::parse(&std::fs::read(dir.join("object.dat")).unwrap()).unwrap();
        let terrain = aa_pak::Pak::global()
            .read(&format!(
                "{}/client/terrain/heightmap.dat",
                aa_world::world::cell_dir("main_world", c.0, c.1)
            ))
            .unwrap();
        crate::tenx_spatial::validate(
            &tree,
            cell.tenx_saved_octree.as_ref().unwrap(),
            crate::tenx_spatial::origin(&terrain).unwrap(),
        )
        .unwrap();
        assert!(
            SectorFile::parse_brushes(&std::fs::read(dir.join("brush.dat")).unwrap()).is_some()
        );
    }
    assert!(objs.undo_redo(false));
    objs.save_all("main_world", &root).unwrap();
    for (i, c) in [(13, 29), (14, 29)].iter().enumerate() {
        let dir = root
            .join(aa_world::world::cell_dir("main_world", c.0, c.1))
            .join("client");
        assert_eq!(std::fs::read(dir.join("brush.dat")).unwrap(), before[i].0);
        assert_eq!(std::fs::read(dir.join("object.dat")).unwrap(), before[i].1);
    }
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn cross_cell_transfer_refuses_invalid_tables_or_non_neighbor_without_touching_source() {
    let (mut objs, p) = fixture();
    let before = bytes(&objs);
    let old_count = objs.cells[&(14, 29)].display.brushes.len();
    let z = objs.item(p).unwrap().position().z;
    for x in [16896., f32::NAN, f32::MAX, -f32::MAX] {
        assert!(
            objs.transfer_selected("main_world", Vec3::new(x, 30208., z))
                .is_err()
        );
    }
    assert_eq!(bytes(&objs), before);
    objs.cells.get_mut(&(14, 29)).unwrap().statobjs = Some(vec![0; 3]);
    assert!(
        objs.transfer_selected("main_world", Vec3::new(14848., 30208., z))
            .unwrap_err()
            .contains("table")
    );
    assert_eq!(bytes(&objs), before);
    assert_eq!(objs.cells[&(14, 29)].display.brushes.len(), old_count);
    assert_eq!(objs.unsaved(), 0);
    assert!(objs.undo.is_empty());
    assert!(objs.cells[&p.cell].alive(p.kind, p.index));
}
#[test]
fn cross_cell_transfer_replaces_table_indices_and_preserves_opaque_native_fields() {
    let (mut objs, p) = fixture();
    let mut payload = objs.cells[&p.cell].export_brush(p.index).unwrap();
    let model = payload.model.clone();
    let material = payload.material.clone();
    let cell = objs.cells.get_mut(&(14, 29)).unwrap();
    let flat_opaque = [0x12, 0x34, 0xab, 0xcd];
    let tree_opaque = [0xff, 0xfe, 0x00, 0x80];
    payload.record[119..123].copy_from_slice(&flat_opaque);
    payload.twin[119..123].copy_from_slice(&tree_opaque);
    for b in [&mut payload.record, &mut payload.twin] {
        b[123..127].copy_from_slice(&i32::MAX.to_le_bytes());
        b[115..119].copy_from_slice(&i32::MAX.to_le_bytes());
    }
    let local = Vec3::new(512., 512., payload.brush.translation().z);
    let (index, _, after) = cell.import_brush("main_world", payload, local).unwrap();
    cell.apply(ObjectKind::Brush, index, &after);
    let transferred = cell.export_brush(index).unwrap();
    let record_model =
        u32::from_le_bytes(transferred.record[123..127].try_into().unwrap()) as usize;
    let twin_model = u32::from_le_bytes(transferred.twin[123..127].try_into().unwrap()) as usize;
    let tree = parse_object_dat(&cell.octree.as_ref().unwrap().write()).unwrap();
    assert_eq!(cell.display.models[record_model], model);
    assert_eq!(tree.models[twin_model], model);
    assert_eq!(&transferred.record[119..123], &flat_opaque);
    assert_eq!(&transferred.twin[119..123], &tree_opaque);
    assert_eq!(transferred.material, material);
    let mut bad = cell.export_brush(index).unwrap();
    bad.material = Some("materials/missing-test-material".into());
    let records = cell.brush_dat.as_ref().unwrap().records.len();
    let tree_before = cell.octree.as_ref().unwrap().clone();
    assert!(cell.import_brush("main_world", bad, local).is_err());
    assert_eq!(cell.brush_dat.as_ref().unwrap().records.len(), records);
    assert_eq!(cell.octree.as_ref().unwrap(), &tree_before);
}
#[test]
fn cross_cell_transfer_rejects_an_ambiguous_source_material_copy() {
    let (mut objs, p) = fixture();
    let source = objs.cells.get_mut(&p.cell).unwrap();
    let cell::Source::BrushDat(record) = source.source(p.kind, p.index) else {
        panic!("fixture needs a brush record")
    };
    let twin = source.twins[&record];
    let material = source.display.brushes[p.index as usize].material;
    source.octree.as_mut().unwrap().items[twin].bytes[115..119]
        .copy_from_slice(&(material + 1).to_le_bytes());
    let before = bytes(&objs);
    assert!(
        objs.transfer_selected("main_world", Vec3::new(14848., 30208., 200.))
            .unwrap_err()
            .contains("material reference differs")
    );
    assert_eq!(bytes(&objs), before);
    assert_eq!(objs.unsaved(), 0);
    assert!(objs.undo.is_empty());
    assert!(objs.cells[&p.cell].alive(p.kind, p.index));
}

fn expansion_fixture() -> (WorldObjects, ObjectPart) {
    let (mut objs, _) = fixture();
    let source = &objs.cells[&(13, 29)];
    let dest = &objs.cells[&(14, 29)];
    let object = parse_object_dat(&dest.octree.as_ref().unwrap().write()).unwrap();
    let external = aa_world::placement::parse_material_list(dest.material_list.as_ref().unwrap());
    let index = (0..source.display.brushes.len() as u32)
        .find(|&i| {
            let b = &source.display.brushes[i as usize];
            let Some(model) = source.display.models.get(b.model as usize) else {
                return false;
            };
            let Some(material) = usize::try_from(b.material)
                .ok()
                .and_then(|i| source.display.materials.get(i))
            else {
                return false;
            };
            !dest.display.models.contains(model)
                && !object.models.contains(model)
                && !object.materials.contains(material)
                && !external.contains(material)
                && b.flags == 0x20000008
                && model.contains("rock")
                && (0..3).all(|axis| b.bbox[axis + 3] - b.bbox[axis] < 40.)
                && source.export_brush(i).is_ok()
        })
        .expect("no real source brush with model AND material absent from destination");
    let p = ObjectPart {
        cell: (13, 29),
        kind: ObjectKind::Brush,
        index,
    };
    let payload = source.export_brush(index).unwrap();
    let world_pos = payload.brush.translation() + cell_origin(13, 29);
    let z = aa_world::TerrainQuery::new("main_world")
        .height_at(14848., 30208.)
        .unwrap()
        + 8.;
    println!(
        "EXPANSION_FIXTURE {}",
        serde_json::json!({"source_world":world_pos.to_array(),"target_world":[14848.,30208.,z],"source_index":index,"model":payload.model,"material":payload.material})
    );
    objs.selected = Some(p);
    (objs, p)
}
fn table_bytes(objs: &WorldObjects) -> Vec<(Vec<u8>, Option<Vec<u8>>, Vec<u8>, Option<Vec<u8>>)> {
    [(13, 29), (14, 29)]
        .iter()
        .map(|c| {
            let cell = &objs.cells[c];
            (
                cell.statobjs.clone().unwrap(),
                cell.material_list.clone(),
                cell.octree.as_ref().unwrap().head.clone(),
                cell.flat_materials.clone(),
            )
        })
        .collect()
}
#[test]
fn cross_cell_expansion_appends_model_material_and_undo_after_save_restores_every_file() {
    let (mut objs, p) = expansion_fixture();
    let before = bytes(&objs);
    let tables = table_bytes(&objs);
    let original = objs.cells[&p.cell].export_brush(p.index).unwrap();
    let target = Vec3::new(14848., 30208., original.brush.translation().z);
    objs.transfer_selected("main_world", target).unwrap();
    let q = objs.selected.unwrap();
    let dest = &objs.cells[&q.cell];
    let parsed = parse_object_dat(&dest.octree.as_ref().unwrap().write()).unwrap();
    let placed = dest.export_brush(q.index).unwrap();
    assert_eq!(placed.model, original.model);
    assert_eq!(placed.material, original.material);
    assert_eq!(
        parsed.models.len(),
        parse_object_dat(&before[1].1).unwrap().models.len() + 1
    );
    assert_eq!(
        dest.display.models.len(),
        aa_world::placement::parse_statobjs(&tables[1].0).len() + 1
    );
    assert_eq!(
        parsed.materials.len(),
        parse_object_dat(&before[1].1).unwrap().materials.len() + 1
    );
    crate::native_tables::validate_append(
        &tables[1].0,
        dest.statobjs.as_ref().unwrap(),
        false,
        256,
    )
    .unwrap();
    crate::native_tables::validate_append(
        &tables[1].2,
        &dest.octree.as_ref().unwrap().head,
        true,
        260,
    )
    .unwrap();
    crate::native_tables::validate_append(tables[1].3.as_ref().unwrap(),dest.flat_materials.as_ref().unwrap(),false,256).unwrap();
    assert_eq!(aa_world::placement::parse_statobjs(dest.flat_materials.as_ref().unwrap()).len(),aa_world::placement::parse_statobjs(tables[1].3.as_ref().unwrap()).len()+1);
    assert_eq!(placed.entries.flat_material, original.entries.flat_material);
    assert_eq!(placed.entries.flat, original.entries.flat);
    assert_eq!(placed.entries.model, original.entries.model);
    assert_eq!(placed.entries.material, original.entries.material);
    assert_eq!(placed.entries.external, original.entries.external);
    let root =
        std::env::temp_dir().join(format!("editor10-table-expansion-{}", std::process::id()));
    assert_eq!(objs.save_all("main_world", &root).unwrap().len(), 7);
    assert_eq!(objs.unsaved(), 0);
    assert!(objs.undo_redo(false));
    assert_eq!(bytes(&objs), before);
    assert_eq!(table_bytes(&objs), tables);
    objs.save_all("main_world", &root).unwrap();
    for (i, c) in [(13, 29), (14, 29)].iter().enumerate() {
        let dir = root
            .join(aa_world::world::cell_dir("main_world", c.0, c.1))
            .join("client");
        assert_eq!(std::fs::read(dir.join("brush.dat")).unwrap(), before[i].0);
        assert_eq!(std::fs::read(dir.join("object.dat")).unwrap(), before[i].1);
    }
    let destdir = root.join("worlds/main_world/cells/014_029/client");
    assert_eq!(
        std::fs::read(destdir.join("statobjs.dat")).unwrap(),
        tables[1].0
    );
    assert_eq!(
        std::fs::read(destdir.join("material_list.dat")).unwrap(),
        tables[1].1.clone().unwrap()
    );
    assert_eq!(std::fs::read(destdir.join("materials.dat")).unwrap(), tables[1].3.clone().unwrap());
    assert!(objs.undo_redo(true));
    objs.save_all("main_world", &root).unwrap();
    assert_eq!(
        objs.cells[&q.cell].export_brush(q.index).unwrap().material,
        original.material
    );
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn cross_cell_expansion_undo_then_different_model_keeps_preview_ids_unique(){
    let (mut objs,p)=expansion_fixture();
    let first=objs.cells[&p.cell].export_brush(p.index).unwrap();
    let native_before=aa_world::placement::parse_statobjs(objs.cells[&(14,29)].statobjs.as_ref().unwrap());
    objs.transfer_selected("main_world",Vec3::new(14848.,30208.,220.)).unwrap();let q=objs.selected.unwrap();
    let first_preview=objs.cells[&q.cell].item(q.kind,q.index).unwrap();
    let Item::Brush(first_brush)=first_preview else{unreachable!()};
    assert!(objs.undo_redo(false));
    assert_eq!(aa_world::placement::parse_statobjs(objs.cells[&(14,29)].statobjs.as_ref().unwrap()),native_before);
    let source=&objs.cells[&p.cell];
    let next=(0..source.display.brushes.len() as u32).find(|&i|{
        let b=&source.display.brushes[i as usize];let name=&source.display.models[b.model as usize];
        *name!=first.model&&!native_before.contains(name)&&(0..3).all(|axis|b.bbox[axis+3]-b.bbox[axis]<20.)&&source.export_brush(i).is_ok()
    }).expect("second distinct model fixture");
    objs.selected=Some(ObjectPart{index:next,..p});
    objs.transfer_selected("main_world",Vec3::new(14860.,30216.,220.)).unwrap();let q=objs.selected.unwrap();
    let cell=&objs.cells[&q.cell];let Item::Brush(second)=cell.item(q.kind,q.index).unwrap()else{unreachable!()};
    assert_ne!(second.model,first_brush.model,"renderer cache IDs must not be reused after undo");
    let exported=cell.export_brush(q.index).unwrap();let native_model=u32::from_le_bytes(exported.record[123..127].try_into().unwrap()) as usize;
    assert_eq!(native_model,native_before.len(),"native append slot is reused after undo");
    assert_eq!(aa_world::placement::parse_statobjs(cell.statobjs.as_ref().unwrap())[native_model],exported.model);
    assert_eq!(cell.display.models[second.model as usize],exported.model);
}
