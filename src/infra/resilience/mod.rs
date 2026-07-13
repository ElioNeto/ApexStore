//! Resilience infrastructure — fault tolerance and degradation management.
//!
//! This module consolidates three subsystems that work together to keep the
//! engine available under adverse conditions:
//!
//! - **`backpressure`** — adaptive EMA-based backpressure controller that
//!   dynamically adjusts write acceptance based on compaction throughput.
//! - **`degradation`** — gradual service degradation modes (Normal, Degraded,
//!   ReadOnly) that gate write operations when resources are constrained.
//! - **`circuit_breaker`** — closed/open/half-open state machine that prevents
//!   cascading failures when downstream operations fail repeatedly.

pub mod backpressure;
pub mod circuit_breaker;
pub mod degradation;
