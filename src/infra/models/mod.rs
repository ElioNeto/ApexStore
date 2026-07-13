//! Data model abstractions — multi-model query support and tiering.
//!
//! This module consolidates two subsystems that extend the engine's data model:
//!
//! - **`data_tiering`** — hot/warm/cold tier tracking with auto-promotion,
//!   age-out, and compaction hints for storage-class-aware data management.
//! - **`multi_model`** — dispatcher for document, time-series, and graph
//!   query models on top of the core key-value engine.

pub mod data_tiering;
pub mod multi_model;
