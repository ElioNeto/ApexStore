//! Resource quotas per tenant.
//!
//! Tracks per-tenant resource usage (keys count, storage bytes, requests per second)
//! and enforces configurable limits. Useful for multi-tenant deployments where
//! resource isolation is required.
//!
//! # Usage
//!
//! ```rust
//! use apexstore::infra::quotas::{QuotaManager, TenantQuota};
//!
//! let qm = QuotaManager::new();
//!
//! // Set quota for a tenant
//! qm.set_quota("tenant-1", TenantQuota {
//!     max_keys: 1000,
//!     max_storage_bytes: 10_000_000,
//!     max_requests_per_second: 100,
//! });
//!
//! // Check before allowing an operation
//! qm.check_quota("tenant-1", 0, 1024).unwrap();
//!
//! // Record usage after an operation
//! qm.record_usage("tenant-1", 1, 1024);
//! ```

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Quota limits for a single tenant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TenantQuota {
    /// Maximum number of keys allowed for this tenant.
    pub max_keys: u64,
    /// Maximum storage bytes across all data for this tenant.
    pub max_storage_bytes: u64,
    /// Maximum requests per second (rate limiting).
    pub max_requests_per_second: u64,
}

impl Default for TenantQuota {
    fn default() -> Self {
        Self {
            max_keys: 10_000,
            max_storage_bytes: 100_000_000, // 100 MB
            max_requests_per_second: 1000,
        }
    }
}

/// Current usage for a single tenant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TenantUsage {
    pub tenant_id: String,
    pub keys_count: u64,
    pub storage_bytes: u64,
    /// Request rate tracking (sliding window) — stored as millis since epoch.
    #[serde(skip)]
    pub request_timestamps: Vec<Instant>,
}

impl TenantUsage {
    fn new(tenant_id: &str) -> Self {
        Self {
            tenant_id: tenant_id.to_string(),
            keys_count: 0,
            storage_bytes: 0,
            request_timestamps: Vec::new(),
        }
    }

    fn prune_requests(&mut self, window: Duration) {
        let now = Instant::now();
        self.request_timestamps
            .retain(|t| now.duration_since(*t) < window);
    }
}

/// Manages per-tenant resource quotas.
pub struct QuotaManager {
    quotas: Mutex<HashMap<String, TenantQuota>>,
    usage: Mutex<HashMap<String, TenantUsage>>,
    /// Default quota applied when no explicit quota is set for a tenant.
    default_quota: TenantQuota,
}

impl Default for QuotaManager {
    fn default() -> Self {
        Self {
            quotas: Mutex::new(HashMap::new()),
            usage: Mutex::new(HashMap::new()),
            default_quota: TenantQuota::default(),
        }
    }
}

impl QuotaManager {
    /// Create a new `QuotaManager`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a new `QuotaManager` with a custom default quota.
    pub fn with_default_quota(default_quota: TenantQuota) -> Self {
        Self {
            default_quota,
            ..Self::default()
        }
    }

    /// Check whether a tenant is allowed to perform an operation.
    ///
    /// Returns `Ok(())` if the operation is within quota, or an error message
    /// explaining which limit was exceeded.
    pub fn check_quota(
        &self,
        tenant_id: &str,
        additional_keys: u64,
        additional_bytes: u64,
    ) -> Result<(), String> {
        let quota = self
            .quotas
            .lock()
            .get(tenant_id)
            .cloned()
            .unwrap_or_else(|| self.default_quota.clone());

        let mut usage = self.usage.lock();
        let tenant_usage = usage
            .entry(tenant_id.to_string())
            .or_insert_with(|| TenantUsage::new(tenant_id));

        // Check keys count
        if tenant_usage.keys_count + additional_keys > quota.max_keys {
            return Err(format!(
                "Tenant '{}' key limit exceeded: {}/{}",
                tenant_id,
                tenant_usage.keys_count + additional_keys,
                quota.max_keys
            ));
        }

        // Check storage bytes
        if tenant_usage.storage_bytes + additional_bytes > quota.max_storage_bytes {
            return Err(format!(
                "Tenant '{}' storage limit exceeded: {}/{} bytes",
                tenant_id,
                tenant_usage.storage_bytes + additional_bytes,
                quota.max_storage_bytes
            ));
        }

        // Check request rate
        let window = Duration::from_secs(1);
        tenant_usage.prune_requests(window);
        if tenant_usage.request_timestamps.len() as u64 >= quota.max_requests_per_second {
            return Err(format!(
                "Tenant '{}' rate limit exceeded: {} req/s (max {})",
                tenant_id,
                tenant_usage.request_timestamps.len(),
                quota.max_requests_per_second
            ));
        }

        Ok(())
    }

    /// Record usage after an operation is performed.
    pub fn record_usage(&self, tenant_id: &str, keys_delta: i64, bytes_delta: i64) {
        let mut usage = self.usage.lock();
        let tenant_usage = usage
            .entry(tenant_id.to_string())
            .or_insert_with(|| TenantUsage::new(tenant_id));

        if keys_delta >= 0 {
            tenant_usage.keys_count = tenant_usage.keys_count.saturating_add(keys_delta as u64);
        } else {
            tenant_usage.keys_count = tenant_usage.keys_count.saturating_sub((-keys_delta) as u64);
        }

        if bytes_delta >= 0 {
            tenant_usage.storage_bytes = tenant_usage
                .storage_bytes
                .saturating_add(bytes_delta as u64);
        } else {
            tenant_usage.storage_bytes = tenant_usage
                .storage_bytes
                .saturating_sub((-bytes_delta) as u64);
        }

        tenant_usage.request_timestamps.push(Instant::now());
    }

    /// Set or update a tenant's quota.
    pub fn set_quota(&self, tenant_id: &str, quota: TenantQuota) {
        self.quotas.lock().insert(tenant_id.to_string(), quota);
    }

    /// Get the current quota for a tenant.
    pub fn get_quota(&self, tenant_id: &str) -> Option<TenantQuota> {
        self.quotas.lock().get(tenant_id).cloned()
    }

    /// Get current usage for a tenant.
    pub fn get_usage(&self, tenant_id: &str) -> Option<TenantUsage> {
        self.usage.lock().get(tenant_id).cloned()
    }

    /// Get all tenants with their current usage.
    pub fn all_usage(&self) -> Vec<TenantUsage> {
        self.usage.lock().values().cloned().collect()
    }

    /// Reset usage counters for a tenant.
    pub fn reset_usage(&self, tenant_id: &str) {
        self.usage
            .lock()
            .insert(tenant_id.to_string(), TenantUsage::new(tenant_id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_quota_ok() {
        let qm = QuotaManager::new();
        qm.set_quota(
            "tenant-a",
            TenantQuota {
                max_keys: 100,
                max_storage_bytes: 1_000_000,
                max_requests_per_second: 100,
            },
        );
        assert!(qm.check_quota("tenant-a", 1, 1024).is_ok());
    }

    #[test]
    fn test_check_quota_exceeds_keys() {
        let qm = QuotaManager::new();
        qm.set_quota(
            "tenant-b",
            TenantQuota {
                max_keys: 5,
                max_storage_bytes: 1_000_000,
                max_requests_per_second: 100,
            },
        );
        assert!(qm.check_quota("tenant-b", 10, 0).is_err());
    }

    #[test]
    fn test_check_quota_exceeds_storage() {
        let qm = QuotaManager::new();
        qm.set_quota(
            "tenant-c",
            TenantQuota {
                max_keys: 100,
                max_storage_bytes: 100, // very small
                max_requests_per_second: 100,
            },
        );
        assert!(qm.check_quota("tenant-c", 0, 200).is_err());
    }

    #[test]
    fn test_record_usage_updates_counters() {
        let qm = QuotaManager::new();
        qm.set_quota(
            "tenant-d",
            TenantQuota {
                max_keys: 1000,
                max_storage_bytes: 1_000_000,
                max_requests_per_second: 100,
            },
        );
        qm.record_usage("tenant-d", 5, 5000);
        let usage = qm.get_usage("tenant-d").unwrap();
        assert_eq!(usage.keys_count, 5);
        assert_eq!(usage.storage_bytes, 5000);
    }

    #[test]
    fn test_default_quota_applied() {
        let qm = QuotaManager::new();
        // No explicit quota set, should use default
        assert!(qm.check_quota("unknown-tenant", 1, 100).is_ok());
        qm.record_usage("unknown-tenant", 1, 100);
        let usage = qm.get_usage("unknown-tenant").unwrap();
        assert_eq!(usage.keys_count, 1);
    }

    #[test]
    fn test_all_usage() {
        let qm = QuotaManager::new();
        qm.record_usage("t1", 1, 100);
        qm.record_usage("t2", 2, 200);
        let all = qm.all_usage();
        assert_eq!(all.len(), 2);
    }
}
