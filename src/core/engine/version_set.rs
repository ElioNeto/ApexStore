use crate::storage::cache::Cache;

pub struct VersionSet<C: Cache> {
    _cache: std::marker::PhantomData<C>,
    tables: std::collections::HashMap<String, Vec<crate::core::table::Table>>,
}

impl<C: Cache> VersionSet<C> {
    pub fn new(_options: crate::core::engine::EngineOptions, _cache: C) -> Self {
        Self {
            _cache: std::marker::PhantomData,
            tables: std::collections::HashMap::new(),
        }
    }

    pub fn get(&self, cf: &str, key: &[u8]) -> Option<Vec<u8>> {
        if let Some(cf_tables) = self.tables.get(cf) {
            for table in cf_tables.iter().rev() {
                if let Some(val) = table.data.get(key) {
                    return Some(val.clone());
                }
            }
        }
        None
    }

    pub fn scan(
        &self,
        _cf: &str,
        _lower: Option<&[u8]>,
        _upper: Option<&[u8]>,
        _limit: Option<usize>,
    ) -> Vec<(Vec<u8>, Vec<u8>)> {
        Vec::new()
    }

    pub fn add_table(&mut self, cf: &str, table: crate::core::table::Table) {
        self.tables.entry(cf.to_string()).or_default().push(table);
    }

    pub fn current_version(&self) -> crate::core::version::Version<C> {
        crate::core::version::Version::new()
    }

    pub fn table_count(&self, cf: &str) -> usize {
        self.tables.get(cf).map_or(0, |v| v.len())
    }

    pub fn drain_tables(&mut self, cf: &str) -> Vec<crate::core::table::Table> {
        self.tables.remove(cf).unwrap_or_default()
    }

    pub fn remove_and_add_table(&mut self, cf: &str, new_table: crate::core::table::Table) {
        // Remove todas as tabelas da CF e substitui pela tabela compactada.
        let entry = self.tables.entry(cf.to_string()).or_default();
        entry.clear();
        entry.push(new_table);
    }
}
