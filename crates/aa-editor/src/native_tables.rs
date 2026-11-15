//! Append-only native reference tables. Preserve every existing record and opaque suffix.
use anyhow::{Context, Result, ensure};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeTables {
    pub models: Vec<u8>,
    pub flat_materials: Option<Vec<u8>>,
    pub head: Vec<u8>,
    pub materials: Option<Vec<u8>>,
}
#[derive(Clone, Debug)]
pub struct Entries {
    pub flat: Vec<u8>,
    pub model: Vec<u8>,
    pub flat_material: Option<Vec<u8>>,
    pub material: Option<Vec<u8>>,
    pub external: Option<Vec<u8>>,
}
pub fn normalized(name: &str) -> String {
    name.replace('\\', "/")
        .to_ascii_lowercase()
        .trim_start_matches("game/")
        .to_string()
}
fn range(bytes: &[u8], start: usize, width: usize) -> Result<(usize, usize)> {
    let end = start.checked_add(4).context("table overflow")?;
    ensure!(end <= bytes.len(), "truncated reference table");
    let n = u32::from_le_bytes(bytes[start..end].try_into()?) as usize;
    let end = end
        .checked_add(n.checked_mul(width).context("table overflow")?)
        .context("table overflow")?;
    ensure!(end <= bytes.len(), "truncated reference records");
    Ok((n, end))
}
fn name(record: &[u8], prefix: usize) -> Result<String> {
    let data = record.get(prefix..).context("truncated reference name")?;
    let end = data
        .iter()
        .position(|&v| v == 0)
        .context("unterminated native reference name")?;
    Ok(std::str::from_utf8(&data[..end])?.trim().to_string())
}
fn entry(bytes: &[u8], start: usize, width: usize, index: usize) -> Result<Vec<u8>> {
    let (n, _) = range(bytes, start, width)?;
    ensure!(index < n, "missing native reference record");
    Ok(bytes[start + 4 + index * width..start + 4 + (index + 1) * width].to_vec())
}
fn find(bytes: &[u8], start: usize, width: usize, wanted: &str) -> Result<Option<usize>> {
    let (n, _) = range(bytes, start, width)?;
    for i in 0..n {
        if normalized(&name(&entry(bytes, start, width, i)?, width - 256)?) == normalized(wanted) {
            return Ok(Some(i));
        }
    }
    Ok(None)
}
fn append(bytes: &mut Vec<u8>, start: usize, width: usize, record: &[u8]) -> Result<usize> {
    ensure!(record.len() == width, "invalid reference record width");
    name(record, width - 256)?;
    let (n, end) = range(bytes, start, width)?;
    let next = u32::try_from(n)?
        .checked_add(1)
        .context("reference table exhausted")?;
    bytes.splice(end..end, record.iter().copied());
    bytes[start..start + 4].copy_from_slice(&next.to_le_bytes());
    Ok(n)
}
impl NativeTables {
    pub fn entries(&self, flat: usize, model: usize, flat_material: i32, material: i32) -> Result<Entries> {
        let (_, at) = range(&self.head, 0, 260)?;
        let material = if material == -1 {
            None
        } else {
            ensure!(material >= 0, "invalid material index");
            Some(entry(&self.head, at, 260, material as usize)?)
        };
        let flat_material = if flat_material == -1 { None } else {
            ensure!(flat_material >= 0, "invalid flat material index");
            Some(entry(self.flat_materials.as_ref().context("missing native materials.dat")?, 0, 256, flat_material as usize)?)
        };
        ensure!(flat_material.as_ref().map(|r|name(r,0)).transpose()?.map(|n|normalized(&n)) == material.as_ref().map(|r|name(r,4)).transpose()?.map(|n|normalized(&n)), "ambiguous native brush copy: material reference differs");
        let external = if let (Some(table), Some(record)) = (&self.materials, &material) {
            find(table, 0, 260, &name(record, 4)?)?
                .map(|i| entry(table, 0, 260, i))
                .transpose()?
        } else {
            None
        };
        Ok(Entries {
            flat: entry(&self.models, 0, 256, flat)?,
            model: entry(&self.head, 0, 260, model)?,
            material,
            flat_material,
            external,
        })
    }
    /// Returns independently remapped flat/ octree model and material indices.
    pub fn import(
        &mut self,
        model: &str,
        material: Option<&str>,
        records: &Entries,
    ) -> Result<(u32, u32, i32, i32)> {
        ensure!(
            normalized(&name(&records.flat, 0)?) == normalized(model)
                && normalized(&name(&records.model, 4)?) == normalized(model),
            "source model records disagree"
        );
        let flat = match find(&self.models, 0, 256, model)? {
            Some(i) => i,
            None => append(&mut self.models, 0, 256, &records.flat)?,
        };
        let object = match find(&self.head, 0, 260, model)? {
            Some(i) => i,
            None => append(&mut self.head, 0, 260, &records.model)?,
        };
        let (_, at) = range(&self.head, 0, 260)?;
        let mut flat_mat = -1;
        let mat = if let Some(wanted) = material {
            let record = records
                .material
                .as_ref()
                .context("missing source material record")?;
            ensure!(
                normalized(&name(record, 4)?) == normalized(wanted),
                "source material records disagree"
            );
            let flat_record = records.flat_material.as_ref().context("missing source materials.dat record")?;
            ensure!(normalized(&name(flat_record, 0)?) == normalized(wanted), "source flat material record disagrees");
            let flat_table = self.flat_materials.as_mut().context("destination has no native materials.dat")?;
            let i = aligned_material(&mut self.head, at, flat_table, wanted, record, flat_record)?;
            flat_mat = i32::try_from(i)?;
            if let Some(external) = self.materials.as_mut() {
                if find(external, 0, 260, wanted)?.is_none() {
                    let raw = records.external.as_ref().unwrap_or(record);
                    ensure!(
                        normalized(&name(raw, 4)?) == normalized(wanted),
                        "source external material record disagrees"
                    );
                    append(external, 0, 260, raw)?;
                }
            }
            i32::try_from(i)?
        } else {
            -1
        };
        Ok((u32::try_from(flat)?, u32::try_from(object)?, flat_mat, mat))
    }
    pub fn display_materials(&self) -> Result<Vec<String>> {
        let (_, at) = range(&self.head, 0, 260)?;
        let (n, _) = range(&self.head, at, 260)?;
        (0..n)
            .map(|i| name(&entry(&self.head, at, 260, i)?, 4))
            .collect()
    }
}
fn prefix(
    old: &[u8],
    old_at: usize,
    new: &[u8],
    new_at: usize,
    width: usize,
) -> Result<(usize, usize)> {
    let (a, end_a) = range(old, old_at, width)?;
    let (b, end_b) = range(new, new_at, width)?;
    ensure!(
        b >= a && old[old_at + 4..end_a] == new[new_at + 4..new_at + 4 + a * width],
        "native reference tables must append without changing existing entries or order"
    );
    Ok((end_a, end_b))
}
/// Entry `index` of the material table of an `object.dat` head: its name and 260-byte record.
pub fn tree_material(head: &[u8], index: usize) -> Result<(String, Vec<u8>)> {
    let (_, at) = range(head, 0, 260)?;
    let record = entry(head, at, 260, index)?;
    Ok((name(&record, 4)?, record))
}

/// The `material_list.dat` record named `wanted`.
pub fn external_material(table: &[u8], wanted: &str) -> Result<Option<Vec<u8>>> {
    find(table, 0, 260, wanted)?.map(|i| entry(table, 0, 260, i)).transpose()
}

/// Index of material `wanted` in the material table of an `object.dat` head, appending `record` (and `external`, else
/// `record`, to `material_list.dat`) when the cell does not have it.
pub fn import_tree_material(head: &mut Vec<u8>, flat_table: Option<&mut Vec<u8>>, external_table: Option<&mut Vec<u8>>, wanted: &str, record: &[u8], external: Option<&[u8]>) -> Result<u32> {
    let (_, at) = range(head, 0, 260)?;
    let i = match flat_table {
        Some(flat) => {
            ensure!(record.len() == 260, "invalid reference record width");
            aligned_material(head, at, flat, wanted, record, &record[4..])?
        }
        None => match find(head, at, 260, wanted)? {
            Some(i) => i,
            None => append(head, at, 260, record)?,
        },
    };
    if let Some(table) = external_table
        && find(table, 0, 260, wanted)?.is_none()
    {
        append(table, 0, 260, external.unwrap_or(record))?;
    }
    Ok(u32::try_from(i)?)
}

/// Index of material `wanted` that is the same in the `object.dat` material table (at `at` in `head`) and in
/// `materials.dat`. Retail cells keep the two tables identical and `brush.dat` resolves its material ids through
/// the `object.dat` one, so a material must never sit at different indices in them. When no index holds it in
/// both, the shorter table is first padded with the other's entries, then the material is appended to both.
fn aligned_material(head: &mut Vec<u8>, at: usize, flat: &mut Vec<u8>, wanted: &str, record: &[u8], flat_record: &[u8]) -> Result<usize> {
    let (n_head, _) = range(head, at, 260)?;
    let (n_flat, _) = range(flat, 0, 256)?;
    let wanted = normalized(wanted);
    for i in 0..n_head.min(n_flat) {
        if normalized(&name(&entry(head, at, 260, i)?, 4)?) == wanted
            && normalized(&name(&entry(flat, 0, 256, i)?, 0)?) == wanted
        {
            return Ok(i);
        }
    }
    for i in n_flat..n_head {
        let r = entry(head, at, 260, i)?;
        append(flat, 0, 256, &r[4..])?;
    }
    for i in n_head..n_flat {
        let mut r = vec![0u8; 260];
        r[4..].copy_from_slice(&entry(flat, 0, 256, i)?);
        append(head, at, 260, &r)?;
    }
    let i = append(head, at, 260, record)?;
    ensure!(append(flat, 0, 256, flat_record)? == i, "material tables out of step");
    Ok(i)
}

pub fn validate_append(old: &[u8], new: &[u8], object: bool, width: usize) -> Result<()> {
    let (mut a, mut b) = prefix(old, 0, new, 0, width)?;
    if object {
        (a, b) = prefix(old, a, new, b, 260)?;
    }
    ensure!(
        old[a..] == new[b..],
        "native reference table opaque suffix changed"
    );
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn record(name: &str, width: usize, tag: u32) -> Vec<u8> {
        let mut b = vec![0; width];
        let p = width - 256;
        if p == 4 {
            b[..4].copy_from_slice(&tag.to_le_bytes())
        }
        b[p..p + name.len()].copy_from_slice(name.as_bytes());
        b
    }
    fn table(r: &[u8]) -> Vec<u8> {
        let mut b = 1u32.to_le_bytes().to_vec();
        b.extend(r);
        b
    }
    #[test]
    fn appends_raw_records_preserves_indices_suffix_and_deduplicates() {
        let mut head = table(&record("old.cgf", 260, 7));
        head.extend(table(&record("old.mtl", 260, 8)));
        let mut t = NativeTables {
            models: table(&record("old.cgf", 256, 0)),
            flat_materials: Some(table(&record("old.mtl",256,0))),
            head,
            materials: Some(table(&record("other.mtl", 260, 9))),
        };
        append(t.flat_materials.as_mut().unwrap(), 0, 256, &record("other-flat.mtl",256,0)).unwrap();
        t.models.extend(b"opaque");
        let before = t.clone();
        let e = Entries {
            flat: record("new.cgf", 256, 0),
            model: record("new.cgf", 260, 91),
            material: Some(record("new.mtl", 260, 92)),
            flat_material: Some(record("new.mtl",256,0)),
            external: Some(record("new.mtl", 260, 93)),
        };
        // materials.dat already has an extra entry: object.dat is padded so the material lands at one index in both
        assert_eq!(t.import("new.cgf", Some("new.mtl"), &e).unwrap(), (1, 1, 2, 2));
        let once = t.clone();
        assert_eq!(
            t.import("GAME/NEW.CGF", Some("NEW.MTL"), &e).unwrap(),
            (1, 1, 2, 2)
        );
        assert_eq!(t, once);
        let exported=t.entries(1,1,2,2).unwrap();
        assert_eq!(exported.flat_material,e.flat_material);
        assert_eq!(exported.material,e.material);
        validate_append(before.flat_materials.as_ref().unwrap(),t.flat_materials.as_ref().unwrap(),false,256).unwrap();
        validate_append(&before.models, &t.models, false, 256).unwrap();
        validate_append(&before.head, &t.head, true, 260).unwrap();
        validate_append(
            before.materials.as_ref().unwrap(),
            t.materials.as_ref().unwrap(),
            false,
            260,
        )
        .unwrap();
        assert_eq!(entry(&t.head, 0, 260, 1).unwrap(), e.model);
        let (_, at) = range(&t.head, 0, 260).unwrap();
        assert_eq!(
            entry(&t.head, at, 260, 2).unwrap(),
            e.material.as_ref().unwrap().clone()
        );
        let mut corrupt = t.head.clone();
        corrupt[8] ^= 1;
        assert!(validate_append(&before.head, &corrupt, true, 260).is_err());
        assert!(
            NativeTables {
                models: vec![0; 3],
                ..before.clone()
            }
            .import("new.cgf", None, &e)
            .is_err()
        );
    }
    #[test]
    fn road_material_keeps_materials_dat_in_step_with_object_dat() {
        let mut head = table(&record("old.cgf", 260, 7));
        head.extend(table(&record("old.mtl", 260, 8)));
        let mut flat = table(&record("old.mtl", 256, 0));
        let mut list = table(&record("old.mtl", 260, 8));
        let road = record("road.mtl", 260, 0);
        assert_eq!(import_tree_material(&mut head, Some(&mut flat), Some(&mut list), "road.mtl", &road, None).unwrap(), 1);
        let (_, at) = range(&head, 0, 260).unwrap();
        assert_eq!(name(&entry(&flat, 0, 256, 1).unwrap(), 0).unwrap(), "road.mtl");
        // a brush copied in afterwards gets the same index in both tables
        let mut t = NativeTables { models: table(&record("old.cgf", 256, 0)), flat_materials: Some(flat), head, materials: Some(list) };
        let e = Entries {
            flat: record("new.cgf", 256, 0),
            model: record("new.cgf", 260, 91),
            material: Some(record("new.mtl", 260, 92)),
            flat_material: Some(record("new.mtl", 256, 0)),
            external: None,
        };
        let (_, _, flat_mat, mat) = t.import("new.cgf", Some("new.mtl"), &e).unwrap();
        assert_eq!((flat_mat, mat), (2, 2));
        let (_, at) = range(&t.head, 0, 260).unwrap();
        assert_eq!(range(t.flat_materials.as_ref().unwrap(), 0, 256).unwrap().0, range(&t.head, at, 260).unwrap().0);
    }

    #[test]
    fn diverged_tables_reuse_an_index_where_both_agree() {
        // 026_015 after the repair: index 1 differs, the material sits again at index 3 in both
        let mut head = table(&record("a.cgf", 260, 0));
        let mut mats = table(&record("x.mtl", 260, 0));
        for n in ["road.mtl", "lamp.mtl", "lamp.mtl"] { append(&mut mats, 0, 260, &record(n, 260, 0)).unwrap(); }
        head.extend(mats);
        let mut flat = table(&record("x.mtl", 256, 0));
        for n in ["lamp.mtl", "road.mtl", "lamp.mtl"] { append(&mut flat, 0, 256, &record(n, 256, 0)).unwrap(); }
        let (_, at) = range(&head, 0, 260).unwrap();
        let before = (head.clone(), flat.clone());
        assert_eq!(aligned_material(&mut head, at, &mut flat, "LAMP.mtl", &record("lamp.mtl", 260, 0), &record("lamp.mtl", 256, 0)).unwrap(), 3);
        assert_eq!((head, flat), before);
    }
}
