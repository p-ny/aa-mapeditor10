//! Level-pack map doodads: `worlds/<world>/level_design/cells/<cx>_<cy>/doodad.g`.
//!
//! Since AAEmu-10.x e22923b5 the World plants open-world doodads from these files (`MapDoodadSource LevelPack`);
//! `doodad_spawns.json` is only a list for templates the cells never carry. Positions are cell-local (cell times
//! 1024 m is added for world space), `ori` is a quaternion (x, y, z, w) in Cry space, `type` is
//! `doodad_almighties.id`. Untouched blocks are written back verbatim; an edited block only gets its `pos`, `ori`
//! and `scale` lines replaced.
use anyhow::{Context, Result, bail};
use bevy::math::{EulerRot, Quat, Vec2, Vec3};

use crate::zone_spawners::fmt_g;

pub const CELL: f32 = 1024.0;

pub fn file_name(world: &str, cell: (i32, i32)) -> String {
    format!("worlds/{world}/level_design/cells/{:03}_{:03}/doodad.g", cell.0, cell.1)
}

/// `.../cells/024_016/doodad.g` -> (24, 16).
pub fn cell_of_name(name: &str) -> Option<(i32, i32)> {
    let mut parts = name.rsplit('/');
    if parts.next()? != "doodad.g" {
        return None;
    }
    let (x, y) = parts.next()?.split_once('_')?;
    Some((x.parse().ok()?, y.parse().ok()?))
}

/// The cell owning a world position.
pub fn cell_at(pos: Vec3) -> (i32, i32) {
    ((pos.x / CELL).floor() as i32, (pos.y / CELL).floor() as i32)
}

pub fn origin(cell: (i32, i32)) -> Vec2 {
    Vec2::new(cell.0 as f32 * CELL, cell.1 as f32 * CELL)
}

#[derive(Clone, Debug)]
pub struct Block {
    pub text: String,
    pub type_id: u32,
    pub local: Vec3,
    pub ori: Quat,
    pub scale: f32,
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

/// `( x 1, y 2, z 3 )` or `( x 0, y 0, z 0, w 1 )`.
fn parse_tuple<const N: usize>(line: &str, key: &str) -> Option<[f32; N]> {
    let inner = value(line, key)?.strip_prefix('(')?.strip_suffix(')')?;
    let mut v = [f32::NAN; N];
    for part in inner.split(',') {
        let (k, n) = part.trim().split_once(char::is_whitespace)?;
        let i = ["x", "y", "z", "w"][..N].iter().position(|a| *a == k)?;
        v[i] = n.trim().parse().ok()?;
    }
    v.iter().all(|f| f.is_finite()).then_some(v)
}

fn parse_block(text: &str) -> Result<Block> {
    let (mut type_id, mut local, mut ori, mut scale) = (None, None, Quat::IDENTITY, 1.0);
    for line in text.lines() {
        if let Some(v) = value(line, "type") {
            type_id = v.parse().ok();
        } else if let Some([x, y, z]) = parse_tuple::<3>(line, "pos") {
            local = Some(Vec3::new(x, y, z));
        } else if let Some([x, y, z, w]) = parse_tuple::<4>(line, "ori") {
            ori = Quat::from_xyzw(x, y, z, w).normalize();
        } else if let Some(s) = value(line, "scale").and_then(|v| v.parse::<f32>().ok()).filter(|s| s.is_finite() && *s > 0.0) {
            scale = s;
        }
    }
    let type_id = type_id.context("doodad without type")?;
    let local = local.with_context(|| format!("doodad {type_id}: invalid pos"))?;
    Ok(Block { text: text.to_string(), type_id, local, ori, scale })
}

pub fn parse(text: &str) -> Result<File> {
    let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut file = File { eol, ..Default::default() };
    let mut current: Option<String> = None;
    for line in text.split_inclusive('\n') {
        if line.trim_end() == "doodad" {
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
    if !file.header.trim().is_empty() {
        bail!("text before the first doodad");
    }
    Ok(file)
}

/// Yaw, pitch, roll in degrees, the order [`crate::spawns::Placement::rotation`] uses.
pub fn angles(q: Quat) -> (f32, f32, f32) {
    let (z, y, x) = q.to_euler(EulerRot::ZYX);
    (z.to_degrees(), y.to_degrees(), x.to_degrees())
}

pub fn quat(yaw: f32, pitch: f32, roll: f32) -> Quat {
    Quat::from_euler(EulerRot::ZYX, yaw.to_radians(), pitch.to_radians(), roll.to_radians())
}

/// `block` with `pos` replaced, and `ori` / `scale` when given; indentation and all other lines unchanged.
pub fn rewrite(block: &str, local: Vec3, ori: Option<Quat>, scale: Option<f32>) -> String {
    let mut out = String::with_capacity(block.len());
    let mut has_scale = false;
    let mut eol = "\n";
    let mut indent = "    ".to_string();
    for line in block.split_inclusive('\n') {
        let body = line.trim_end_matches(['\r', '\n']);
        let ending = &line[body.len()..];
        if !ending.is_empty() {
            eol = if ending == "\r\n" { "\r\n" } else { "\n" };
        }
        let pad = &body[..body.len() - body.trim_start().len()];
        if !pad.is_empty() {
            indent = pad.to_string();
        }
        let replaced = if body.trim_start().starts_with("pos ") {
            Some(format!("pos ( x {}, y {}, z {} )", fmt_g(local.x), fmt_g(local.y), fmt_g(local.z)))
        } else if body.trim_start().starts_with("ori ") {
            ori.map(|q| format!("ori ( x {}, y {}, z {}, w {} )", fmt_g(q.x), fmt_g(q.y), fmt_g(q.z), fmt_g(q.w)))
        } else if value(body, "scale").is_some() {
            has_scale = true;
            scale.map(|s| format!("scale {}", fmt_g(s)))
        } else {
            None
        };
        match replaced {
            Some(r) => {
                out.push_str(pad);
                out.push_str(&r);
                out.push_str(ending);
            }
            None => out.push_str(line),
        }
    }
    if let Some(s) = scale.filter(|s| !has_scale && *s != 1.0) {
        if !out.ends_with('\n') {
            out.push_str(eol);
        }
        out.push_str(&format!("{indent}scale {}{eol}", fmt_g(s)));
    }
    out
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

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "doodad\r\n    category 17\r\n    type 2768\r\n    family 0\r\n    vegetation false\r\n    pos ( x 150, y 549, z 100.05 )\r\n    ori ( x 0, y 0, z 0.7071068, w 0.7071068 )\r\n    scale 1\r\ndoodad\r\n    category 17\r\n    type 3484\r\n    family 0\r\n    vegetation false\r\n    pos ( x 253.565, y 977.835, z 0 )\r\n    ori ( x 0, y 0, z 0, w 1 )\r\n    scale 1\r\n";

    #[test]
    fn parses_and_writes_back_verbatim() {
        let f = parse(SAMPLE).unwrap();
        assert_eq!(f.blocks.len(), 2);
        assert_eq!(f.blocks[0].type_id, 2768);
        assert_eq!(f.blocks[0].local, Vec3::new(150.0, 549.0, 100.05));
        assert!((angles(f.blocks[0].ori).0 - 90.0).abs() < 0.01);
        assert_eq!(write(&f, f.blocks.iter().map(|b| b.text.clone())), SAMPLE);
    }

    #[test]
    fn rewrite_replaces_only_pos_ori_scale() {
        let f = parse(SAMPLE).unwrap();
        let text = rewrite(&f.blocks[1].text, Vec3::new(10.5, 20.0, 3.0), Some(quat(180.0, 0.0, 0.0)), None);
        assert!(text.contains("    pos ( x 10.5, y 20, z 3 )\r\n"), "{text}");
        assert!(text.contains("    ori ( x 0, y 0, z 1, w 0 )\r\n"), "{text}");
        assert!(text.contains("    category 17\r\n    type 3484\r\n"), "{text}");
        assert!(text.ends_with("    scale 1\r\n"), "{text}");
        let again = parse(&text).unwrap();
        assert_eq!(again.blocks[0].local, Vec3::new(10.5, 20.0, 3.0));
    }

    #[test]
    fn rewrite_adds_missing_scale() {
        let text = rewrite("doodad\n    type 5\n    pos ( x 1, y 2, z 3 )\n", Vec3::ONE, None, Some(2.0));
        assert!(text.ends_with("    scale 2\n"), "{text}");
    }

    /// Every cell file of the main server data parses and writes back byte for byte (skipped without that folder).
    #[test]
    fn real_cells_round_trip() {
        let dir = std::path::Path::new("server/game/worlds/main_world/level_design/cells");
        if !dir.is_dir() {
            return;
        }
        let mut count = 0;
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path().join("doodad.g");
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            let f = parse(&text).unwrap_or_else(|e| panic!("{}: {e:#}", path.display()));
            assert_eq!(write(&f, f.blocks.iter().map(|b| b.text.clone())), text, "{}", path.display());
            count += f.blocks.len();
        }
        assert_eq!(count, 80357);
    }

    #[test]
    fn cell_names() {
        assert_eq!(cell_of_name("worlds/main_world/level_design/cells/024_016/doodad.g"), Some((24, 16)));
        assert_eq!(file_name("main_world", (4, 16)), "worlds/main_world/level_design/cells/004_016/doodad.g");
        assert_eq!(cell_at(Vec3::new(24.0 * 1024.0 + 5.0, 16.0 * 1024.0, 0.0)), (24, 16));
    }
}
