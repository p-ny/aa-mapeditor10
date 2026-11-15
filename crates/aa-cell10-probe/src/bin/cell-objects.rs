//! `cell-objects <XXX_YYY>`: roads and water volumes of a main_world cell with their material names
//! (AA_PAK / AA_PAK_OVERLAY select the files).
fn main() {
    for c in std::env::args().skip(1) {
        let o = aa_world::placement::load_cell_objects("main_world", c[..3].parse().unwrap(), c[4..].parse().unwrap());
        let mat = |i: u32| o.materials.get(i as usize).cloned().unwrap_or_else(|| format!("<missing {i}>"));
        println!("{c}: {} roads, {} water volumes, {} materials", o.roads.len(), o.water.len(), o.materials.len());
        for r in &o.roads {
            println!("  road at {:.0},{:.0} {} points, material {}", (r.bbox[0] + r.bbox[3]) / 2.0, (r.bbox[1] + r.bbox[4]) / 2.0, r.points.len(), mat(r.material));
        }
        for w in &o.water {
            println!("  water at {:.0},{:.0} surface {:.1} {} points, material {}", (w.bbox[0] + w.bbox[3]) / 2.0, (w.bbox[1] + w.bbox[4]) / 2.0, w.surface, w.points.len(), mat(w.material));
        }
    }
}
