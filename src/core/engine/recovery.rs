//! One inspection/quarantine policy for disk discovery and snapshot restore.
use super::EngineCore;
use crate::{
    core::table::TableLoad,
    infra::error::{LsmError, Result},
    storage::{cache::Cache, encryption::EncryptionConfig},
};
use std::{
    path::{Path, PathBuf},
    sync::atomic::Ordering,
};

pub(super) struct Candidate {
    pub cf: String,
    pub path: PathBuf,
    pub outcome: TableLoad,
}

pub(super) fn table_path(directory: &Path, name: &str) -> Result<PathBuf> {
    let path = Path::new(name);
    if path.components().count() != 1
        || !matches!(
            path.components().next(),
            Some(std::path::Component::Normal(_))
        )
        || path.extension().is_none_or(|ext| ext != "sst")
    {
        return Err(LsmError::InvalidArgument(
            "manifest SSTable names must be plain .sst filenames".into(),
        ));
    }
    Ok(directory.join(path))
}

/// Inspect every file before moving anything. In particular a wrong key in any
/// table must abort without quarantining earlier files in the same directory.
pub(super) fn inspect(
    paths: Vec<(String, PathBuf)>,
    encryption: &EncryptionConfig,
) -> Result<Vec<Candidate>> {
    let mut seen = std::collections::HashSet::new();
    paths
        .into_iter()
        .map(|(cf, path)| {
            if !seen.insert(path.clone()) {
                return Err(LsmError::InvalidArgument(
                    "duplicate SSTable in manifest".into(),
                ));
            }
            if !std::fs::symlink_metadata(&path)?.file_type().is_file() {
                return Err(LsmError::InvalidArgument(
                    "SSTable must be a regular file".into(),
                ));
            }
            let outcome = crate::core::table::Table::load_for_recovery(&path, encryption)?;
            Ok(Candidate { cf, path, outcome })
        })
        .collect()
}

pub(super) fn quarantine(path: &Path, directory: &Path, error: &str) -> Result<()> {
    let filename = path
        .file_name()
        .ok_or_else(|| LsmError::InvalidArgument("SSTable filename missing".into()))?;
    let quarantine_dir = directory.join("quarantine");
    std::fs::create_dir_all(&quarantine_dir)?;
    let mut target = quarantine_dir.join(filename);
    let mut suffix = 0_u64;
    while target.exists() || target.with_extension("sst.quarantine-reason").exists() {
        suffix = suffix.checked_add(1).ok_or_else(|| {
            LsmError::InvalidArgument("quarantine filename space exhausted".into())
        })?;
        target = quarantine_dir.join(format!("{}-{}", suffix, filename.to_string_lossy()));
    }
    let reason = serde_json::json!({
        "original_filename":filename.to_string_lossy(),
        "error":error,
        "engine_version":env!("CARGO_PKG_VERSION"),
        "timestamp_unix_ms":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_millis(),
    });
    let mut temporary = tempfile::NamedTempFile::new_in(&quarantine_dir)?;
    serde_json::to_writer_pretty(temporary.as_file_mut(), &reason)?;
    temporary.as_file().sync_all()?;
    let reason_path = target.with_extension("sst.quarantine-reason");
    temporary
        .persist_noclobber(&reason_path)
        .map_err(|e| LsmError::Io(e.error))?;
    // Persist the reason before moving data. No file is discarded or overwritten.
    #[cfg(unix)]
    std::fs::File::open(&quarantine_dir)?.sync_all()?;
    std::fs::rename(path, &target)?;
    #[cfg(unix)]
    {
        std::fs::File::open(&quarantine_dir)?.sync_all()?;
        std::fs::File::open(directory)?.sync_all()?;
    }
    tracing::warn!(target:"apexstore::recovery", path=%path.display(), destination=%target.display(), %error, "quarantined structurally unreadable SSTable");
    Ok(())
}

pub(super) fn count_quarantined(directory: &Path) -> Result<u64> {
    let directory = directory.join("quarantine");
    if !directory.exists() {
        return Ok(0);
    }
    let mut count = 0;
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        if entry.file_type()?.is_file() && entry.path().extension().is_some_and(|ext| ext == "sst")
        {
            count += 1;
        }
    }
    Ok(count)
}

pub(super) fn apply<C: Cache>(
    core: &mut EngineCore<C>,
    candidates: Vec<Candidate>,
    directory: &Path,
) -> Result<()> {
    for candidate in candidates {
        let table = match candidate.outcome {
            TableLoad::Healthy(table) => table,
            TableLoad::DegradedBloom(table) => {
                core.version_set()
                    .degraded_bloom_count
                    .fetch_add(1, Ordering::Relaxed);
                table
            }
            TableLoad::Corrupt(error) => {
                quarantine(&candidate.path, directory, &error.to_string())?;
                core.version_set()
                    .quarantined_count
                    .fetch_add(1, Ordering::Relaxed);
                core.version_set()
                    .unreadable_count
                    .fetch_add(1, Ordering::Relaxed);
                continue;
            }
        };
        core.version_set_mut().add_table(&candidate.cf, table);
    }
    Ok(())
}
