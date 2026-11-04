//! Bounded, expiring REST transactions owned by the authenticated token.

use super::{auth, config::AuthEnabled, require_permission, Permission};
use crate::core::engine::transaction::Transaction;
use crate::storage::cache::GlobalBlockCache;
use crate::LsmEngine;
use actix_web::{http::StatusCode, web, HttpRequest, HttpResponse, ResponseError};
use parking_lot::Mutex;
use rand::{rngs::SysRng, TryRng};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

type EngineTransaction = Transaction<Arc<GlobalBlockCache>>;

#[derive(Clone, PartialEq, Eq)]
pub(super) enum Owner {
    Token(String),
    Anonymous,
}

pub(super) fn owner(req: &HttpRequest, permission: Permission) -> Result<Owner, Box<HttpResponse>> {
    require_permission(req, permission)?;
    if req
        .app_data::<web::Data<AuthEnabled>>()
        .is_some_and(|flag| !flag.get_ref().0)
    {
        return Ok(Owner::Anonymous);
    }
    auth::middleware::extract_token(req)
        .map(|token| Owner::Token(token.id))
        .ok_or_else(|| Box::new(HttpResponse::Unauthorized().finish()))
}

struct ActiveTransaction {
    owner: Owner,
    created_at: Instant,
    transaction: EngineTransaction,
}

#[derive(Debug, thiserror::Error)]
pub(super) enum TransactionError {
    #[error("transaction not found")]
    NotFound,
    #[error("too many open transactions")]
    Capacity,
    #[error("secure transaction identifier unavailable")]
    Identifier,
}

impl ResponseError for TransactionError {
    fn status_code(&self) -> StatusCode {
        match self {
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::Capacity => StatusCode::TOO_MANY_REQUESTS,
            Self::Identifier => StatusCode::SERVICE_UNAVAILABLE,
        }
    }

    fn error_response(&self) -> HttpResponse {
        HttpResponse::build(self.status_code()).json(json!({"error": self.to_string()}))
    }
}

/// REST transactions expire five minutes after creation, with at most 32 per
/// token and 1024 overall. Access and insertion remove expired entries; touching
/// a transaction never extends its lifetime. With authentication explicitly
/// disabled, every request belongs to one shared anonymous owner.
///
/// Staging and authorization share a lock so concurrent requests cannot steal,
/// lose or resurrect a transaction. Commit/rollback remove it atomically before
/// doing storage work. Expired/abandoned transactions are dropped, never committed.
pub struct TransactionManager {
    transactions: Mutex<HashMap<Uuid, ActiveTransaction>>,
    lifetime: Duration,
    max_per_owner: usize,
    max_total: usize,
}

impl Default for TransactionManager {
    fn default() -> Self {
        Self::new()
    }
}

impl TransactionManager {
    pub fn new() -> Self {
        Self::with_limits(Duration::from_secs(300), 32, 1024)
    }

    /// Configure limits for an embedded HTTP application. A zero limit or zero
    /// lifetime rejects new transactions. The server uses the bounded defaults.
    pub fn with_limits(lifetime: Duration, max_per_owner: usize, max_total: usize) -> Self {
        Self {
            transactions: Mutex::new(HashMap::new()),
            lifetime,
            max_per_owner,
            max_total,
        }
    }

    fn prune(&self, entries: &mut HashMap<Uuid, ActiveTransaction>, now: Instant) {
        entries.retain(|_, entry| now.duration_since(entry.created_at) < self.lifetime);
    }

    pub(super) fn begin(&self, engine: &LsmEngine, owner: Owner) -> Result<Uuid, TransactionError> {
        let mut entries = self.transactions.lock();
        let now = Instant::now();
        self.prune(&mut entries, now);
        if self.lifetime.is_zero()
            || entries.len() >= self.max_total
            || entries
                .values()
                .filter(|entry| entry.owner == owner)
                .count()
                >= self.max_per_owner
        {
            return Err(TransactionError::Capacity);
        }
        // Generate a UUID v4 with fallible OS entropy. Never use a counter or
        // overwrite an existing entry, even in the unlikely event of a collision.
        for _ in 0..8 {
            let mut bytes = [0; 16];
            SysRng
                .try_fill_bytes(&mut bytes)
                .map_err(|_| TransactionError::Identifier)?;
            let id = uuid::Builder::from_random_bytes(bytes).into_uuid();
            if let std::collections::hash_map::Entry::Vacant(slot) = entries.entry(id) {
                slot.insert(ActiveTransaction {
                    owner,
                    created_at: now,
                    transaction: engine.begin_transaction(),
                });
                return Ok(id);
            }
        }
        Err(TransactionError::Identifier)
    }

    pub(super) fn with_transaction<T>(
        &self,
        id: &str,
        owner: &Owner,
        change: impl FnOnce(&mut EngineTransaction) -> T,
    ) -> Result<T, TransactionError> {
        let mut entries = self.transactions.lock();
        self.prune(&mut entries, Instant::now());
        let id = Uuid::parse_str(id).map_err(|_| TransactionError::NotFound)?;
        let entry = entries
            .get_mut(&id)
            .filter(|entry| &entry.owner == owner)
            .ok_or(TransactionError::NotFound)?;
        Ok(change(&mut entry.transaction))
    }

    pub(super) fn take(
        &self,
        id: &str,
        owner: &Owner,
    ) -> Result<EngineTransaction, TransactionError> {
        let mut entries = self.transactions.lock();
        self.prune(&mut entries, Instant::now());
        let id = Uuid::parse_str(id).map_err(|_| TransactionError::NotFound)?;
        if !entries.get(&id).is_some_and(|entry| &entry.owner == owner) {
            return Err(TransactionError::NotFound);
        }
        entries
            .remove(&id)
            .map(|entry| entry.transaction)
            .ok_or(TransactionError::NotFound)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::test_helpers::test_engine;

    #[test]
    fn concurrent_begins_cannot_exceed_owner_capacity() {
        let (engine, _directory) = test_engine();
        let engine = Arc::new(engine);
        let manager = Arc::new(TransactionManager::with_limits(
            Duration::from_secs(300),
            2,
            8,
        ));
        let gate = Arc::new(std::sync::Barrier::new(16));
        let handles: Vec<_> = (0..16)
            .map(|_| {
                let (engine, manager, gate) = (engine.clone(), manager.clone(), gate.clone());
                std::thread::spawn(move || {
                    gate.wait();
                    manager.begin(&engine, Owner::Token("same-owner".into()))
                })
            })
            .collect();
        let results: Vec<_> = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect();
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 2);
        assert!(results
            .iter()
            .filter_map(|result| result.as_ref().err())
            .all(|error| matches!(error, TransactionError::Capacity)));
        assert_eq!(manager.transactions.lock().len(), 2);
    }

    #[test]
    fn concurrent_staging_keeps_every_write() {
        let (engine, _directory) = test_engine();
        let manager = Arc::new(TransactionManager::new());
        let owner = Owner::Token("owner".into());
        let id = manager.begin(&engine, owner.clone()).unwrap().to_string();
        let gate = Arc::new(std::sync::Barrier::new(16));
        let handles: Vec<_> = (0..16)
            .map(|n| {
                let (manager, owner, id, gate) =
                    (manager.clone(), owner.clone(), id.clone(), gate.clone());
                std::thread::spawn(move || {
                    gate.wait();
                    manager
                        .with_transaction(&id, &owner, |txn| txn.put(format!("key-{n}"), b"value"))
                        .unwrap()
                        .unwrap();
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
        manager.take(&id, &owner).unwrap().commit().unwrap();
        for n in 0..16 {
            assert_eq!(
                engine.get(format!("key-{n}")).unwrap(),
                Some(b"value".to_vec())
            );
        }
        assert!(manager.transactions.lock().is_empty());
    }

    #[test]
    fn accesses_preserve_absolute_expiry_and_prune_other_owners() {
        let (engine, _directory) = test_engine();
        let manager = TransactionManager::new();
        let owner = Owner::Token("owner".into());
        let id = manager.begin(&engine, owner.clone()).unwrap();
        let created = manager.transactions.lock()[&id].created_at;
        manager
            .with_transaction(&id.to_string(), &owner, |txn| txn.put("expired", "value"))
            .unwrap()
            .unwrap();
        assert_eq!(manager.transactions.lock()[&id].created_at, created);
        // Simulate an old entry; expiry still uses the production monotonic clock.
        manager.transactions.lock().get_mut(&id).unwrap().created_at =
            Instant::now() - Duration::from_secs(301);
        assert!(matches!(
            manager.take(&id.to_string(), &Owner::Token("other".into())),
            Err(TransactionError::NotFound)
        ));
        assert!(manager.transactions.lock().is_empty());
        assert_eq!(engine.get("expired").unwrap(), None);
    }
}
