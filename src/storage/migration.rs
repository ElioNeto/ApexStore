//! Offline encryption migration into a new database directory. Originals are
//! retained, and a failed conversion removes only its private staging directory.
use super::{
    builder::SstableBuilder, cache::GlobalBlockCache, encryption::EncryptionConfig,
    reader::SstableReader,
};
use crate::infra::{
    config::StorageConfig,
    error::{LsmError, Result},
};
use fs2::FileExt;
use std::{
    fs::{File, OpenOptions},
    path::Path,
};

#[derive(Debug, Default, serde::Serialize)]
pub struct MigrationReport {
    pub sstables: u64,
    pub wal_frames: u64,
    pub copied_files: u64,
}

/// Rewrite active data and snapshots under an exclusive source database lock.
/// Output must not exist and must be outside the source. Symlinks and special
/// files are rejected, and any unreadable record aborts the whole migration.
pub fn migrate_encryption(
    source: &Path,
    output: &Path,
    source_encryption: &EncryptionConfig,
    target_encryption: &EncryptionConfig,
) -> Result<MigrationReport> {
    if !cfg!(any(
        target_os = "linux",
        target_os = "android",
        target_vendor = "apple",
        target_os = "redox"
    )) {
        return Err(LsmError::InvalidArgument(
            "atomic no-replace migration publication is unsupported on this platform".into(),
        ));
    }
    if target_encryption.enabled && target_encryption.key == [0; 32] {
        return Err(LsmError::InvalidArgument(
            "migration target must not use the legacy zero key".into(),
        ));
    }
    let source = source.canonicalize()?;
    if !source.is_dir() {
        return Err(LsmError::InvalidArgument(
            "source must be a database directory".into(),
        ));
    }
    let lock_path = source.join(".apexstore.lock");
    if std::fs::symlink_metadata(&lock_path).is_ok_and(|m| !m.file_type().is_file()) {
        return Err(LsmError::InvalidArgument(
            "database lock must be a regular file".into(),
        ));
    }
    let mut options = OpenOptions::new();
    options.create(true).truncate(false).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let lock = options.open(lock_path)?;
    lock.try_lock_exclusive().map_err(|e| {
        LsmError::InvalidArgument(format!(
            "stop the server and CLI before migration: database lock unavailable: {e}"
        ))
    })?;
    if output.exists() || output.symlink_metadata().is_ok() {
        return Err(LsmError::InvalidArgument(
            "migration output already exists; choose a new directory".into(),
        ));
    }
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let parent = parent.canonicalize()?;
    let name = output
        .file_name()
        .ok_or_else(|| LsmError::InvalidArgument("migration output name missing".into()))?;
    let output = parent.join(name);
    if output.starts_with(&source) {
        return Err(LsmError::InvalidArgument(
            "migration output must be outside the source".into(),
        ));
    }
    let staging = tempfile::Builder::new()
        .prefix(".apexstore-migration-")
        .tempdir_in(&parent)?;
    options.create_new(true);
    let destination_lock = options.open(staging.path().join(".apexstore.lock"))?;
    destination_lock.try_lock_exclusive()?;
    let mut report = MigrationReport::default();
    rewrite_directory(
        &source,
        staging.path(),
        source_encryption,
        target_encryption,
        &mut report,
    )?;
    sync_directory(staging.path())?;
    // Source and target database locks prevent cooperating engines from opening
    // either tree during conversion and publication. Existing targets are refused.
    if output.exists() || output.symlink_metadata().is_ok() {
        return Err(LsmError::InvalidArgument(
            "migration output appeared during conversion".into(),
        ));
    }
    publish_directory(staging.path(), &output)?;
    sync_directory(&parent)?;
    drop(destination_lock);
    drop(lock);
    Ok(report)
}

fn rewrite_directory(
    source: &Path,
    output: &Path,
    from: &EncryptionConfig,
    to: &EncryptionConfig,
    report: &mut MigrationReport,
) -> Result<()> {
    let mut entries = std::fs::read_dir(source)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let kind = entry.file_type()?;
        if !kind.is_file() && !kind.is_dir() {
            return Err(LsmError::InvalidArgument(format!(
                "migration refuses symlink/special file {}",
                entry.path().display()
            )));
        }
        let name = entry.file_name();
        if name == ".apexstore.lock" {
            continue;
        }
        let input = entry.path();
        let target = output.join(&name);
        if kind.is_dir() {
            // Snapshots may contain another independently opened database.
            // Refuse live nested databases rather than copy a changing WAL.
            let nested_lock_path = input.join(".apexstore.lock");
            let _nested_lock = if nested_lock_path.exists() {
                if !nested_lock_path.symlink_metadata()?.file_type().is_file() {
                    return Err(LsmError::InvalidArgument(
                        "nested database lock must be a regular file".into(),
                    ));
                }
                let lock = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(nested_lock_path)?;
                lock.try_lock_exclusive().map_err(|error| {
                    LsmError::InvalidArgument(format!(
                        "stop nested database {} before migration: {error}",
                        input.display()
                    ))
                })?;
                Some(lock)
            } else {
                None
            };
            std::fs::create_dir(&target)?;
            rewrite_directory(&input, &target, from, to, report)?;
            sync_directory(&target)?;
        } else if input.extension().is_some_and(|ext| ext == "sst") {
            rewrite_sstable(&input, &target, from, to)?;
            report.sstables += 1;
        } else if name.to_str().is_some_and(|name| {
            name == "wal.log" || (name.starts_with("wal-") && name.ends_with(".log"))
        }) {
            report.wal_frames += super::wal::rewrite_for_migration(&input, &target, from, to)?;
        } else {
            std::fs::copy(&input, &target)?;
            File::open(&target)?.sync_all()?;
            report.copied_files += 1;
        }
    }
    Ok(())
}

fn rewrite_sstable(
    source: &Path,
    target: &Path,
    from: &EncryptionConfig,
    to: &EncryptionConfig,
) -> Result<()> {
    let reader = SstableReader::open_with_encryption(
        source.to_path_buf(),
        StorageConfig::default(),
        GlobalBlockCache::new(1, 4096),
        from,
    )?;
    let metadata = reader.metadata();
    let max_block = metadata
        .blocks
        .iter()
        .map(|block| block.uncompressed_size as usize)
        .max()
        .unwrap_or(4096);
    let config = StorageConfig {
        block_size: max_block.clamp(
            4096,
            crate::core::engine::MAX_VALUE_SIZE + crate::core::engine::MAX_KEY_SIZE + 4096,
        ),
        ..StorageConfig::default()
    };
    let mut builder =
        SstableBuilder::new_with_encryption(target.to_path_buf(), config, metadata.timestamp, to)?;
    let mut count = 0;
    for (index, block) in metadata.blocks.iter().enumerate() {
        let end = metadata
            .blocks
            .get(index + 1)
            .map(|next| next.first_key.as_slice());
        for (key, record) in reader.scan_range(Some(&block.first_key), end)? {
            if key != record.key {
                return Err(LsmError::CorruptedData(
                    "SSTable key differs from its record key".into(),
                ));
            }
            builder.add(&key, &record)?;
            count += 1;
        }
    }
    if count != metadata.record_count {
        return Err(LsmError::CorruptedData(
            "SSTable record count differs from metadata; refusing a partial migration".into(),
        ));
    }
    builder.finish()?;
    Ok(())
}

fn sync_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// Publish with an OS-enforced refusal to replace any existing destination,
/// including an empty directory created after the earlier existence check.
fn publish_directory(source: &Path, output: &Path) -> Result<()> {
    #[cfg(any(
        target_os = "linux",
        target_os = "android",
        target_vendor = "apple",
        target_os = "redox"
    ))]
    {
        rustix::fs::renameat_with(
            rustix::fs::CWD,
            source,
            rustix::fs::CWD,
            output,
            rustix::fs::RenameFlags::NOREPLACE,
        )
        .map_err(std::io::Error::from)?;
        Ok(())
    }
    #[cfg(not(any(
        target_os = "linux",
        target_os = "android",
        target_vendor = "apple",
        target_os = "redox"
    )))]
    {
        let _ = (source, output);
        Err(LsmError::InvalidArgument(
            "atomic no-replace migration publication is unsupported on this platform".into(),
        ))
    }
}

#[cfg(all(
    test,
    any(
        target_os = "linux",
        target_os = "android",
        target_vendor = "apple",
        target_os = "redox"
    )
))]
mod tests {
    #[test]
    fn publication_refuses_even_an_empty_destination_created_after_preflight() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("staging");
        let output = dir.path().join("output");
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("data"), b"retained").unwrap();
        // Simulate a competing mkdir between preflight and atomic publication.
        std::fs::create_dir(&output).unwrap();
        assert!(super::publish_directory(&source, &output).is_err());
        assert_eq!(std::fs::read(source.join("data")).unwrap(), b"retained");
        assert_eq!(std::fs::read_dir(output).unwrap().count(), 0);
    }
}
