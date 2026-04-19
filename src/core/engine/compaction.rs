use crate::core::engine::EngineOptions;
use crate::core::table::Table;
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct Compaction {
    pub max_tables_per_compaction: usize,
    pub compaction_threshold: usize,
}

impl Default for Compaction {
    fn default() -> Self {
        Self {
            max_tables_per_compaction: 4,
            compaction_threshold: 8,
        }
    }
}

impl Compaction {
    pub fn merge_tables(tables: Vec<Table>, options: &EngineOptions) -> Option<Table> {
        let mut merged_data: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();
        // Itera do mais antigo (índice 0) ao mais novo (último)
        // para que escritas mais novas sobrescrevam as antigas
        for table in &tables {
            for (key, value) in table.data.iter() {
                if value.is_empty() {
                    // Tombstone: remover a chave do resultado
                    merged_data.remove(key);
                } else {
                    merged_data.insert(key.clone(), value.clone());
                }
            }
        }
        if merged_data.is_empty() {
            return None;
        }
        Some(Table::build(merged_data.into_iter().collect(), options))
    }
}
