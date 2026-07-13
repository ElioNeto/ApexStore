//! Graceful degradation modes for ApexStore.
//!
//! Allows the system to operate in reduced-capacity modes when resources are
//! constrained (e.g. disk full, memory pressure, high error rates).
//!
//! # Modes
//!
//! * **Normal** — full read/write capability.
//! * **ReadOnly** — only reads are allowed; writes return an error.
//! * **Degraded** — reads allowed, writes are best-effort but may fail.

use std::sync::RwLock;

/// Operational modes for graceful degradation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DegradationMode {
    /// Full read/write capability.
    Normal,
    /// Only reads allowed. Writes are rejected.
    ReadOnly,
    /// Reduced capacity. Reads allowed, writes are best-effort.
    Degraded,
}

impl std::fmt::Display for DegradationMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DegradationMode::Normal => write!(f, "normal"),
            DegradationMode::ReadOnly => write!(f, "read_only"),
            DegradationMode::Degraded => write!(f, "degraded"),
        }
    }
}

/// Manages the current degradation mode and enforces write restrictions.
pub struct DegradationManager {
    mode: RwLock<DegradationMode>,
}

impl DegradationManager {
    /// Create a new manager in the given initial mode.
    pub fn new(mode: DegradationMode) -> Self {
        Self {
            mode: RwLock::new(mode),
        }
    }

    /// Create a new manager in Normal mode.
    pub fn normal() -> Self {
        Self::new(DegradationMode::Normal)
    }

    /// Set the current degradation mode.
    pub fn set_mode(&self, mode: DegradationMode) {
        let mut current = self.mode.write().unwrap();
        *current = mode;
    }

    /// Returns the current degradation mode.
    pub fn current_mode(&self) -> DegradationMode {
        let current = self.mode.read().unwrap();
        *current
    }

    /// Returns `true` if the engine is in read-only mode.
    pub fn is_read_only(&self) -> bool {
        let current = self.mode.read().unwrap();
        *current == DegradationMode::ReadOnly
    }

    /// Returns `true` if the engine is in degraded mode.
    pub fn is_degraded(&self) -> bool {
        let current = self.mode.read().unwrap();
        *current == DegradationMode::Degraded
    }

    /// Attempt to check whether a write operation is allowed.
    ///
    /// Returns `Ok(())` if writes are allowed, or an error string explaining
    /// why the write was rejected.
    pub fn check_write_allowed(&self) -> Result<(), String> {
        let current = self.mode.read().unwrap();
        match *current {
            DegradationMode::Normal | DegradationMode::Degraded => Ok(()),
            DegradationMode::ReadOnly => {
                Err("engine is in read-only mode; writes are rejected".to_string())
            }
        }
    }
}

impl Default for DegradationManager {
    fn default() -> Self {
        Self::normal()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_is_normal() {
        let mgr = DegradationManager::normal();
        assert_eq!(mgr.current_mode(), DegradationMode::Normal);
        assert!(!mgr.is_read_only());
        assert!(!mgr.is_degraded());
    }

    #[test]
    fn test_set_mode() {
        let mgr = DegradationManager::normal();
        mgr.set_mode(DegradationMode::ReadOnly);
        assert_eq!(mgr.current_mode(), DegradationMode::ReadOnly);
        assert!(mgr.is_read_only());
        assert!(!mgr.is_degraded());

        mgr.set_mode(DegradationMode::Degraded);
        assert!(mgr.is_degraded());
        assert!(!mgr.is_read_only());

        mgr.set_mode(DegradationMode::Normal);
        assert!(!mgr.is_read_only());
        assert!(!mgr.is_degraded());
    }

    #[test]
    fn test_write_allowed_in_normal() {
        let mgr = DegradationManager::normal();
        assert!(mgr.check_write_allowed().is_ok());
    }

    #[test]
    fn test_write_allowed_in_degraded() {
        let mgr = DegradationManager::new(DegradationMode::Degraded);
        assert!(mgr.check_write_allowed().is_ok());
    }

    #[test]
    fn test_write_rejected_in_read_only() {
        let mgr = DegradationManager::new(DegradationMode::ReadOnly);
        let result = mgr.check_write_allowed();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("read-only"));
    }
}
