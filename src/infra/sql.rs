//! SQL query engine for ApexStore.
//!
//! Provides a `SqlEngine` wrapper around the LSM engine that accepts SQL-like
//! statements and maps them to engine operations:
//!
//! - `SELECT * FROM <cf>` → `scan_cf(cf, ...)`
//! - `SELECT * FROM <cf> WHERE key = '<k>'` → `get_cf(cf, k)`
//! - `INSERT INTO <cf> (key, value) VALUES ('k', 'v')` → `put_cf(cf, k, v)`
//! - `DELETE FROM <cf> WHERE key = '<k>'` → `delete_cf(cf, k)`

use crate::core::engine::Engine;
use crate::infra::error::Result;
use crate::storage::cache::Cache;
use sqlparser::ast::{
    Expr, FromTable, ObjectName, SetExpr, Statement as SqlStatement, TableFactor, TableWithJoins,
    Value,
};
use sqlparser::dialect::GenericDialect;
use sqlparser::parser::Parser;

/// SQL result types.
#[derive(Debug)]
pub enum SqlResult {
    /// Rows returned from a SELECT query.
    Rows {
        columns: Vec<String>,
        data: Vec<Vec<String>>,
    },
    /// Acknowledgment for INSERT/DELETE.
    Affected(u64),
}

/// A simple SQL engine that wraps a reference to the LSM key-value engine.
///
/// Supports basic SQL statements:
/// - `SELECT * FROM <cf>` — scan all keys in a column family
/// - `SELECT * FROM <cf> WHERE key = '<k>'` — get a specific key
/// - `INSERT INTO <cf> (key, value) VALUES ('k', 'v')` — insert or update
/// - `DELETE FROM <cf> WHERE key = '<k>'` — delete a key
pub struct SqlEngine<'a, C: Cache> {
    engine: &'a Engine<C>,
}

impl<'a, C: Cache> SqlEngine<'a, C> {
    /// Create a new SQL engine wrapping the given LSM engine reference.
    pub fn new(engine: &'a Engine<C>) -> Self {
        Self { engine }
    }

    /// Returns a reference to the underlying LSM engine.
    pub fn inner(&self) -> &Engine<C> {
        self.engine
    }

    /// Execute a SQL query string and return the result.
    pub fn execute(&self, sql: &str) -> Result<SqlResult> {
        let dialect = GenericDialect {};
        let statements = Parser::parse_sql(&dialect, sql).map_err(|e| {
            crate::infra::error::LsmError::InvalidArgument(format!("SQL error: {}", e))
        })?;

        if statements.is_empty() {
            return Err(crate::infra::error::LsmError::InvalidArgument(
                "Empty SQL statement".to_string(),
            ));
        }

        self.execute_statement(&statements[0])
    }

    /// Execute a parsed SQL statement.
    fn execute_statement(&self, stmt: &SqlStatement) -> Result<SqlResult> {
        match stmt {
            SqlStatement::Query(query) => {
                // Extract the body of the query (SELECT)
                match &*query.body {
                    SetExpr::Select(select) => {
                        let from = &select.from;
                        let selection = &select.selection;

                        // Determine column family from FROM clause
                        let cf = table_name_from_from_clause(from)
                            .unwrap_or_else(|| "default".to_string());

                        // Handle WHERE clause
                        if let Some(expr) = selection {
                            match expr {
                                Expr::BinaryOp {
                                    left: _,
                                    op: _,
                                    right,
                                } => {
                                    // Extract key from WHERE key = 'value'
                                    let key = extract_string_value(right)?;
                                    let key_str = key.trim_matches('\'');

                                    match self.engine.get_cf(&cf, key_str.as_bytes()) {
                                        Ok(Some(value)) => Ok(SqlResult::Rows {
                                            columns: vec!["key".to_string(), "value".to_string()],
                                            data: vec![vec![
                                                key_str.to_string(),
                                                String::from_utf8_lossy(&value).to_string(),
                                            ]],
                                        }),
                                        Ok(None) => Ok(SqlResult::Rows {
                                            columns: vec!["key".to_string(), "value".to_string()],
                                            data: vec![],
                                        }),
                                        Err(e) => Err(e),
                                    }
                                }
                                _ => Err(crate::infra::error::LsmError::InvalidArgument(
                                    "Unsupported WHERE clause".to_string(),
                                )),
                            }
                        } else {
                            // Full scan
                            let results = self.engine.scan_cf(
                                &cf,
                                None,
                                None,
                                Some(crate::core::engine::MAX_SCAN_LIMIT),
                            )?;
                            let columns = vec!["key".to_string(), "value".to_string()];
                            let data: Vec<Vec<String>> = results
                                .into_iter()
                                .map(|(k, v)| {
                                    vec![
                                        String::from_utf8_lossy(&k).to_string(),
                                        String::from_utf8_lossy(&v).to_string(),
                                    ]
                                })
                                .collect();
                            Ok(SqlResult::Rows { columns, data })
                        }
                    }
                    _ => Err(crate::infra::error::LsmError::InvalidArgument(
                        "Only SELECT queries are supported".to_string(),
                    )),
                }
            }
            SqlStatement::Insert {
                table_name,
                columns,
                source,
                ..
            } => {
                let cf = object_name_to_string(table_name);

                // Extract the source query
                let source_query = source.as_ref().ok_or_else(|| {
                    crate::infra::error::LsmError::InvalidArgument(
                        "INSERT requires a VALUES clause".to_string(),
                    )
                })?;

                // Extract values from the INSERT source
                match &*source_query.body {
                    SetExpr::Values(values) => {
                        if values.rows.is_empty() {
                            return Err(crate::infra::error::LsmError::InvalidArgument(
                                "INSERT requires at least one row".to_string(),
                            ));
                        }
                        let row = &values.rows[0];

                        // Determine position of key and value columns
                        let col_names: Vec<String> = columns
                            .iter()
                            .map(|c| c.value.to_lowercase())
                            .collect();

                        let key_idx = col_names.iter().position(|c| c == "key");
                        let value_idx = col_names.iter().position(|c| c == "value");

                        // If no columns specified, assume (key, value)
                        let (key_str, value_str) = if columns.is_empty() && row.len() >= 2 {
                            (
                                extract_string_value(&row[0])?,
                                extract_string_value(&row[1])?,
                            )
                        } else {
                            let ki = key_idx.ok_or_else(|| {
                                crate::infra::error::LsmError::InvalidArgument(
                                    "INSERT requires a 'key' column".to_string(),
                                )
                            })?;
                            let vi = value_idx.ok_or_else(|| {
                                crate::infra::error::LsmError::InvalidArgument(
                                    "INSERT requires a 'value' column".to_string(),
                                )
                            })?;
                            (
                                extract_string_value(&row[ki])?,
                                extract_string_value(&row[vi])?,
                            )
                        };

                        let key = key_str.trim_matches('\'');
                        let value = value_str.trim_matches('\'');

                        self.engine
                            .put_cf(&cf, key.as_bytes().to_vec(), value.as_bytes().to_vec())?;

                        Ok(SqlResult::Affected(1))
                    }
                    _ => Err(crate::infra::error::LsmError::InvalidArgument(
                        "INSERT source must be VALUES".to_string(),
                    )),
                }
            }
            SqlStatement::Delete {
                from,
                selection,
                ..
            } => {
                let cf = from_table_name(from).unwrap_or_else(|| "default".to_string());

                if let Some(expr) = selection {
                    match expr {
                        Expr::BinaryOp {
                            left: _,
                            op: _,
                            right,
                        } => {
                            let key_str = extract_string_value(right)?;
                            let key = key_str.trim_matches('\'');

                            self.engine.delete_cf(&cf, key.as_bytes())?;

                            Ok(SqlResult::Affected(1))
                        }
                        _ => Err(crate::infra::error::LsmError::InvalidArgument(
                            "DELETE requires a WHERE key = '<key>' clause".to_string(),
                        )),
                    }
                } else {
                    Err(crate::infra::error::LsmError::InvalidArgument(
                        "DELETE without WHERE is not supported".to_string(),
                    ))
                }
            }
            _ => Err(crate::infra::error::LsmError::InvalidArgument(
                "Unsupported SQL statement. Supported: SELECT, INSERT, DELETE".to_string(),
            )),
        }
    }
}

/// Extract the table name from a `FROM` clause (Vec<TableWithJoins>).
fn table_name_from_from_clause(from: &[TableWithJoins]) -> Option<String> {
    from.first()
        .and_then(|twj| table_factor_name(&twj.relation))
}

/// Extract the table name from a `FromTable` enum.
fn from_table_name(from: &FromTable) -> Option<String> {
    match from {
        FromTable::WithFromKeyword(tables) | FromTable::WithoutKeyword(tables) => {
            tables.first().and_then(|twj| table_factor_name(&twj.relation))
        }
    }
}

/// Extract the table name from a `TableFactor`.
fn table_factor_name(factor: &TableFactor) -> Option<String> {
    match factor {
        TableFactor::Table { name, .. } => object_name_to_string(name).into(),
        _ => None,
    }
}

/// Convert an ObjectName to a plain string.
fn object_name_to_string(name: &ObjectName) -> String {
    name.0
        .first()
        .map(|ident| ident.value.clone())
        .unwrap_or_else(|| "default".to_string())
}

/// Extract a string value from an expression.
fn extract_string_value(expr: &Expr) -> Result<String> {
    match expr {
        Expr::Value(Value::SingleQuotedString(s)) => Ok(format!("'{}'", s)),
        Expr::Value(Value::Number(n, _)) => Ok(n.clone()),
        Expr::Value(Value::Boolean(b)) => Ok(b.to_string()),
        Expr::Identifier(ident) => Ok(ident.value.clone()),
        _ => Err(crate::infra::error::LsmError::InvalidArgument(format!(
            "Expected a string literal, got: {:?}",
            expr
        ))),
    }
}

/// Format an SQL result for human-readable display.
pub fn format_sql_result(result: &SqlResult) -> String {
    match result {
        SqlResult::Rows { columns, data } => {
            if data.is_empty() {
                return "(no rows)".to_string();
            }

            // Calculate column widths
            let col_widths: Vec<usize> = columns
                .iter()
                .enumerate()
                .map(|(i, col)| {
                    let max_data = data
                        .iter()
                        .map(|row| row.get(i).map(|s| s.len()).unwrap_or(0))
                        .max()
                        .unwrap_or(0);
                    col.len().max(max_data)
                })
                .collect();

            let mut output = String::new();

            // Header
            for (i, col) in columns.iter().enumerate() {
                if i > 0 {
                    output.push_str(" | ");
                }
                output.push_str(&format!("{:width$}", col, width = col_widths[i]));
            }
            output.push('\n');

            // Separator
            for (i, w) in col_widths.iter().enumerate() {
                if i > 0 {
                    output.push_str("-+-");
                }
                output.push_str(&"-".repeat(*w));
            }
            output.push('\n');

            // Data rows
            for row in data {
                for (i, val) in row.iter().enumerate() {
                    if i > 0 {
                        output.push_str(" | ");
                    }
                    output.push_str(&format!("{:width$}", val, width = col_widths[i]));
                }
                output.push('\n');
            }

            output.push_str(&format!("({} row(s))\n", data.len()));
            output
        }
        SqlResult::Affected(n) => format!("Affected rows: {}", n),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::config::LsmConfig;
    use crate::storage::cache::GlobalBlockCache;
    use std::sync::Arc;

    fn setup_engine() -> Engine<Arc<GlobalBlockCache>> {
        let dir = tempfile::tempdir().unwrap();
        let mut config = LsmConfig::default();
        config.core.dir_path = dir.path().to_path_buf();
        Engine::<Arc<GlobalBlockCache>>::new_from_config(&config, GlobalBlockCache::new(100, 4096)).unwrap()
    }

    #[test]
    fn test_sql_insert_and_select() {
        let engine = setup_engine();
        let sql = SqlEngine::new(&engine);

        // Insert a key
        let result = sql
            .execute("INSERT INTO default (key, value) VALUES ('k1', 'v1')")
            .unwrap();
        match result {
            SqlResult::Affected(n) => assert_eq!(n, 1),
            _ => panic!("Expected Affected"),
        }

        // Select it back
        let result = sql
            .execute("SELECT * FROM default WHERE key = 'k1'")
            .unwrap();
        match result {
            SqlResult::Rows { columns, data } => {
                assert_eq!(columns, vec!["key", "value"]);
                assert_eq!(data.len(), 1);
                assert_eq!(data[0], vec!["k1", "v1"]);
            }
            _ => panic!("Expected Rows"),
        }
    }

    #[test]
    fn test_sql_select_all() {
        let engine = setup_engine();
        let sql = SqlEngine::new(&engine);

        sql.execute("INSERT INTO default (key, value) VALUES ('a', '1')")
            .unwrap();
        sql.execute("INSERT INTO default (key, value) VALUES ('b', '2')")
            .unwrap();

        let result = sql.execute("SELECT * FROM default").unwrap();
        match result {
            SqlResult::Rows { columns, data } => {
                assert_eq!(columns, vec!["key", "value"]);
                assert_eq!(data.len(), 2);
            }
            _ => panic!("Expected Rows"),
        }
    }

    #[test]
    fn test_sql_delete() {
        let engine = setup_engine();
        let sql = SqlEngine::new(&engine);

        sql.execute("INSERT INTO default (key, value) VALUES ('k1', 'v1')")
            .unwrap();

        let result = sql
            .execute("DELETE FROM default WHERE key = 'k1'")
            .unwrap();
        match result {
            SqlResult::Affected(n) => assert_eq!(n, 1),
            _ => panic!("Expected Affected"),
        }

        // Verify deletion
        let result = sql
            .execute("SELECT * FROM default WHERE key = 'k1'")
            .unwrap();
        match result {
            SqlResult::Rows { data, .. } => {
                assert_eq!(data.len(), 0);
            }
            _ => panic!("Expected Rows"),
        }
    }

    #[test]
    fn test_sql_insert_without_column_names() {
        let engine = setup_engine();
        let sql = SqlEngine::new(&engine);

        // Some SQL dialects allow VALUES without column names
        let result = sql.execute("INSERT INTO default VALUES ('k1', 'v1')").unwrap();
        match result {
            SqlResult::Affected(n) => assert_eq!(n, 1),
            _ => panic!("Expected Affected"),
        }
    }

    #[test]
    fn test_sql_select_missing_key() {
        let engine = setup_engine();
        let sql = SqlEngine::new(&engine);

        let result = sql
            .execute("SELECT * FROM default WHERE key = 'nonexistent'")
            .unwrap();
        match result {
            SqlResult::Rows { data, .. } => {
                assert_eq!(data.len(), 0);
            }
            _ => panic!("Expected Rows"),
        }
    }

    #[test]
    fn test_format_sql_result() {
        let result = SqlResult::Rows {
            columns: vec!["key".to_string(), "value".to_string()],
            data: vec![
                vec!["k1".to_string(), "v1".to_string()],
                vec!["k2".to_string(), "v2".to_string()],
            ],
        };
        let formatted = format_sql_result(&result);
        assert!(formatted.contains("k1"));
        assert!(formatted.contains("v1"));
        assert!(formatted.contains("k2"));
        assert!(formatted.contains("2 row(s)"));
    }

    #[test]
    fn test_format_empty_result() {
        let result = SqlResult::Rows {
            columns: vec!["key".to_string(), "value".to_string()],
            data: vec![],
        };
        let formatted = format_sql_result(&result);
        assert_eq!(formatted, "(no rows)");
    }

    #[test]
    fn test_sql_insert_with_column_names_any_order() {
        let engine = setup_engine();
        let sql = SqlEngine::new(&engine);

        // Test with column order reversed (value first, key second)
        let result = sql
            .execute("INSERT INTO default (value, key) VALUES ('v1', 'k1')")
            .unwrap();
        match result {
            SqlResult::Affected(n) => assert_eq!(n, 1),
            _ => panic!("Expected Affected"),
        }

        // Verify
        let result = sql
            .execute("SELECT * FROM default WHERE key = 'k1'")
            .unwrap();
        match result {
            SqlResult::Rows { data, .. } => {
                assert_eq!(data.len(), 1);
                assert_eq!(data[0], vec!["k1", "v1"]);
            }
            _ => panic!("Expected Rows"),
        }
    }
}
