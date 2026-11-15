//! Native 10.x NPC spawners: `worlds/<world>/level_design/zone/<id>/zone_server/npc_spawners.g`.
//!
//! The native zone host reads these from its loose game data, not from the pak. Positions are zone-local
//! (the zone's origin cell times 1024 m is added for world space) and `zRot` is radians. `spawnerType` is
//! `npc_spawners.id`. Only single-point spawners without paths are editable; every other block is kept verbatim.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result, bail, ensure};
use bevy::math::{Vec2, Vec3};

pub const CELL: f32 = 1024.0;

pub fn file_name(world: &str, zone: u32) -> String {
    format!("worlds/{world}/level_design/zone/{zone}/zone_server/npc_spawners.g")
}

#[derive(Clone, Debug)]
pub struct Block {
    pub text: String,
    pub id: u32,
    pub spawner_type: u32,
    /// Zone-local position and `zRot` of an editable single-point spawner.
    pub point: Option<(Vec3, f32)>,
}

#[derive(Clone, Debug, Default)]
pub struct File {
    /// Text before the first block.
    pub header: String,
    pub blocks: Vec<Block>,
    pub eol: &'static str,
}

fn value<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let rest = line.trim().strip_prefix(key)?;
    rest.starts_with(char::is_whitespace).then(|| rest.trim())
}

fn parse_pos(line: &str) -> Option<Vec3> {
    let inner = value(line, "pos")?.strip_prefix('(')?.strip_suffix(')')?;
    let mut v = [f32::NAN; 3];
    for part in inner.split(',') {
        let (k, n) = part.trim().split_once(char::is_whitespace)?;
        let i = ["x", "y", "z"].iter().position(|a| *a == k)?;
        v[i] = n.trim().parse().ok()?;
    }
    let p = Vec3::from_array(v);
    p.is_finite().then_some(p)
}

fn parse_block(text: &str) -> Result<Block> {
    let (mut id, mut spawner_type, mut area) = (None, None, None);
    let (mut points, mut paths, mut pos, mut rot) = (0, false, None, None);
    for line in text.lines() {
        if let Some(v) = value(line, "spawnerId") {
            id = v.parse().ok();
        } else if let Some(v) = value(line, "spawnerType") {
            spawner_type = v.parse().ok();
        } else if let Some(v) = value(line, "spawnAreaType") {
            area = Some(v.to_string());
        } else if line.trim() == "point" {
            points += 1;
        } else if line.trim() == "paths" {
            paths = true;
        } else if line.trim_start().starts_with("pos ") && points == 1 && pos.is_none() {
            pos = parse_pos(line);
        } else if let Some(v) = value(line, "zRot").filter(|_| points == 1 && rot.is_none()) {
            rot = v.parse::<f32>().ok().filter(|r| r.is_finite());
        }
    }
    let id = id.context("spawner without spawnerId")?;
    let spawner_type = spawner_type.with_context(|| format!("spawner {id} without spawnerType"))?;
    let editable = area.as_deref() == Some("point") && points == 1 && !paths;
    let point = if editable { Some((pos.with_context(|| format!("spawner {id}: invalid pos"))?, rot.unwrap_or(0.0))) } else { None };
    Ok(Block { text: text.to_string(), id, spawner_type, point })
}

pub fn parse(text: &str) -> Result<File> {
    let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut file = File { eol, ..Default::default() };
    let mut current: Option<String> = None;
    for line in text.split_inclusive('\n') {
        if line.trim_end() == "spawner" {
            if let Some(b) = current.take() {
                file.blocks.push(parse_block(&b)?);
            }
            current = Some(String::new());
        }
        match current.as_mut() {
            Some(b) => b.push_str(line),
            None => file.header.push_str(line),
        }
    }
    if let Some(b) = current {
        file.blocks.push(parse_block(&b)?);
    }
    Ok(file)
}

/// `%g`: six significant digits, no trailing zeros.
pub fn fmt_g(v: f32) -> String {
    if v.abs() < 1e-6 || !v.is_finite() {
        return "0".into();
    }
    let decimals = (5 - v.abs().log10().floor() as i32).max(0) as usize;
    let s = format!("{v:.decimals$}");
    let s = if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.') } else { &s };
    if s == "-0" { "0".into() } else { s.to_string() }
}

fn pos_text(p: Vec3) -> String {
    format!("pos ( x {}, y {}, z {} )", fmt_g(p.x), fmt_g(p.y), fmt_g(p.z))
}

/// `block` with its point, `zRot` and `spawnerType` replaced; indentation and all other lines unchanged.
pub fn rewrite(block: &str, spawner_type: u32, pos: Vec3, rot: f32) -> String {
    let mut out = String::with_capacity(block.len());
    for line in block.split_inclusive('\n') {
        let body = line.trim_end_matches(['\r', '\n']);
        let ending = &line[body.len()..];
        let indent = &body[..body.len() - body.trim_start().len()];
        let replaced = if value(body, "spawnerType").is_some() {
            Some(format!("spawnerType {spawner_type}"))
        } else if body.trim_start().starts_with("pos ") {
            Some(pos_text(pos))
        } else if value(body, "zRot").is_some() {
            Some(format!("zRot {}", fmt_g(rot)))
        } else {
            None
        };
        match replaced {
            Some(r) => {
                out.push_str(indent);
                out.push_str(&r);
                out.push_str(ending);
            }
            None => out.push_str(line),
        }
    }
    out
}

pub fn new_block(id: u32, spawner_type: u32, pos: Vec3, rot: f32, eol: &str) -> String {
    [
        "spawner".to_string(),
        format!("    spawnerId {id}"),
        "    spawnAreaType point".into(),
        format!("    spawnerType {spawner_type}"),
        "    points".into(),
        "        point".into(),
        format!("            {}", pos_text(pos)),
        format!("            zRot {}", fmt_g(rot)),
    ]
    .iter()
    .map(|l| format!("{l}{eol}"))
    .collect()
}

pub fn write(file: &File, blocks: impl IntoIterator<Item = String>) -> String {
    let mut out = file.header.clone();
    for b in blocks {
        if !out.is_empty() && !out.ends_with('\n') {
            out.push_str(file.eol);
        }
        out.push_str(&b);
    }
    out
}

#[derive(Clone, Debug)]
pub struct Zone {
    pub id: u32,
    /// The zone's world.xml name (its native map name).
    pub name: String,
    /// World-space offset of zone-local coordinates.
    pub origin: Vec2,
}

/// The world's zones and the 64 m sector owners, as the native World resolves them (later entries win).
#[derive(Default)]
pub struct WorldZones {
    pub zones: Vec<Zone>,
    sectors: HashMap<(i32, i32), usize>,
}

impl WorldZones {
    pub fn parse(xml: &str) -> Result<WorldZones> {
        let doc = roxmltree::Document::parse(xml)?;
        let mut out = WorldZones::default();
        let attr = |n: roxmltree::Node, k: &str| -> Result<i32> { Ok(n.attribute(k).with_context(|| format!("missing {k}"))?.parse()?) };
        for z in doc.descendants().filter(|n| n.has_tag_name("Zone") && n.parent().is_some_and(|p| p.has_tag_name("ZoneList"))) {
            let index = out.zones.len();
            out.zones.push(Zone {
                id: attr(z, "id")? as u32,
                name: z.attribute("name").unwrap_or_default().to_string(),
                origin: Vec2::new(attr(z, "originX")? as f32, attr(z, "originY")? as f32) * CELL,
            });
            for cell in z.descendants().filter(|n| n.has_tag_name("cell")) {
                let (cx, cy) = (attr(cell, "x")?, attr(cell, "y")?);
                for s in cell.descendants().filter(|n| n.has_tag_name("sector")) {
                    out.sectors.insert((cx * 16 + attr(s, "x")?, cy * 16 + attr(s, "y")?), index);
                }
            }
        }
        ensure!(!out.zones.is_empty(), "world.xml lists no zones");
        Ok(out)
    }

    pub fn owner(&self, world_pos: Vec3) -> Option<&Zone> {
        if !world_pos.is_finite() {
            return None;
        }
        let key = ((world_pos.x / 64.0).floor() as i32, (world_pos.y / 64.0).floor() as i32);
        self.sectors.get(&key).map(|&i| &self.zones[i])
    }

    /// The zone owning a 64 m world sector.
    pub fn sector_owner(&self, sector: (i32, i32)) -> Option<&Zone> {
        self.sectors.get(&sector).map(|&i| &self.zones[i])
    }

    pub fn get(&self, id: u32) -> Option<&Zone> {
        self.zones.iter().find(|z| z.id == id)
    }
}

/// Which spawner template places a given NPC, and which NPC a template shows.
#[derive(Default)]
pub struct Templates {
    /// NPC -> a single-NPC template that is active without a schedule.
    pub by_npc: HashMap<u32, u32>,
    /// Template -> its first NPC member.
    pub npc_of: HashMap<u32, u32>,
}

impl Templates {
    pub fn load(db: &Path) -> Result<Templates> {
        let conn = rusqlite::Connection::open_with_flags(db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let mut out = Templates::default();
        let mut q = conn.prepare("SELECT npc_spawner_id, member_id FROM npc_spawner_npcs WHERE member_type = 'Npc' ORDER BY id")?;
        for row in q.query_map([], |r| Ok((r.get::<_, u32>(0)?, r.get::<_, u32>(1)?)))? {
            let (spawner, npc) = row?;
            out.npc_of.entry(spawner).or_insert(npc);
        }
        let mut q = conn.prepare(
            "SELECT m.member_id, s.id FROM npc_spawners s JOIN npc_spawner_npcs m ON m.npc_spawner_id = s.id
             WHERE m.member_type = 'Npc' AND s.activation_state = 't' AND s.startTime = 0 AND s.endTime = 0
               AND (SELECT COUNT(*) FROM npc_spawner_npcs o WHERE o.npc_spawner_id = s.id) = 1
             ORDER BY m.member_id, s.maxPopulation <> 1, s.id",
        )?;
        for row in q.query_map([], |r| Ok((r.get::<_, u32>(0)?, r.get::<_, u32>(1)?)))? {
            let (npc, spawner) = row?;
            out.by_npc.entry(npc).or_insert(spawner);
        }
        if out.by_npc.is_empty() {
            bail!("{}: no NPC spawner templates", db.display());
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "spawner\r\n    spawnerId 215279\r\n    spawnAreaType point\r\n    spawnerType 23542\r\n    points\r\n        point\r\n            pos ( x 1442.24, y 1752.42, z 134.191 )\r\n            zRot -0.610865\r\nspawner\r\n    spawnerId 215325\r\n    spawnAreaType area\r\n    spawnerType 23646\r\n    roamingArea 215325\r\n    points\r\n        point\r\n            pos ( x 1424.25, y 2381.07, z 349.378 )\r\n            zRot 0\r\n        point\r\n            pos ( x 1489.91, y 2326.87, z 346.595 )\r\n            zRot 0\r\n";

    #[test]
    fn untouched_blocks_round_trip_and_only_points_are_editable() {
        let f = parse(SAMPLE).unwrap();
        assert_eq!(f.eol, "\r\n");
        assert_eq!(f.blocks.len(), 2);
        assert_eq!(f.blocks[0].point, Some((Vec3::new(1442.24, 1752.42, 134.191), -0.610865)));
        assert!(f.blocks[1].point.is_none());
        assert_eq!(write(&f, f.blocks.iter().map(|b| b.text.clone())), SAMPLE);
    }

    #[test]
    fn rewrite_changes_only_the_point_and_type_lines() {
        let f = parse(SAMPLE).unwrap();
        let out = rewrite(&f.blocks[0].text, 777, Vec3::new(1277.18, 1279.0, 392.0), 0.5);
        assert_eq!(out, f.blocks[0].text.replace("23542", "777").replace("x 1442.24, y 1752.42, z 134.191", "x 1277.18, y 1279, z 392").replace("-0.610865", "0.5"));
        let again = parse(&out).unwrap();
        assert_eq!(again.blocks[0].point, Some((Vec3::new(1277.18, 1279.0, 392.0), 0.5)));
        let added = parse(&new_block(900, 23542, Vec3::new(1.5, 2.0, 3.25), -1.0, "\r\n")).unwrap();
        assert_eq!((added.blocks[0].id, added.blocks[0].spawner_type, added.blocks[0].point), (900, 23542, Some((Vec3::new(1.5, 2.0, 3.25), -1.0))));
    }

    #[test]
    fn numbers_use_six_significant_digits() {
        assert_eq!(fmt_g(1442.2412), "1442.24");
        assert_eq!(fmt_g(-0.6108652), "-0.610865");
        assert_eq!(fmt_g(0.0), "0");
        assert_eq!(fmt_g(392.0), "392");
        assert_eq!(fmt_g(-0.0000001), "0");
    }

    #[test]
    fn later_zones_own_overlapping_sectors() {
        let xml = r#"<World><ZoneList>
            <Zone name="a" id="1" originX="12" originY="28"><cellList><cell x="14" y="29"><sectorList><sector x="0" y="1"/><sector x="1" y="1"/></sectorList></cell></cellList></Zone>
            <Zone name="b" id="2" originX="12" originY="28"><cellList><cell x="14" y="29"><sectorList><sector x="1" y="1"/></sectorList></cell></cellList></Zone>
        </ZoneList></World>"#;
        let w = WorldZones::parse(xml).unwrap();
        assert_eq!(w.owner(Vec3::new(14.0 * 1024.0 + 10.0, 29.0 * 1024.0 + 70.0, 0.0)).map(|z| z.id), Some(1));
        assert_eq!(w.owner(Vec3::new(14.0 * 1024.0 + 70.0, 29.0 * 1024.0 + 70.0, 0.0)).map(|z| z.id), Some(2));
        assert!(w.owner(Vec3::new(0.0, 0.0, 0.0)).is_none());
        assert_eq!(w.get(2).unwrap().origin, Vec2::new(12.0 * 1024.0, 28.0 * 1024.0));
    }

    #[test]
    fn real_zone_file_round_trips() {
        if aa_pak::Pak::global().is_empty() {
            return;
        }
        let Ok(bytes) = aa_pak::Pak::global().read(&file_name("main_world", 391)) else { return };
        let text = String::from_utf8(bytes).unwrap();
        let f = parse(&text).unwrap();
        assert!(f.blocks.iter().filter(|b| b.point.is_some()).count() > 100);
        assert_eq!(write(&f, f.blocks.iter().map(|b| b.text.clone())), text);
    }
}
