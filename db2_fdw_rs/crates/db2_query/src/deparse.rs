//! SQL Deparsing - Convert PostgreSQL expressions to DB2 SQL
//!
//! This module provides safe expression deparsing, replacing the C implementation's
//! sprintf-based approach with type-safe Rust string building.

use std::fmt::Write;
use thiserror::Error;
use tracing::debug;

/// Errors that can occur during deparsing
#[derive(Error, Debug)]
pub enum DeparseError {
    #[error("Unsupported node type: {0}")]
    UnsupportedNode(String),

    #[error("Invalid expression: {0}")]
    InvalidExpression(String),

    #[error("Column not found: {0}")]
    ColumnNotFound(String),

    #[error("Formatting error: {0}")]
    FormatError(#[from] std::fmt::Error),
}

/// Result type for deparse operations
pub type DeparseResult<T> = Result<T, DeparseError>;

/// Context for deparsing operations
#[derive(Debug, Clone)]
pub struct DeparseContext {
    /// Table alias for column references
    pub table_alias: Option<String>,
    /// List of target column names
    pub target_columns: Vec<String>,
    /// Parameter counter for bind variables
    param_counter: u32,
    /// Whether to quote identifiers
    pub quote_identifiers: bool,
}

impl DeparseContext {
    /// Create a new deparse context
    pub fn new() -> Self {
        Self {
            table_alias: None,
            target_columns: Vec::new(),
            param_counter: 0,
            quote_identifiers: true,
        }
    }

    /// Set the table alias
    pub fn with_alias(mut self, alias: impl Into<String>) -> Self {
        self.table_alias = Some(alias.into());
        self
    }

    /// Add target columns
    pub fn with_columns(mut self, columns: Vec<String>) -> Self {
        self.target_columns = columns;
        self
    }

    /// Get the next parameter placeholder
    pub fn next_param(&mut self) -> String {
        self.param_counter += 1;
        format!("?")  // DB2 uses ? for parameter markers
    }

    /// Quote an identifier for DB2
    pub fn quote_identifier(&self, name: &str) -> String {
        if self.quote_identifiers {
            format!("\"{}\"", name.replace('"', "\"\""))
        } else {
            name.to_string()
        }
    }
}

impl Default for DeparseContext {
    fn default() -> Self {
        Self::new()
    }
}

/// SQL expression builder with type safety
///
/// This replaces the C implementation's sprintf-based SQL building
/// with a safe, type-checked approach.
#[derive(Debug)]
pub struct Deparser {
    context: DeparseContext,
    output: String,
}

impl Deparser {
    /// Create a new deparser
    pub fn new(context: DeparseContext) -> Self {
        Self {
            context,
            output: String::with_capacity(256),
        }
    }

    /// Create with default context
    pub fn default_context() -> Self {
        Self::new(DeparseContext::new())
    }

    /// Get the generated SQL
    pub fn sql(&self) -> &str {
        &self.output
    }

    /// Take the generated SQL
    pub fn into_sql(self) -> String {
        self.output
    }

    /// Clear the output
    pub fn clear(&mut self) {
        self.output.clear();
    }

    /// Build a SELECT query
    pub fn build_select(
        &mut self,
        schema: Option<&str>,
        table: &str,
        columns: &[&str],
        where_clause: Option<&str>,
        order_by: Option<&str>,
        limit: Option<u64>,
    ) -> DeparseResult<&str> {
        self.clear();

        // SELECT clause
        write!(self.output, "SELECT ")?;
        if columns.is_empty() {
            write!(self.output, "*")?;
        } else {
            for (i, col) in columns.iter().enumerate() {
                if i > 0 {
                    write!(self.output, ", ")?;
                }
                write!(self.output, "{}", self.context.quote_identifier(col))?;
            }
        }

        // FROM clause
        write!(self.output, " FROM ")?;
        if let Some(s) = schema {
            write!(self.output, "{}.", self.context.quote_identifier(s))?;
        }
        write!(self.output, "{}", self.context.quote_identifier(table))?;

        // WHERE clause
        if let Some(where_sql) = where_clause {
            write!(self.output, " WHERE {}", where_sql)?;
        }

        // ORDER BY clause
        if let Some(order) = order_by {
            write!(self.output, " ORDER BY {}", order)?;
        }

        // FETCH FIRST (DB2's LIMIT equivalent)
        if let Some(n) = limit {
            write!(self.output, " FETCH FIRST {} ROWS ONLY", n)?;
        }

        debug!(sql = %self.output, "Built SELECT query");
        Ok(&self.output)
    }

    /// Build an INSERT query
    pub fn build_insert(
        &mut self,
        schema: Option<&str>,
        table: &str,
        columns: &[&str],
    ) -> DeparseResult<&str> {
        self.clear();

        write!(self.output, "INSERT INTO ")?;
        if let Some(s) = schema {
            write!(self.output, "{}.", self.context.quote_identifier(s))?;
        }
        write!(self.output, "{}", self.context.quote_identifier(table))?;

        // Column list
        write!(self.output, " (")?;
        for (i, col) in columns.iter().enumerate() {
            if i > 0 {
                write!(self.output, ", ")?;
            }
            write!(self.output, "{}", self.context.quote_identifier(col))?;
        }
        write!(self.output, ")")?;

        // VALUES clause with parameter markers
        write!(self.output, " VALUES (")?;
        for i in 0..columns.len() {
            if i > 0 {
                write!(self.output, ", ")?;
            }
            write!(self.output, "?")?;
        }
        write!(self.output, ")")?;

        debug!(sql = %self.output, "Built INSERT query");
        Ok(&self.output)
    }

    /// Build an UPDATE query
    pub fn build_update(
        &mut self,
        schema: Option<&str>,
        table: &str,
        set_columns: &[&str],
        key_columns: &[&str],
    ) -> DeparseResult<&str> {
        self.clear();

        write!(self.output, "UPDATE ")?;
        if let Some(s) = schema {
            write!(self.output, "{}.", self.context.quote_identifier(s))?;
        }
        write!(self.output, "{}", self.context.quote_identifier(table))?;

        // SET clause
        write!(self.output, " SET ")?;
        for (i, col) in set_columns.iter().enumerate() {
            if i > 0 {
                write!(self.output, ", ")?;
            }
            write!(self.output, "{} = ?", self.context.quote_identifier(col))?;
        }

        // WHERE clause for key columns
        if !key_columns.is_empty() {
            write!(self.output, " WHERE ")?;
            for (i, col) in key_columns.iter().enumerate() {
                if i > 0 {
                    write!(self.output, " AND ")?;
                }
                write!(self.output, "{} = ?", self.context.quote_identifier(col))?;
            }
        }

        debug!(sql = %self.output, "Built UPDATE query");
        Ok(&self.output)
    }

    /// Build a DELETE query
    pub fn build_delete(
        &mut self,
        schema: Option<&str>,
        table: &str,
        key_columns: &[&str],
    ) -> DeparseResult<&str> {
        self.clear();

        write!(self.output, "DELETE FROM ")?;
        if let Some(s) = schema {
            write!(self.output, "{}.", self.context.quote_identifier(s))?;
        }
        write!(self.output, "{}", self.context.quote_identifier(table))?;

        // WHERE clause for key columns
        if !key_columns.is_empty() {
            write!(self.output, " WHERE ")?;
            for (i, col) in key_columns.iter().enumerate() {
                if i > 0 {
                    write!(self.output, " AND ")?;
                }
                write!(self.output, "{} = ?", self.context.quote_identifier(col))?;
            }
        }

        debug!(sql = %self.output, "Built DELETE query");
        Ok(&self.output)
    }

    /// Build a TRUNCATE query
    pub fn build_truncate(
        &mut self,
        schema: Option<&str>,
        table: &str,
    ) -> DeparseResult<&str> {
        self.clear();

        write!(self.output, "TRUNCATE TABLE ")?;
        if let Some(s) = schema {
            write!(self.output, "{}.", self.context.quote_identifier(s))?;
        }
        write!(self.output, "{} IMMEDIATE", self.context.quote_identifier(table))?;

        debug!(sql = %self.output, "Built TRUNCATE query");
        Ok(&self.output)
    }

    /// Deparse a literal value for embedding in SQL
    pub fn deparse_literal(&self, value: &db2_odbc::Db2Value) -> String {
        match value {
            db2_odbc::Db2Value::Null => "NULL".to_string(),
            db2_odbc::Db2Value::Text(s) => format!("'{}'", s.replace('\'', "''")),
            db2_odbc::Db2Value::Binary(b) => format!("X'{}'", hex_encode(b)),
            db2_odbc::Db2Value::SmallInt(n) => n.to_string(),
            db2_odbc::Db2Value::Integer(n) => n.to_string(),
            db2_odbc::Db2Value::BigInt(n) => n.to_string(),
            db2_odbc::Db2Value::Real(n) => n.to_string(),
            db2_odbc::Db2Value::Double(n) => n.to_string(),
            db2_odbc::Db2Value::Decimal(d) => d.to_string(),
            db2_odbc::Db2Value::Date(d) => format!("DATE '{}'", d),
            db2_odbc::Db2Value::Time(t) => format!("TIME '{}'", t),
            db2_odbc::Db2Value::Timestamp(ts) => format!("TIMESTAMP '{}'", ts),
            db2_odbc::Db2Value::Boolean(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
            db2_odbc::Db2Value::Xml(x) => format!("XMLPARSE(DOCUMENT '{}')", x.replace('\'', "''")),
        }
    }

    /// Deparse a column reference
    pub fn deparse_column(&self, column: &str) -> String {
        let quoted = self.context.quote_identifier(column);
        match &self.context.table_alias {
            Some(alias) => format!("{}.{}", alias, quoted),
            None => quoted,
        }
    }

    /// Deparse a binary operator expression
    pub fn deparse_binary_op(
        &self,
        left: &str,
        op: &str,
        right: &str,
    ) -> String {
        format!("({} {} {})", left, op, right)
    }

    /// Deparse a comparison operator
    pub fn deparse_comparison(
        &self,
        left: &str,
        op: ComparisonOp,
        right: &str,
    ) -> String {
        format!("{} {} {}", left, op.to_sql(), right)
    }

    /// Deparse IS NULL / IS NOT NULL
    pub fn deparse_null_test(&self, expr: &str, is_null: bool) -> String {
        if is_null {
            format!("{} IS NULL", expr)
        } else {
            format!("{} IS NOT NULL", expr)
        }
    }

    /// Deparse a LIKE expression
    pub fn deparse_like(&self, expr: &str, pattern: &str, escape: Option<char>) -> String {
        match escape {
            Some(esc) => format!("{} LIKE '{}' ESCAPE '{}'", expr, pattern, esc),
            None => format!("{} LIKE '{}'", expr, pattern),
        }
    }

    /// Deparse an IN expression
    pub fn deparse_in(&self, expr: &str, values: &[String], negated: bool) -> String {
        let op = if negated { "NOT IN" } else { "IN" };
        format!("{} {} ({})", expr, op, values.join(", "))
    }

    /// Deparse a BETWEEN expression
    pub fn deparse_between(
        &self,
        expr: &str,
        low: &str,
        high: &str,
        negated: bool,
    ) -> String {
        let op = if negated { "NOT BETWEEN" } else { "BETWEEN" };
        format!("{} {} {} AND {}", expr, op, low, high)
    }
}

/// Comparison operators
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComparisonOp {
    Equal,
    NotEqual,
    LessThan,
    LessEqual,
    GreaterThan,
    GreaterEqual,
}

impl ComparisonOp {
    pub fn to_sql(&self) -> &'static str {
        match self {
            ComparisonOp::Equal => "=",
            ComparisonOp::NotEqual => "<>",
            ComparisonOp::LessThan => "<",
            ComparisonOp::LessEqual => "<=",
            ComparisonOp::GreaterThan => ">",
            ComparisonOp::GreaterEqual => ">=",
        }
    }
}

/// Helper function to hex-encode binary data
fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02X}", b)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quote_identifier() {
        let ctx = DeparseContext::new();
        assert_eq!(ctx.quote_identifier("id"), "\"id\"");
        assert_eq!(ctx.quote_identifier("MY_TABLE"), "\"MY_TABLE\"");
        assert_eq!(ctx.quote_identifier("has\"quote"), "\"has\"\"quote\"");
    }

    #[test]
    fn test_build_select() {
        let mut dp = Deparser::default_context();
        dp.build_select(
            Some("MYSCHEMA"),
            "MYTABLE",
            &["ID", "NAME"],
            Some("ID > 10"),
            None,
            Some(100),
        )
        .unwrap();

        let sql = dp.sql();
        assert!(sql.contains("SELECT"));
        assert!(sql.contains("\"ID\""));
        assert!(sql.contains("\"NAME\""));
        assert!(sql.contains("\"MYSCHEMA\".\"MYTABLE\""));
        assert!(sql.contains("WHERE ID > 10"));
        assert!(sql.contains("FETCH FIRST 100 ROWS ONLY"));
    }

    #[test]
    fn test_build_insert() {
        let mut dp = Deparser::default_context();
        dp.build_insert(None, "MYTABLE", &["COL1", "COL2", "COL3"])
            .unwrap();

        let sql = dp.sql();
        assert!(sql.contains("INSERT INTO"));
        assert!(sql.contains("VALUES (?, ?, ?)"));
    }

    #[test]
    fn test_build_update() {
        let mut dp = Deparser::default_context();
        dp.build_update(None, "MYTABLE", &["NAME", "VALUE"], &["ID"])
            .unwrap();

        let sql = dp.sql();
        assert!(sql.contains("UPDATE"));
        assert!(sql.contains("SET \"NAME\" = ?, \"VALUE\" = ?"));
        assert!(sql.contains("WHERE \"ID\" = ?"));
    }

    #[test]
    fn test_build_delete() {
        let mut dp = Deparser::default_context();
        dp.build_delete(None, "MYTABLE", &["ID"]).unwrap();

        let sql = dp.sql();
        assert!(sql.contains("DELETE FROM"));
        assert!(sql.contains("WHERE \"ID\" = ?"));
    }

    #[test]
    fn test_comparison_ops() {
        assert_eq!(ComparisonOp::Equal.to_sql(), "=");
        assert_eq!(ComparisonOp::NotEqual.to_sql(), "<>");
        assert_eq!(ComparisonOp::LessThan.to_sql(), "<");
    }
}
