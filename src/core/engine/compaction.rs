#[derive(Debug, Clone, Default)]
pub struct Compaction {
    pub max_tables_per_compaction: usize,
    pub compaction_threshold: usize,
}
