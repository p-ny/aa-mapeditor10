//! `model-find <word>...`: static models (from every main_world cell's statobjs.dat) whose path contains a word, with
//! how many cells use each (AA_PAK selects the pak).
use std::collections::BTreeMap;
fn main() {
    let words: Vec<String> = std::env::args().skip(1).map(|w| w.to_lowercase()).collect();
    let pak = aa_pak::Pak::global();
    let names: Vec<String> = pak.names().filter(|n| n.contains("worlds/main_world/cells/") && n.ends_with("/client/statobjs.dat")).map(String::from).collect();
    let mut found: BTreeMap<String, usize> = BTreeMap::new();
    for n in names {
        let Ok(b) = pak.read(&n) else { continue };
        for m in aa_world::placement::parse_statobjs(&b) {
            let l = m.to_lowercase();
            if words.iter().any(|w| l.contains(w)) {
                *found.entry(l).or_default() += 1;
            }
        }
    }
    for (m, c) in found { println!("{c}\t{m}"); }
}
