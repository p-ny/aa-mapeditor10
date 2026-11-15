//! The ground colour of each surface material, from the ground catalog (`island-finish catalog`): the mean cover
//! texel of real 64 m sectors that are only that material. The paint brush tints the cover with it, so painted
//! sand also looks like sand from afar.
use std::collections::HashMap;
use std::sync::OnceLock;

const DEFAULT: &str = "data/local/data/ground_catalog.json";

/// `DetailMaterial` as a catalog key: lower case, forward slashes, without a leading `game/`.
fn key(material: &str) -> String {
    let m = material.to_lowercase().replace('\\', "/");
    m.strip_prefix("game/").unwrap_or(&m).to_string()
}

fn table() -> &'static HashMap<String, [f32; 3]> {
    static TABLE: OnceLock<HashMap<String, [f32; 3]>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let path = crate::arg("--ground-catalog").unwrap_or_else(|| DEFAULT.to_string());
        let Ok(text) = std::fs::read_to_string(&path) else { return HashMap::new() };
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else { return HashMap::new() };
        json.as_object()
            .into_iter()
            .flatten()
            .filter_map(|(k, v)| {
                let m = v.get("mean")?.as_array()?;
                let c = [m.first()?.as_f64()?, m.get(1)?.as_f64()?, m.get(2)?.as_f64()?];
                Some((k.clone(), c.map(|x| (x / 255.0) as f32)))
            })
            .collect()
    })
}

/// The cover texel (encoded, 0..1) of real ground of `material`, when the catalog has it.
pub fn texel(material: &str) -> Option<[f32; 3]> {
    table().get(&key(material)).copied()
}
