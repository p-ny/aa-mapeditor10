//! Recoverable batch saves for native objects spanning multiple cells.
//! A journal blocks publication until every destination is installed or restored.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Component, Path, PathBuf},
};
const JOURNAL: &str = ".object-save-recovery";
#[derive(Serialize, Deserialize)]
struct Entry {
    path: PathBuf,
    existed: bool,
}
fn lock(root: &Path) -> Result<File> {
    fs::create_dir_all(root)?;
    let name = root
        .file_name()
        .context("overlay has no name")?
        .to_string_lossy();
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.with_file_name(format!("{name}.object-save.lock")))?;
    file.try_lock()
        .context("another editor is saving or recovering this overlay")?;
    Ok(file)
}
fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    fs::create_dir_all(path.parent().context("file has no parent")?)?;
    let mut file = File::create(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}
fn relative(path: &Path) -> Result<()> {
    ensure!(
        path.components().all(|c| matches!(c, Component::Normal(_))) && path.starts_with("worlds"),
        "invalid object-save path"
    );
    Ok(())
}
fn recover_locked(root: &Path) -> Result<bool> {
    let dir = root.join(JOURNAL);
    if !dir.exists() {
        return Ok(false);
    }
    ensure!(
        !dir.symlink_metadata()?.file_type().is_symlink(),
        "object-save journal is a symlink"
    );
    if dir.join("applying").exists() {
        let entries: Vec<Entry> = serde_json::from_slice(&fs::read(dir.join("manifest.json"))?)?;
        // Validate every backup before restoring anything.
        for e in &entries {
            relative(&e.path)?;
            if e.existed {
                ensure!(
                    dir.join("before").join(&e.path).is_file(),
                    "missing object-save backup"
                );
            }
        }
        for e in entries {
            let path = root.join(&e.path);
            if e.existed {
                write(&path, &fs::read(dir.join("before").join(&e.path))?)?;
            } else if path.exists() {
                fs::remove_file(path)?;
            }
        }
    }
    fs::remove_dir_all(dir)?;
    Ok(true)
}
/// Run before the editor opens the pak, so a crashed transfer is never loaded halfway.
pub fn recover(root: &Path) -> Result<bool> {
    let _lock = lock(root)?;
    recover_locked(root)
}
pub fn save(root: &Path, files: &[(PathBuf, Vec<u8>)]) -> Result<()> {
    save_impl(root, files, None)
}
fn save_impl(root: &Path, files: &[(PathBuf, Vec<u8>)], fail_after: Option<usize>) -> Result<()> {
    let _lock = lock(root)?;
    recover_locked(root)?;
    if files.is_empty() {
        return Ok(());
    }
    let mut seen = std::collections::HashSet::new();
    for (p, _) in files {
        relative(p)?;
        ensure!(seen.insert(p), "duplicate object-save target");
    }
    let dir = root.join(JOURNAL);
    fs::create_dir(&dir)?;
    let result = (|| -> Result<()> {
        let mut entries = Vec::new();
        for (rel, bytes) in files {
            let target = root.join(rel);
            let existed = target.try_exists()?;
            if existed {
                write(&dir.join("before").join(rel), &fs::read(&target)?)?;
            }
            write(&dir.join("after").join(rel), bytes)?;
            entries.push(Entry {
                path: rel.clone(),
                existed,
            });
        }
        write(&dir.join("manifest.json"), &serde_json::to_vec(&entries)?)?;
        write(
            &dir.join("applying"),
            b"Recover before opening editor or publishing",
        )?;
        for (i, (rel, bytes)) in files.iter().enumerate() {
            if fail_after == Some(i) {
                anyhow::bail!("injected batch-write failure")
            }
            write(&root.join(rel), bytes)?;
        }
        // Commit marker: once absent, recovery keeps the fully written batch.
        fs::remove_file(dir.join("applying"))?;
        Ok(())
    })();
    if let Err(error) = result {
        recover_locked(root).context(format!(
            "object save failed: {error:#}; recovery also failed"
        ))?;
        return Err(error);
    }
    fs::remove_dir_all(dir)?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn root() -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "editor10-object-save-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&p).unwrap();
        p
    }
    #[test]
    fn failed_transfer_save_restores_existing_and_removes_created_files() {
        let r = root();
        let a = PathBuf::from("worlds/test/cells/001_001/client/brush.dat");
        let b = PathBuf::from("worlds/test/cells/002_001/client/object.dat");
        let c = PathBuf::from("worlds/test/cells/002_001/client/brush.dat");
        write(&r.join(&a), b"original").unwrap();
        let files = vec![
            (a.clone(), b"removed-source".to_vec()),
            (b.clone(), b"inserted-target".to_vec()),
            (c, b"twin".to_vec()),
        ];
        assert!(save_impl(&r, &files, Some(2)).is_err());
        assert_eq!(fs::read(r.join(&a)).unwrap(), b"original");
        assert!(!r.join(&b).exists());
        assert!(!r.join(JOURNAL).exists());
        save(&r, &files).unwrap();
        assert_eq!(fs::read(r.join(&a)).unwrap(), b"removed-source");
        assert_eq!(fs::read(r.join(&b)).unwrap(), b"inserted-target");
        fs::remove_dir_all(r).unwrap();
    }
    #[test]
    fn interrupted_save_recovers_before_any_pak_read() {
        let r = root();
        let rel = PathBuf::from("worlds/test/cells/001_001/client/brush.dat");
        let dir = r.join(JOURNAL);
        write(&dir.join("before").join(&rel), b"original").unwrap();
        write(&r.join(&rel), b"partial").unwrap();
        write(
            &dir.join("manifest.json"),
            &serde_json::to_vec(&vec![Entry {
                path: rel.clone(),
                existed: true,
            }])
            .unwrap(),
        )
        .unwrap();
        write(&dir.join("applying"), b"yes").unwrap();
        assert!(recover(&r).unwrap());
        assert_eq!(fs::read(r.join(&rel)).unwrap(), b"original");
        assert!(!recover(&r).unwrap());
        fs::remove_dir_all(r).unwrap();
    }
    #[test]
    fn concurrent_editor_recovery_refuses_an_active_save() {
        let r = root();
        let held = lock(&r).unwrap();
        assert!(recover(&r).is_err());
        drop(held);
        assert!(!recover(&r).unwrap());
        fs::remove_dir_all(r).unwrap();
    }
}
