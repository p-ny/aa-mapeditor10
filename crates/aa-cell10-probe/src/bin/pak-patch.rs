//! `pak-patch apply <pak> <undo> <pak path>=<file> ...` adds or replaces entries with an undo log and reads them back;
//! `pak-patch undo <pak> <undo>` restores the pak from that log.
use anyhow::{Context, Result, bail, ensure};
fn main() -> Result<()> {
    unsafe { std::env::remove_var("AA_PAK_OVERLAY"); }
    let a: Vec<String> = std::env::args().skip(1).collect();
    match a.first().map(String::as_str) {
        Some("apply") if a.len() >= 4 => {
            let (pak, undo) = (&a[1], &a[2]);
            ensure!(!std::path::Path::new(undo).exists(), "undo file already exists: {undo}");
            let files = a[3..]
                .iter()
                .map(|p| {
                    let (name, src) = p.split_once('=').context("expected <pak path>=<file>")?;
                    ensure!(name.starts_with("game/") && !name.contains(".."), "not a game_pak path: {name}");
                    Ok((name.to_string(), std::fs::read(src).with_context(|| format!("read {src}"))?))
                })
                .collect::<Result<Vec<_>>>()?;
            let report = aa_pak::write::patch_with_undo(pak, &files, Some(undo))?;
            let readback = aa_pak::Pak::open(pak)?;
            for (name, bytes) in &files {
                ensure!(readback.read(name)? == *bytes, "readback failed: {name}");
            }
            println!("patched {} entries ({} new), readback verified: {report:?}", files.len(), report.new_names);
        }
        Some("undo") if a.len() == 3 => {
            aa_pak::write::undo(&a[1], &a[2])?;
            println!("restored {}", a[1]);
        }
        _ => bail!("pak-patch apply <pak> <undo> <pak path>=<file>... | pak-patch undo <pak> <undo>"),
    }
    Ok(())
}
