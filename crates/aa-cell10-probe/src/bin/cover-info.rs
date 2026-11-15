//! `cover-info <cell file>...`: tile count per level of `cover.ctc` files.
use aa_world::cover::Cover;
fn main() {
    for p in std::env::args().skip(1) {
        let b = std::fs::read(&p).unwrap();
        let c = Cover::parse(&b).unwrap();
        let levels: Vec<String> = (0..=c.max_level).map(|l| {
            let n = (0..1usize << l).flat_map(|x| (0..1usize << l).map(move |y| (x, y))).filter(|&(x, y)| c.tile(0, l, x, y).is_some()).count();
            format!("L{l}:{n}/{}", 1usize << (2 * l))
        }).collect();
        println!("{p}: {} layers, {}", c.layers.len(), levels.join(" "));
    }
}
