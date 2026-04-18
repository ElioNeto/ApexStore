use crate::core::key::KeySlice;
use crate::core::iterators::StorageIterator;
use crate::core::table::Table;
use crate::core::version::Version;
use crate::core::engine::manifest::Manifest;
use crate::core::engine::version_set::VersionSet;
use crate::core::engine::compaction::Compaction;

pub const DEFAULT_SCAN_LIMIT: usize = 128;
pub const MAX_SCAN_LIMIT: usize = 1024;

pub struct LsmEngine;
