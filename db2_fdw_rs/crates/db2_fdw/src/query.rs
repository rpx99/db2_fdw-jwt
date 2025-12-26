//! Query building helpers for FDW operations
//!
//! This module bridges the FDW state with the db2_query deparser.

use db2_query::deparse::{Deparser, DeparseContext};
use tracing::debug;

use crate::options::FdwOptions;

/// Query builder for foreign table operations
pub struct QueryBuilder {
    schema: Option<String>,
    table: String,
    columns: Vec<String>,
    key_columns: Vec<String>,
}

impl QueryBuilder {
    /// Create a new query builder from FDW options
    pub fn from_options(options: &FdwOptions) -> Option<Self> {
        let table = options.table.clone()?;
        Some(Self {
            schema: options.schema.clone(),
            table,
            columns: Vec::new(),
            key_columns: options.key_columns.clone(),
        })
    }

    /// Set the columns to select/modify
    pub fn with_columns(mut self, columns: Vec<String>) -> Self {
        self.columns = columns;
        self
    }

    /// Set the key columns for UPDATE/DELETE
    pub fn with_key_columns(mut self, keys: Vec<String>) -> Self {
        self.key_columns = keys;
        self
    }

    /// Build a SELECT query
    pub fn build_select(
        &self,
        where_clause: Option<&str>,
        order_by: Option<&str>,
        limit: Option<u64>,
    ) -> String {
        let mut dp = Deparser::new(DeparseContext::new());

        let col_refs: Vec<&str> = if self.columns.is_empty() {
            vec![]  // Will produce SELECT *
        } else {
            self.columns.iter().map(|s| s.as_str()).collect()
        };

        let _ = dp.build_select(
            self.schema.as_deref(),
            &self.table,
            &col_refs,
            where_clause,
            order_by,
            limit,
        );

        let sql = dp.into_sql();
        debug!(sql = %sql, "Built SELECT query");
        sql
    }

    /// Build an INSERT query with parameter markers
    pub fn build_insert(&self) -> String {
        let mut dp = Deparser::new(DeparseContext::new());

        let col_refs: Vec<&str> = self.columns.iter().map(|s| s.as_str()).collect();

        let _ = dp.build_insert(
            self.schema.as_deref(),
            &self.table,
            &col_refs,
        );

        let sql = dp.into_sql();
        debug!(sql = %sql, "Built INSERT query");
        sql
    }

    /// Build an UPDATE query with parameter markers
    pub fn build_update(&self) -> String {
        let mut dp = Deparser::new(DeparseContext::new());

        let set_cols: Vec<&str> = self.columns.iter().map(|s| s.as_str()).collect();
        let key_cols: Vec<&str> = self.key_columns.iter().map(|s| s.as_str()).collect();

        let _ = dp.build_update(
            self.schema.as_deref(),
            &self.table,
            &set_cols,
            &key_cols,
        );

        let sql = dp.into_sql();
        debug!(sql = %sql, "Built UPDATE query");
        sql
    }

    /// Build a DELETE query with parameter markers
    pub fn build_delete(&self) -> String {
        let mut dp = Deparser::new(DeparseContext::new());

        let key_cols: Vec<&str> = self.key_columns.iter().map(|s| s.as_str()).collect();

        let _ = dp.build_delete(
            self.schema.as_deref(),
            &self.table,
            &key_cols,
        );

        let sql = dp.into_sql();
        debug!(sql = %sql, "Built DELETE query");
        sql
    }

    /// Build a TRUNCATE query
    pub fn build_truncate(&self) -> String {
        let mut dp = Deparser::new(DeparseContext::new());

        let _ = dp.build_truncate(
            self.schema.as_deref(),
            &self.table,
        );

        let sql = dp.into_sql();
        debug!(sql = %sql, "Built TRUNCATE query");
        sql
    }

    /// Get the table name
    pub fn table(&self) -> &str {
        &self.table
    }

    /// Get the schema name
    pub fn schema(&self) -> Option<&str> {
        self.schema.as_deref()
    }

    /// Get the key columns
    pub fn key_columns(&self) -> &[String] {
        &self.key_columns
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_select_query() {
        let mut opts = FdwOptions::new();
        opts.table = Some("EMPLOYEES".into());
        opts.schema = Some("HR".into());

        let qb = QueryBuilder::from_options(&opts).unwrap()
            .with_columns(vec!["ID".into(), "NAME".into()]);

        let sql = qb.build_select(Some("ID > 10"), None, Some(100));
        assert!(sql.contains("SELECT"));
        assert!(sql.contains("\"HR\".\"EMPLOYEES\""));
        assert!(sql.contains("WHERE ID > 10"));
        assert!(sql.contains("FETCH FIRST 100 ROWS ONLY"));
    }

    #[test]
    fn test_insert_query() {
        let mut opts = FdwOptions::new();
        opts.table = Some("EMPLOYEES".into());

        let qb = QueryBuilder::from_options(&opts).unwrap()
            .with_columns(vec!["NAME".into(), "SALARY".into()]);

        let sql = qb.build_insert();
        assert!(sql.contains("INSERT INTO"));
        assert!(sql.contains("VALUES (?, ?)"));
    }

    #[test]
    fn test_update_query() {
        let mut opts = FdwOptions::new();
        opts.table = Some("EMPLOYEES".into());
        opts.key_columns = vec!["ID".into()];

        let qb = QueryBuilder::from_options(&opts).unwrap()
            .with_columns(vec!["NAME".into(), "SALARY".into()]);

        let sql = qb.build_update();
        assert!(sql.contains("UPDATE"));
        assert!(sql.contains("SET"));
        assert!(sql.contains("WHERE \"ID\" = ?"));
    }

    #[test]
    fn test_delete_query() {
        let mut opts = FdwOptions::new();
        opts.table = Some("EMPLOYEES".into());
        opts.key_columns = vec!["ID".into()];

        let qb = QueryBuilder::from_options(&opts).unwrap();

        let sql = qb.build_delete();
        assert!(sql.contains("DELETE FROM"));
        assert!(sql.contains("WHERE \"ID\" = ?"));
    }
}
