//! The static brush models a native 10.x world uses, with the cells holding an instance of each. Placing a model
//! copies one of those instances, so its native record, model and material entries come from real data.
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, OnceLock};

pub struct BrushLibrary {
    /// Normalized model name -> (model name as a cell lists it, cells using it).
    pub models: BTreeMap<String, (String, Vec<(i32, i32)>)>,
}

/// The library of `world`, read once from every cell's `statobjs.dat`.
pub fn get(world: &str) -> Arc<BrushLibrary> {
    static CACHE: OnceLock<Mutex<HashMap<String, Arc<BrushLibrary>>>> = OnceLock::new();
    let mut cache = CACHE.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    cache.entry(world.to_string()).or_insert_with(|| Arc::new(build(world))).clone()
}

fn build(world: &str) -> BrushLibrary {
    let pak = aa_pak::Pak::global();
    let prefix = format!("worlds/{world}/cells/");
    let mut models: BTreeMap<String, (String, Vec<(i32, i32)>)> = BTreeMap::new();
    let names: Vec<String> = pak.names().map(|n| n.trim_start_matches("game/").to_string()).filter(|n| n.starts_with(&prefix) && n.ends_with("/client/statobjs.dat")).collect();
    for name in names {
        let Some(cell) = name[prefix.len()..].split('/').next().and_then(cell_of) else { continue };
        let Ok(bytes) = pak.read(&format!("game/{name}")) else { continue };
        for model in aa_world::placement::parse_statobjs(&bytes) {
            let entry = models.entry(crate::native_tables::normalized(&model)).or_insert_with(|| (model.clone(), Vec::new()));
            if !entry.1.contains(&cell) {
                entry.1.push(cell);
            }
        }
    }
    BrushLibrary { models }
}

fn cell_of(dir: &str) -> Option<(i32, i32)> {
    let (x, y) = dir.split_once('_')?;
    Some((x.parse().ok()?, y.parse().ok()?))
}

#[cfg(test)]
mod tests {
    #[test]
    fn cell_directories_parse() {
        assert_eq!(super::cell_of("013_029"), Some((13, 29)));
        assert_eq!(super::cell_of("bad"), None);
    }
}

/// Words in a model's folders or file name that mark buildings and building parts.
const BUILDING_WORDS: &[&str] = &[
    "housing", "house", "building", "build", "castle", "wall", "gate", "tower", "bridge", "town", "village", "ruin", "fence", "door",
    "roof", "stair", "pillar", "column", "statue", "temple", "shrine", "dock", "pier", "arch", "tent", "hut", "palace", "fort",
    "city", "harbor", "harbour", "port", "inn", "shop", "barrack", "keep", "lighthouse", "windmill", "mill", "chapel", "church",
    "altar", "monument", "gazebo", "pavilion", "balcony", "floor", "window", "_wall", "struct",
];

pub struct Model {
    pub name: String,
    pub path: String,
    /// Cells holding an instance.
    pub cells: usize,
}

/// The library as folders: region (first folder under `objects/env/` or `objects/`) -> type folder -> models.
pub struct Tree {
    pub regions: BTreeMap<String, BTreeMap<String, Vec<Model>>>,
    pub models: usize,
}

fn readable(folder: &str) -> String {
    let t = folder.trim_start_matches(|c: char| c.is_ascii_digit()).trim_start_matches('_');
    match t {
        "nuia" => "Nuia".into(),
        "harihara" => "Haranya".into(),
        "origin_continent" => "Auroria".into(),
        "thirdforce" => "Third continent races".into(),
        "" => folder.to_string(),
        _ => t.replace('_', " "),
    }
}

/// The folder tree of `world`'s models, only buildings and building parts unless `all`.
pub fn tree(world: &str, all: bool) -> Arc<Tree> {
    static CACHE: OnceLock<Mutex<HashMap<(String, bool), Arc<Tree>>>> = OnceLock::new();
    let mut cache = CACHE.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    cache
        .entry((world.to_string(), all))
        .or_insert_with(|| {
            let library = get(world);
            let mut regions: BTreeMap<String, BTreeMap<String, Vec<Model>>> = BTreeMap::new();
            let mut count = 0;
            for (key, (path, cells)) in &library.models {
                if !all && !BUILDING_WORDS.iter().any(|w| key.contains(w)) {
                    continue;
                }
                let rel = key.strip_prefix("objects/").unwrap_or(key);
                let rel = rel.strip_prefix("env/").unwrap_or(rel);
                let parts: Vec<&str> = rel.split('/').collect();
                let (region, group) = match parts.as_slice() {
                    [r, _, _, ..] => (readable(r), parts[1..parts.len() - 1].iter().map(|p| readable(p)).collect::<Vec<_>>().join(" / ")),
                    [r, _] => (readable(r), String::from("(top)")),
                    _ => (String::from("other"), String::from("(top)")),
                };
                let name = parts.last().copied().unwrap_or(key).trim_end_matches(".cgf").to_string();
                regions.entry(region).or_default().entry(group).or_default().push(Model { name, path: path.clone(), cells: cells.len() });
                count += 1;
            }
            Arc::new(Tree { regions, models: count })
        })
        .clone()
}
