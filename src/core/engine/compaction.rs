use crate::core::engine::EngineOptions;
use crate::core::table::Table;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default)]
pub struct Compaction {
    pub max_tables_per_compaction: usize,
    pub compaction_threshold: usize,
}

impl Compaction {
    pub fn merge_tables(tables: Vec<Table>, options: &EngineOptions) -> Option<Table> {
        if tables.is_empty() {
            return None;
        }

        let mut merged_data: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();

        for table in tables {
            for (key, value) in table.data.iter() {
                // Last write wins
                merged_data.insert(key.clone(), value.clone());
            }
        }

        if merged_data.is_empty() {
            return None;
        }

        Some(Table::build(merged_data.into_iter().collect(), options))
    }
}
