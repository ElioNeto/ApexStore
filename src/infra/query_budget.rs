//! Budget-aware queries — track cost per query and enforce limits.
//!
//! This module provides:
//!
//! - [`QueryBudget`] — tracks resource consumption during query execution,
//!   including key reads and bytes scanned.
//! - [`BudgetExhausted`] — an error type returned when budget is exhausted.

use std::error::Error;
use std::fmt;

/// Error returned when a query has exhausted its allocated budget.
#[derive(Debug, Clone)]
pub struct BudgetExhausted {
    /// The kind of resource that was exhausted.
    pub resource: &'static str,
    /// How much was requested.
    pub requested: u64,
    /// How much was remaining.
    pub remaining: u64,
}

impl fmt::Display for BudgetExhausted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "query budget exhausted: {} — requested {}, remaining {}",
            self.resource, self.requested, self.remaining
        )
    }
}

impl Error for BudgetExhausted {}

/// Tracks the execution budget for a single query.
///
/// A budget can be set for key reads and bytes scanned. When either limit is
/// reached, further operations are denied with [`BudgetExhausted`].
///
/// # Example
///
/// ```ignore
/// let mut budget = QueryBudget::with_budget(100, 10_000);
/// budget.spend_key_read()?;          // costs 1 key read
/// budget.spend_bytes_scanned(256)?;  // costs 256 bytes
/// ```
#[derive(Debug, Clone)]
pub struct QueryBudget {
    max_key_reads: u64,
    max_bytes_scanned: u64,
    key_reads_used: u64,
    bytes_scanned_used: u64,
}

impl QueryBudget {
    /// Create a new budget with no limits (unbounded).
    pub fn unlimited() -> Self {
        Self {
            max_key_reads: u64::MAX,
            max_bytes_scanned: u64::MAX,
            key_reads_used: 0,
            bytes_scanned_used: 0,
        }
    }

    /// Create a new budget with the given limits.
    ///
    /// * `max_key_reads` — maximum number of key-value lookups allowed.
    /// * `max_bytes_scanned` — maximum number of bytes that can be scanned.
    pub fn with_budget(max_key_reads: u64, max_bytes_scanned: u64) -> Self {
        Self {
            max_key_reads,
            max_bytes_scanned,
            key_reads_used: 0,
            bytes_scanned_used: 0,
        }
    }

    /// Spend one key read from the budget.
    ///
    /// Returns `Err(BudgetExhausted)` if the key-read limit has been reached.
    pub fn spend_key_read(&mut self) -> Result<(), BudgetExhausted> {
        if self.key_reads_used >= self.max_key_reads {
            return Err(BudgetExhausted {
                resource: "key_reads",
                requested: 1,
                remaining: self.remaining_key_reads(),
            });
        }
        self.key_reads_used += 1;
        Ok(())
    }

    /// Spend the given number of bytes scanned.
    ///
    /// Returns `Err(BudgetExhausted)` if the byte-scan limit would be exceeded.
    pub fn spend_bytes_scanned(&mut self, bytes: u64) -> Result<(), BudgetExhausted> {
        let new_total = self.bytes_scanned_used.saturating_add(bytes);
        if new_total > self.max_bytes_scanned {
            return Err(BudgetExhausted {
                resource: "bytes_scanned",
                requested: bytes,
                remaining: self.remaining_bytes_scanned(),
            });
        }
        self.bytes_scanned_used = new_total;
        Ok(())
    }

    /// Spend an arbitrary `cost` value (generic cost unit).
    ///
    /// If the remaining budget is less than `cost`, returns an error. This is
    /// useful for integrating custom cost models.
    pub fn spend(&mut self, cost: u64) -> Result<(), BudgetExhausted> {
        // Delegate to key-read spending as a simple heuristic.
        if self.remaining() < cost {
            return Err(BudgetExhausted {
                resource: "generic_cost",
                requested: cost,
                remaining: self.remaining(),
            });
        }
        self.key_reads_used = self.key_reads_used.saturating_add(cost);
        Ok(())
    }

    /// Return the remaining budget (in generic cost units).
    ///
    /// Uses `max_key_reads - key_reads_used` as the primary metric.
    pub fn remaining(&self) -> u64 {
        self.max_key_reads.saturating_sub(self.key_reads_used)
    }

    /// Return the remaining key-read budget.
    pub fn remaining_key_reads(&self) -> u64 {
        self.max_key_reads.saturating_sub(self.key_reads_used)
    }

    /// Return the remaining byte-scan budget.
    pub fn remaining_bytes_scanned(&self) -> u64 {
        self.max_bytes_scanned
            .saturating_sub(self.bytes_scanned_used)
    }

    /// Return `true` if the budget is fully exhausted (no key reads left).
    pub fn is_exhausted(&self) -> bool {
        self.key_reads_used >= self.max_key_reads
    }

    /// Reset all counters back to zero.
    pub fn reset(&mut self) {
        self.key_reads_used = 0;
        self.bytes_scanned_used = 0;
    }
}

impl Default for QueryBudget {
    fn default() -> Self {
        Self::unlimited()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unlimited_budget() {
        let mut budget = QueryBudget::unlimited();
        assert!(!budget.is_exhausted());
        assert_eq!(budget.remaining(), u64::MAX);
        assert!(budget.spend_key_read().is_ok());
        assert!(budget.spend_key_read().is_ok());
        assert!(!budget.is_exhausted());
    }

    #[test]
    fn test_limited_budget_exhausted() {
        let mut budget = QueryBudget::with_budget(3, 100);
        assert!(budget.spend_key_read().is_ok());
        assert!(budget.spend_key_read().is_ok());
        assert!(budget.spend_key_read().is_ok());
        assert!(budget.is_exhausted());
        let err = budget.spend_key_read().unwrap_err();
        assert_eq!(err.resource, "key_reads");
    }

    #[test]
    fn test_bytes_scanned_exhaustion() {
        let mut budget = QueryBudget::with_budget(10, 100);
        assert!(budget.spend_bytes_scanned(60).is_ok());
        assert!(budget.spend_bytes_scanned(40).is_ok());
        // Next spend should fail.
        let err = budget.spend_bytes_scanned(1).unwrap_err();
        assert_eq!(err.resource, "bytes_scanned");
    }

    #[test]
    fn test_remaining() {
        let mut budget = QueryBudget::with_budget(10, 500);
        assert_eq!(budget.remaining(), 10);
        budget.spend_key_read().unwrap();
        assert_eq!(budget.remaining(), 9);
    }

    #[test]
    fn test_spend_generic() {
        let mut budget = QueryBudget::with_budget(5, 100);
        assert!(budget.spend(3).is_ok());
        assert_eq!(budget.remaining(), 2);
        let err = budget.spend(3).unwrap_err();
        assert_eq!(err.resource, "generic_cost");
        assert_eq!(err.requested, 3);
        assert_eq!(err.remaining, 2);
    }

    #[test]
    fn test_reset() {
        let mut budget = QueryBudget::with_budget(2, 50);
        budget.spend_key_read().unwrap();
        budget.spend_bytes_scanned(30).unwrap();
        assert_eq!(budget.remaining_key_reads(), 1);
        assert_eq!(budget.remaining_bytes_scanned(), 20);
        budget.reset();
        assert_eq!(budget.remaining_key_reads(), 2);
        assert_eq!(budget.remaining_bytes_scanned(), 50);
    }
}
