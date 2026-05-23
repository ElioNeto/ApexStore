//! Automatic backup scheduling.
//!
//! Periodically creates engine snapshots with configurable intervals and
//! retention policies. Integrates with the engine's existing `create_snapshot`
//! / `restore_snapshot` / `list_snapshots` API.
//!
//! # Usage
//!
//! ```rust
//! use apexstore::infra::backup_scheduler::BackupScheduler;
//! use std::time::Duration;
//! use std::sync::Arc;
//!
//! // Create a scheduler (requires an engine reference)
//! // let scheduler = BackupScheduler::new(engine, "/path/to/backups");
//!
//! // Schedule automatic backups every 30 minutes
//! // scheduler.schedule(Duration::from_secs(1800));
//!
//! // Trigger an immediate backup
//! // scheduler.backup_now().unwrap();
//!
//! // List all backups
//! // let backups = scheduler.list_backups().unwrap();
//! ```

use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// Information about a stored backup.
#[derive(Debug, Clone, Serialize)]
pub struct BackupInfo {
    /// Unique backup identifier (timestamp-based).
    pub id: String,
    /// Full path to the backup directory.
    pub path: PathBuf,
    /// Size of the backup in bytes.
    pub size_bytes: u64,
    /// Number of files in the backup.
    pub file_count: usize,
    /// ISO-8601 timestamp of when the backup was created.
    pub created_at: String,
}

/// Configuration for the backup scheduler.
#[derive(Debug, Clone)]
pub struct BackupConfig {
    /// Number of most recent backups to retain (oldest are pruned).
    pub retention_count: usize,
    /// Backup directory path.
    pub backup_dir: PathBuf,
}

impl Default for BackupConfig {
    fn default() -> Self {
        Self {
            retention_count: 10,
            backup_dir: PathBuf::from("backups"),
        }
    }
}

/// Type alias for snapshot and list functions wrapped in Arc.
pub type SnapshotFn = Arc<dyn Fn(&Path) -> crate::infra::error::Result<()> + Send + Sync>;
pub type ListFn = Arc<
    dyn Fn(&Path) -> crate::infra::error::Result<Vec<crate::core::engine::SnapshotInfo>>
        + Send
        + Sync,
>;

/// Manages periodic backups of the LSM engine.
pub struct BackupScheduler {
    /// Configuration.
    config: Mutex<BackupConfig>,
    /// Whether the scheduler is running.
    running: AtomicBool,
    /// Handle to the background scheduler thread.
    thread_handle: Mutex<Option<JoinHandle<()>>>,
    /// Snapshot function: given a path, creates a snapshot there.
    snapshot_fn: SnapshotFn,
    /// List snapshots function.
    list_fn: ListFn,
}

impl BackupScheduler {
    /// Create a new `BackupScheduler`.
    ///
    /// * `snapshot_fn` — closure that calls `engine.create_snapshot(path)`
    /// * `list_fn` — closure that calls `engine.list_snapshots(path)`
    /// * `backup_dir` — directory where backups are stored
    pub fn new(snapshot_fn: SnapshotFn, list_fn: ListFn, backup_dir: PathBuf) -> Self {
        Self {
            config: Mutex::new(BackupConfig {
                backup_dir,
                ..BackupConfig::default()
            }),
            running: AtomicBool::new(false),
            thread_handle: Mutex::new(None),
            snapshot_fn,
            list_fn,
        }
    }

    /// Start periodic backups.
    ///
    /// Spawns a background thread that creates a snapshot every `interval`.
    pub fn schedule(&self, interval: Duration) {
        if self.running.swap(true, Ordering::SeqCst) {
            tracing::warn!("Backup scheduler is already running");
            return;
        }

        let snapshot_fn = self.snapshot_fn.clone();
        let list_fn = self.list_fn.clone();
        let config = Arc::new(Mutex::new(self.config.lock().clone()));
        let running_flag = Arc::new(AtomicBool::new(true));

        let handle = thread::Builder::new()
            .name("backup-scheduler".to_string())
            .spawn(move || {
                while running_flag.load(Ordering::SeqCst) {
                    thread::sleep(interval);

                    let cfg = config.lock();
                    let backup_dir = cfg.backup_dir.clone();
                    let retention = cfg.retention_count;
                    drop(cfg);

                    // Create timestamp-based backup directory
                    let timestamp = Utc::now().format("%Y%m%d_%H%M%S_%3f").to_string();
                    let backup_path = backup_dir.join(&timestamp);

                    if let Err(e) = std::fs::create_dir_all(&backup_path) {
                        tracing::error!("Backup scheduler: failed to create backup dir: {}", e);
                        continue;
                    }

                    // Create snapshot into backup directory
                    if let Err(e) = (snapshot_fn)(&backup_path) {
                        tracing::error!("Backup scheduler: snapshot failed: {}", e);
                        continue;
                    }

                    tracing::info!(
                        "Backup scheduler: created backup at {}",
                        backup_path.display()
                    );

                    // Enforce retention: remove oldest backups
                    if let Ok(backups) = (list_fn)(&backup_dir) {
                        if backups.len() > retention {
                            let to_remove = backups.len() - retention;
                            for backup in backups.iter().rev().take(to_remove) {
                                let _ = std::fs::remove_dir_all(&backup.path);
                                tracing::info!(
                                    "Backup scheduler: pruned old backup at {}",
                                    backup.path.display()
                                );
                            }
                        }
                    }
                }
            })
            .expect("Failed to spawn backup scheduler thread");

        *self.thread_handle.lock() = Some(handle);
    }

    /// Trigger an immediate backup.
    ///
    /// Creates a snapshot in a timestamped subdirectory under the configured
    /// backup directory.
    pub fn backup_now(&self) -> crate::infra::error::Result<BackupInfo> {
        let cfg = self.config.lock();
        let backup_dir = cfg.backup_dir.clone();
        let retention = cfg.retention_count;
        drop(cfg);

        std::fs::create_dir_all(&backup_dir)?;

        let timestamp = Utc::now().format("%Y%m%d_%H%M%S_%3f").to_string();
        let backup_path = backup_dir.join(&timestamp);

        (self.snapshot_fn)(&backup_path)?;

        // Compute size and file count
        let size_bytes = dir_size(&backup_path);
        let file_count = file_count_dir(&backup_path);

        let info = BackupInfo {
            id: timestamp.clone(),
            path: backup_path,
            size_bytes,
            file_count,
            created_at: Utc::now().to_rfc3339(),
        };

        // Enforce retention
        self.enforce_retention(&backup_dir, retention)?;

        Ok(info)
    }

    /// List all available backups.
    pub fn list_backups(&self) -> crate::infra::error::Result<Vec<BackupInfo>> {
        let cfg = self.config.lock();
        let backup_dir = cfg.backup_dir.clone();
        drop(cfg);

        let snapshots = (self.list_fn)(&backup_dir)?;

        let mut backups = Vec::new();
        for snap in snapshots {
            let id = snap
                .path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            backups.push(BackupInfo {
                id,
                path: snap.path,
                size_bytes: snap.size_bytes,
                file_count: snap.file_count,
                created_at: datetime_from_system_time(snap.created_at),
            });
        }

        Ok(backups)
    }

    /// Restore from a backup by ID.
    ///
    /// # Arguments
    ///
    /// * `backup_id` — the timestamp-based ID (e.g., "20250101_120000")
    /// * `restore_fn` — closure that calls `engine.restore_snapshot(path)`
    pub fn restore(
        &self,
        backup_id: &str,
        restore_fn: &dyn Fn(&Path) -> crate::infra::error::Result<()>,
    ) -> crate::infra::error::Result<()> {
        let cfg = self.config.lock();
        let backup_path = cfg.backup_dir.join(backup_id);
        drop(cfg);

        if !backup_path.exists() {
            return Err(crate::infra::error::LsmError::InvalidArgument(format!(
                "Backup not found: {}",
                backup_id
            )));
        }

        restore_fn(&backup_path)
    }

    /// Stop the background scheduler thread.
    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
        if let Some(handle) = self.thread_handle.lock().take() {
            handle.thread().unpark();
        }
    }

    /// Update backup configuration.
    pub fn set_config(&self, config: BackupConfig) {
        *self.config.lock() = config;
    }

    /// Get the current backup configuration.
    pub fn config(&self) -> BackupConfig {
        self.config.lock().clone()
    }

    /// Enforce retention policy: remove oldest backups exceeding the limit.
    fn enforce_retention(
        &self,
        backup_dir: &Path,
        retention: usize,
    ) -> crate::infra::error::Result<()> {
        let snapshots = (self.list_fn)(backup_dir)?;
        if snapshots.len() > retention {
            let to_remove = snapshots.len() - retention;
            for snap in snapshots.iter().rev().take(to_remove) {
                let _ = std::fs::remove_dir_all(&snap.path);
                tracing::info!(
                    "Backup scheduler: pruned old backup at {}",
                    snap.path.display()
                );
            }
        }
        Ok(())
    }
}

/// Compute total size of a directory recursively.
fn dir_size(dir: &Path) -> u64 {
    let mut total = 0u64;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                total += dir_size(&path);
            } else if let Ok(meta) = path.metadata() {
                total += meta.len();
            }
        }
    }
    total
}

/// Count files in a directory recursively.
fn file_count_dir(dir: &Path) -> usize {
    let mut count = 0;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                count += file_count_dir(&path);
            } else {
                count += 1;
            }
        }
    }
    count
}

/// Convert `SystemTime` to ISO-8601 string.
fn datetime_from_system_time(t: std::time::SystemTime) -> String {
    let dt: DateTime<Utc> = t.into();
    dt.to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backup_now_and_list() {
        let dir = tempfile::tempdir().unwrap();
        let backup_dir = dir.path().join("backups");

        let snapshot_fn = Arc::new(|path: &Path| {
            std::fs::create_dir_all(path)?;
            std::fs::write(path.join("wal.log"), b"")?;
            std::fs::write(path.join("snapshot.manifest"), b"{}")?;
            Ok(())
        }) as SnapshotFn;

        let list_fn = Arc::new(move |path: &Path| {
            let mut snapshots = Vec::new();
            if let Ok(entries) = std::fs::read_dir(path) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    if p.is_dir() && p.join("wal.log").exists() {
                        snapshots.push(crate::core::engine::SnapshotInfo {
                            path: p,
                            created_at: std::time::SystemTime::now(),
                            size_bytes: 0,
                            file_count: 0,
                        });
                    }
                }
            }
            snapshots.sort_by_key(|b| std::cmp::Reverse(b.created_at));
            Ok(snapshots)
        }) as ListFn;

        let scheduler = BackupScheduler::new(snapshot_fn, list_fn, backup_dir.clone());
        let info = scheduler.backup_now().unwrap();
        assert!(!info.id.is_empty());
        assert!(info.path.exists());

        let backups = scheduler.list_backups().unwrap();
        assert_eq!(backups.len(), 1);
        assert_eq!(backups[0].id, info.id);
    }

    #[test]
    fn test_retention() {
        let dir = tempfile::tempdir().unwrap();
        let backup_dir = dir.path().join("backups");

        let snapshot_fn = Arc::new(|path: &Path| {
            std::fs::create_dir_all(path)?;
            std::fs::write(path.join("wal.log"), b"")?;
            std::fs::write(path.join("snapshot.manifest"), b"{}")?;
            Ok(())
        }) as SnapshotFn;

        let list_fn = Arc::new(move |path: &Path| {
            let mut snapshots = Vec::new();
            if let Ok(entries) = std::fs::read_dir(path) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    if p.is_dir() && p.join("wal.log").exists() {
                        snapshots.push(crate::core::engine::SnapshotInfo {
                            path: p,
                            created_at: std::time::SystemTime::now(),
                            size_bytes: 0,
                            file_count: 0,
                        });
                    }
                }
            }
            snapshots.sort_by_key(|b| std::cmp::Reverse(b.created_at));
            Ok(snapshots)
        }) as ListFn;

        let scheduler = BackupScheduler::new(snapshot_fn, list_fn, backup_dir.clone());
        scheduler.set_config(BackupConfig {
            retention_count: 2,
            backup_dir: backup_dir.clone(),
        });

        // Create 3 backups
        scheduler.backup_now().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(10));
        scheduler.backup_now().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(10));
        scheduler.backup_now().unwrap();

        let backups = scheduler.list_backups().unwrap();
        assert_eq!(backups.len(), 2); // retention=2, oldest should be removed
    }

    #[test]
    fn test_restore_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let backup_dir = dir.path().join("backups");

        let snapshot_fn = Arc::new(|_: &Path| Ok(())) as SnapshotFn;
        let list_fn = Arc::new(|_: &Path| Ok(Vec::new())) as ListFn;

        let scheduler = BackupScheduler::new(snapshot_fn, list_fn, backup_dir);
        let restore_fn = |_: &Path| -> crate::infra::error::Result<()> { Ok(()) };
        let result = scheduler.restore("nonexistent", &restore_fn);
        assert!(result.is_err());
    }
}
