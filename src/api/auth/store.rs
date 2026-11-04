//! Private credential persistence. Public LSM records are never a trust source.

use super::{ApiToken, AuthError};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    version: u32,
    tokens: Vec<ApiToken>,
}

pub(super) struct TokenStore {
    directory: PathBuf,
    // Hold the exclusive lock across reads and atomic replacements.
    _lock: File,
}

fn storage_error(error: impl std::fmt::Display) -> AuthError {
    AuthError::Internal(format!("Private token storage: {error}"))
}

impl TokenStore {
    pub fn open(directory: PathBuf) -> Result<Self, AuthError> {
        std::fs::create_dir_all(&directory).map_err(storage_error)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))
                .map_err(storage_error)?;
        }
        let mut options = OpenOptions::new();
        options.create(true).truncate(false).read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let lock = options
            .open(directory.join("LOCK"))
            .map_err(storage_error)?;
        lock.try_lock_exclusive().map_err(storage_error)?;
        Ok(Self {
            directory,
            _lock: lock,
        })
    }

    pub fn load(&self) -> Result<HashMap<String, ApiToken>, AuthError> {
        let bytes = match std::fs::read(self.directory.join("tokens.json")) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(HashMap::new()),
            Err(error) => return Err(storage_error(error)),
        };
        let snapshot: Snapshot = serde_json::from_slice(&bytes).map_err(storage_error)?;
        if snapshot.version != 1 {
            return Err(storage_error("unsupported snapshot version"));
        }
        let mut tokens = HashMap::new();
        for token in snapshot.tokens {
            if uuid::Uuid::parse_str(&token.id).is_err()
                || token.token_hash.len() != 64
                || !token
                    .token_hash
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit())
            {
                return Err(storage_error("invalid token record"));
            }
            if tokens.insert(token.id.clone(), token).is_some() {
                return Err(storage_error("duplicate token identifier"));
            }
        }
        Ok(tokens)
    }

    pub fn save(&self, tokens: &HashMap<String, ApiToken>) -> Result<(), AuthError> {
        let mut records: Vec<_> = tokens.values().cloned().collect();
        records.sort_by(|left, right| left.id.cmp(&right.id));
        let snapshot = Snapshot {
            version: 1,
            tokens: records,
        };
        // Same-directory replacement is atomic; sync the contents before rename
        // and the directory afterwards so an acknowledged update is durable.
        let mut pending =
            tempfile::NamedTempFile::new_in(&self.directory).map_err(storage_error)?;
        serde_json::to_writer(&mut pending, &snapshot).map_err(storage_error)?;
        pending.flush().map_err(storage_error)?;
        pending.as_file().sync_all().map_err(storage_error)?;
        pending
            .persist(self.directory.join("tokens.json"))
            .map_err(storage_error)?;
        #[cfg(unix)]
        File::open(&self.directory)
            .and_then(|directory| directory.sync_all())
            .map_err(storage_error)?;
        Ok(())
    }
}
