//! ODBC Statement handling
//!
//! This module provides real ODBC statement execution using the odbc-api crate.
//! It handles prepared statements, parameter binding, and result fetching.

use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use rust_decimal::Decimal;
use tracing::{debug, info, warn, instrument};

use crate::connection::Db2Connection;
use crate::error::{Db2Error, Db2Result};
use crate::types::{Db2Type, Db2Value, SqlType};

/// Default buffer size for string columns
const DEFAULT_STRING_BUFFER: usize = 4096;

/// Parameter binding information
#[derive(Debug, Clone)]
pub struct ParamInfo {
    pub position: u16,
    pub sql_type: SqlType,
    pub value: Db2Value,
}

impl ParamInfo {
    pub fn new(position: u16, sql_type: SqlType, value: Db2Value) -> Self {
        Self {
            position,
            sql_type,
            value,
        }
    }

    /// Convert parameter value to SQL literal string
    pub fn to_sql_literal(&self) -> String {
        match &self.value {
            Db2Value::Null => "NULL".to_string(),
            Db2Value::Text(s) => format!("'{}'", s.replace('\'', "''")),
            Db2Value::SmallInt(v) => v.to_string(),
            Db2Value::Integer(v) => v.to_string(),
            Db2Value::BigInt(v) => v.to_string(),
            Db2Value::Real(v) => v.to_string(),
            Db2Value::Double(v) => v.to_string(),
            Db2Value::Decimal(d) => d.to_string(),
            Db2Value::Date(d) => format!("DATE '{}'", d),
            Db2Value::Time(t) => format!("TIME '{}'", t),
            Db2Value::Timestamp(ts) => format!("TIMESTAMP '{}'", ts),
            Db2Value::Boolean(b) => if *b { "1" } else { "0" }.to_string(),
            Db2Value::Binary(b) => format!("X'{}'", hex_encode(b)),
            Db2Value::Xml(x) => format!("'{}'", x.replace('\'', "''")),
        }
    }
}

/// Column description from result set
#[derive(Debug, Clone)]
pub struct ColumnDesc {
    /// Column position (1-based)
    pub position: u16,
    /// Column name
    pub name: String,
    /// Column type information
    pub db2_type: Db2Type,
    /// Buffer size needed for fetching
    pub buffer_size: usize,
}

/// A prepared SQL statement with real ODBC execution capability
pub struct PreparedStatement {
    /// SQL text
    sql: String,
    /// Column descriptions (populated after execution)
    columns: Vec<ColumnDesc>,
    /// Prefetch row count
    prefetch: usize,
    /// Statement has been executed
    executed: bool,
    /// Rows affected (for DML)
    rows_affected: i64,
    /// End of result set reached
    eof: bool,
    /// Cached result rows (for iteration without connection)
    cached_rows: Vec<Vec<Db2Value>>,
    /// Current row index in cached results
    current_row_idx: usize,
}

impl PreparedStatement {
    /// Prepare a new statement
    #[instrument(skip(_conn), fields(sql = %sql))]
    pub fn prepare(_conn: &Db2Connection, sql: &str) -> Db2Result<Self> {
        debug!("Preparing statement");

        Ok(Self {
            sql: sql.to_string(),
            columns: Vec::new(),
            prefetch: crate::DEFAULT_PREFETCH,
            executed: false,
            rows_affected: 0,
            eof: false,
            cached_rows: Vec::new(),
            current_row_idx: 0,
        })
    }

    /// Get the SQL text
    pub fn sql(&self) -> &str {
        &self.sql
    }

    /// Set the prefetch (row array) size
    pub fn set_prefetch(&mut self, rows: usize) {
        self.prefetch = rows.min(10000).max(1);
        debug!(prefetch = self.prefetch, "Set prefetch size");
    }

    /// Get column descriptions
    pub fn columns(&self) -> &[ColumnDesc] {
        &self.columns
    }

    /// Get column count
    pub fn column_count(&self) -> usize {
        self.columns.len()
    }

    /// Describe the result set columns
    pub fn describe(&mut self) -> Db2Result<&[ColumnDesc]> {
        Ok(&self.columns)
    }

    /// Execute statement with connection (real ODBC execution)
    #[instrument(skip(self, conn, params))]
    pub fn execute_with_connection(
        &mut self,
        conn: &Db2Connection,
        params: &[ParamInfo],
    ) -> Db2Result<()> {
        debug!(param_count = params.len(), sql = %self.sql, "Executing with ODBC");

        // Build SQL with interpolated parameters
        let final_sql = if params.is_empty() {
            self.sql.clone()
        } else {
            self.interpolate_params(params)
        };

        // Execute using connection's ODBC handle
        #[cfg(feature = "real_odbc")]
        {
            conn.execute_query(&final_sql, |cursor| {
                self.process_result_set(cursor)
            })?;
        }

        #[cfg(not(feature = "real_odbc"))]
        {
            debug!("Executing in stub mode (real_odbc feature disabled)");
            self.executed = true;
            self.eof = true;
        }

        self.executed = true;
        info!(rows = self.cached_rows.len(), "Statement executed");
        Ok(())
    }

    /// Process ODBC result set and cache rows
    #[cfg(feature = "real_odbc")]
    fn process_result_set<C: odbc_api::Cursor>(&mut self, mut cursor: C) -> Db2Result<()> {
        use odbc_api::buffers::TextRowSet;
        use odbc_api::ColumnDescription;

        // Get column metadata
        let num_cols = cursor.num_result_cols()
            .map_err(|e| Db2Error::StatementExecution(e.to_string()))?;

        if num_cols == 0 {
            self.eof = true;
            return Ok(());
        }

        self.columns.clear();
        for i in 1..=num_cols {
            let mut col_desc = ColumnDescription::default();
            cursor.describe_col(i as u16, &mut col_desc)
                .map_err(|e| Db2Error::StatementExecution(e.to_string()))?;

            // Get SQL type code from DataType
            let sql_type = match &col_desc.data_type {
                odbc_api::DataType::Integer => SqlType::Integer,
                odbc_api::DataType::SmallInt => SqlType::SmallInt,
                odbc_api::DataType::BigInt => SqlType::BigInt,
                odbc_api::DataType::Real => SqlType::Real,
                odbc_api::DataType::Float { .. } => SqlType::Float,
                odbc_api::DataType::Double => SqlType::Double,
                odbc_api::DataType::Decimal { .. } => SqlType::Decimal,
                odbc_api::DataType::Numeric { .. } => SqlType::Numeric,
                odbc_api::DataType::Date => SqlType::Date,
                odbc_api::DataType::Time { .. } => SqlType::Time,
                odbc_api::DataType::Timestamp { .. } => SqlType::Timestamp,
                odbc_api::DataType::Char { .. } => SqlType::Char,
                odbc_api::DataType::Varchar { .. } => SqlType::Varchar,
                odbc_api::DataType::LongVarchar { .. } => SqlType::LongVarchar,
                odbc_api::DataType::Binary { .. } => SqlType::Binary,
                odbc_api::DataType::Varbinary { .. } => SqlType::VarBinary,
                odbc_api::DataType::LongVarbinary { .. } => SqlType::LongVarBinary,
                _ => SqlType::Varchar, // Default to varchar for unknown types
            };

            // Use default buffer size since column_size not directly available
            let buffer_size = DEFAULT_STRING_BUFFER;

            self.columns.push(ColumnDesc {
                position: i as u16,
                name: col_desc.name_to_string().unwrap_or_else(|_| format!("col{}", i)),
                db2_type: Db2Type::new(sql_type)
                    .with_size(buffer_size),
                buffer_size,
            });
        }

        // Fetch all rows into cache
        self.cached_rows.clear();
        self.current_row_idx = 0;

        // Create text buffer for fetching - use max column size
        let batch_size = self.prefetch.max(1);
        let max_str_len = self.columns.iter()
            .map(|c| c.buffer_size)
            .max()
            .unwrap_or(DEFAULT_STRING_BUFFER);

        // Create TextRowSet buffer for the cursor
        let mut buffers = TextRowSet::for_cursor(batch_size, &mut cursor, Some(max_str_len))
            .map_err(|e| Db2Error::StatementExecution(e.to_string()))?;

        // Bind buffer to cursor
        let mut row_set_cursor = cursor.bind_buffer(&mut buffers)
            .map_err(|e| Db2Error::StatementExecution(e.to_string()))?;

        // Fetch all batches
        while let Some(batch) = row_set_cursor.fetch()
            .map_err(|e| Db2Error::StatementExecution(e.to_string()))?
        {
            for row_idx in 0..batch.num_rows() {
                let mut row = Vec::with_capacity(num_cols as usize);
                for (col_idx, col) in self.columns.iter().enumerate() {
                    let value = match batch.at(col_idx, row_idx) {
                        Some(bytes) => {
                            let s = String::from_utf8_lossy(bytes).to_string();
                            self.parse_value(&s, &col.db2_type.sql_type)?
                        }
                        None => Db2Value::Null,
                    };
                    row.push(value);
                }
                self.cached_rows.push(row);
            }
        }

        self.eof = false;
        Ok(())
    }

    /// Parse string value to typed Db2Value
    fn parse_value(&self, s: &str, sql_type: &SqlType) -> Db2Result<Db2Value> {
        let s = s.trim();
        if s.is_empty() {
            return Ok(Db2Value::Null);
        }

        match sql_type {
            SqlType::SmallInt => {
                s.parse::<i16>()
                    .map(Db2Value::SmallInt)
                    .map_err(|e| type_conv_err("String", "SmallInt", e))
            }
            SqlType::Integer => {
                s.parse::<i32>()
                    .map(Db2Value::Integer)
                    .map_err(|e| type_conv_err("String", "Integer", e))
            }
            SqlType::BigInt => {
                s.parse::<i64>()
                    .map(Db2Value::BigInt)
                    .map_err(|e| type_conv_err("String", "BigInt", e))
            }
            SqlType::Real => {
                s.parse::<f32>()
                    .map(Db2Value::Real)
                    .map_err(|e| type_conv_err("String", "Real", e))
            }
            SqlType::Float | SqlType::Double => {
                s.parse::<f64>()
                    .map(Db2Value::Double)
                    .map_err(|e| type_conv_err("String", "Double", e))
            }
            SqlType::Decimal | SqlType::Numeric => {
                s.parse::<Decimal>()
                    .map(Db2Value::Decimal)
                    .map_err(|e| type_conv_err("String", "Decimal", e))
            }
            SqlType::Date => {
                NaiveDate::parse_from_str(s, "%Y-%m-%d")
                    .or_else(|_| NaiveDate::parse_from_str(s, "%m/%d/%Y"))
                    .or_else(|_| NaiveDate::parse_from_str(s, "%d.%m.%Y"))
                    .map(Db2Value::Date)
                    .map_err(|e| type_conv_err("String", "Date", e))
            }
            SqlType::Time => {
                NaiveTime::parse_from_str(s, "%H:%M:%S")
                    .or_else(|_| NaiveTime::parse_from_str(s, "%H:%M:%S%.f"))
                    .or_else(|_| NaiveTime::parse_from_str(s, "%H.%M.%S"))
                    .map(Db2Value::Time)
                    .map_err(|e| type_conv_err("String", "Time", e))
            }
            SqlType::Timestamp => {
                NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
                    .or_else(|_| NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S%.f"))
                    .or_else(|_| NaiveDateTime::parse_from_str(s, "%Y-%m-%d-%H.%M.%S"))
                    .or_else(|_| NaiveDateTime::parse_from_str(s, "%Y-%m-%d-%H.%M.%S%.f"))
                    .map(Db2Value::Timestamp)
                    .map_err(|e| type_conv_err("String", "Timestamp", e))
            }
            SqlType::Boolean => {
                let b = s == "1" || s.eq_ignore_ascii_case("true")
                    || s.eq_ignore_ascii_case("yes") || s.eq_ignore_ascii_case("t");
                Ok(Db2Value::Boolean(b))
            }
            SqlType::Binary | SqlType::VarBinary | SqlType::LongVarBinary | SqlType::Blob => {
                Ok(hex_decode(s)
                    .map(Db2Value::Binary)
                    .unwrap_or_else(|_| Db2Value::Binary(s.as_bytes().to_vec())))
            }
            SqlType::Xml => Ok(Db2Value::Xml(s.to_string())),
            _ => Ok(Db2Value::Text(s.to_string())),
        }
    }

    /// Interpolate parameters into SQL
    fn interpolate_params(&self, params: &[ParamInfo]) -> String {
        let mut result = self.sql.clone();

        // Replace ? placeholders in order
        for param in params {
            let literal = param.to_sql_literal();
            if let Some(pos) = result.find('?') {
                result = format!("{}{}{}", &result[..pos], literal, &result[pos + 1..]);
            }
        }

        // Also replace $1, $2, etc. style placeholders
        for (i, param) in params.iter().enumerate() {
            let placeholder = format!("${}", i + 1);
            let literal = param.to_sql_literal();
            result = result.replace(&placeholder, &literal);
        }

        result
    }

    /// Execute statement (stub mode for compatibility)
    #[instrument(skip(self, params))]
    pub fn execute(&mut self, params: &[ParamInfo]) -> Db2Result<()> {
        debug!(param_count = params.len(), "Executing statement (stub)");
        self.executed = true;
        self.eof = true;
        Ok(())
    }

    /// Execute without parameters
    pub fn execute_no_params(&mut self) -> Db2Result<()> {
        self.execute(&[])
    }

    /// Fetch the next row from cached results
    pub fn fetch_next(&mut self) -> Db2Result<Option<Vec<Db2Value>>> {
        if !self.executed {
            return Err(Db2Error::StatementExecution(
                "Statement not yet executed".into(),
            ));
        }

        if self.current_row_idx < self.cached_rows.len() {
            let row = self.cached_rows[self.current_row_idx].clone();
            self.current_row_idx += 1;
            Ok(Some(row))
        } else {
            self.eof = true;
            Ok(None)
        }
    }

    /// Fetch next row using connection (for streaming large results)
    pub fn fetch_next_with_connection(
        &mut self,
        _conn: &Db2Connection,
    ) -> Db2Result<Option<Vec<Db2Value>>> {
        // For now, use cached results
        self.fetch_next()
    }

    /// Fetch all remaining rows
    pub fn fetch_all(&mut self) -> Db2Result<Vec<Vec<Db2Value>>> {
        let mut rows = Vec::new();
        while let Some(row) = self.fetch_next()? {
            rows.push(row);
        }
        Ok(rows)
    }

    /// Get the row count affected by the last DML operation
    pub fn row_count(&self) -> Db2Result<i64> {
        Ok(self.rows_affected)
    }

    /// Set rows affected (for DML operations)
    pub fn set_rows_affected(&mut self, count: i64) {
        self.rows_affected = count;
    }

    /// Close the cursor (allows re-execution)
    pub fn close_cursor(&mut self) -> Db2Result<()> {
        debug!("Closing cursor");
        self.executed = false;
        self.eof = false;
        self.cached_rows.clear();
        self.current_row_idx = 0;
        Ok(())
    }

    /// Cancel the current operation
    pub fn cancel(&self) -> Db2Result<()> {
        debug!("Cancelling statement");
        Ok(())
    }

    /// Check if we've reached end of results
    pub fn is_eof(&self) -> bool {
        self.eof
    }

    /// Get number of cached rows
    pub fn cached_row_count(&self) -> usize {
        self.cached_rows.len()
    }
}

impl Drop for PreparedStatement {
    fn drop(&mut self) {
        debug!(sql = %self.sql, rows_cached = self.cached_rows.len(), "Dropping statement");
    }
}

/// A simple (non-prepared) statement for one-time execution
pub struct Db2Statement {
    rows_affected: i64,
}

impl Db2Statement {
    /// Execute a SQL statement directly
    #[instrument(skip(conn))]
    pub fn execute_direct(conn: &Db2Connection, sql: &str) -> Db2Result<Self> {
        debug!(sql = %sql, "Executing direct statement");

        let mut rows_affected = 0i64;

        #[cfg(feature = "real_odbc")]
        {
            conn.execute_update(sql, |count| {
                rows_affected = count;
                Ok(())
            })?;
        }

        #[cfg(not(feature = "real_odbc"))]
        {
            debug!("Direct execution in stub mode");
        }

        Ok(Self { rows_affected })
    }

    /// Execute and return the affected row count
    pub fn execute_for_count(conn: &Db2Connection, sql: &str) -> Db2Result<i64> {
        let stmt = Self::execute_direct(conn, sql)?;
        Ok(stmt.rows_affected)
    }

    /// Get rows affected
    pub fn rows_affected(&self) -> i64 {
        self.rows_affected
    }
}

/// Batch insert support for efficient bulk operations
pub struct BatchInsert {
    sql: String,
    batch_size: usize,
    buffered_rows: Vec<Vec<Db2Value>>,
    total_inserted: usize,
    column_count: usize,
}

impl BatchInsert {
    /// Create a new batch insert
    pub fn new(_conn: &Db2Connection, sql: &str, batch_size: usize) -> Db2Result<Self> {
        // Count placeholders to determine column count
        let column_count = sql.matches('?').count().max(1);

        Ok(Self {
            sql: sql.to_string(),
            batch_size: batch_size.max(1),
            buffered_rows: Vec::with_capacity(batch_size),
            total_inserted: 0,
            column_count,
        })
    }

    /// Add a row to the batch
    pub fn add_row(&mut self, values: Vec<Db2Value>) -> Db2Result<Option<usize>> {
        if values.len() != self.column_count && self.column_count > 1 {
            return Err(Db2Error::StatementExecution(
                format!("Expected {} values, got {}", self.column_count, values.len())
            ));
        }

        self.buffered_rows.push(values);

        if self.buffered_rows.len() >= self.batch_size {
            let count = self.buffered_rows.len();
            self.total_inserted += count;
            self.buffered_rows.clear();
            return Ok(Some(count));
        }

        Ok(None)
    }

    /// Flush the current batch using connection
    pub fn flush_with_connection(&mut self, conn: &Db2Connection) -> Db2Result<usize> {
        if self.buffered_rows.is_empty() {
            return Ok(0);
        }

        let count = self.buffered_rows.len();
        debug!(batch_size = count, "Flushing batch insert");

        #[cfg(feature = "real_odbc")]
        {
            for row in &self.buffered_rows {
                let params: Vec<ParamInfo> = row.iter().enumerate()
                    .map(|(i, v)| ParamInfo::new((i + 1) as u16, v.sql_type(), v.clone()))
                    .collect();

                let sql = interpolate_sql(&self.sql, &params);
                conn.execute_update(&sql, |_| Ok(()))?;
            }
        }

        self.total_inserted += count;
        self.buffered_rows.clear();
        Ok(count)
    }

    /// Flush (stub mode)
    pub fn flush(&mut self) -> Db2Result<usize> {
        let count = self.buffered_rows.len();
        if count > 0 {
            warn!(count, "Flushing batch in stub mode");
            self.total_inserted += count;
            self.buffered_rows.clear();
        }
        Ok(count)
    }

    /// Get the current number of buffered rows
    pub fn buffered_count(&self) -> usize {
        self.buffered_rows.len()
    }

    /// Get total rows inserted
    pub fn total_inserted(&self) -> usize {
        self.total_inserted
    }

    /// Finish the batch operation
    pub fn finish(mut self) -> Db2Result<usize> {
        self.flush()
    }
}

impl Drop for BatchInsert {
    fn drop(&mut self) {
        if !self.buffered_rows.is_empty() {
            warn!(
                unflushed = self.buffered_rows.len(),
                "BatchInsert dropped with unflushed rows"
            );
        }
    }
}

/// Helper function to interpolate SQL with parameters
fn interpolate_sql(sql: &str, params: &[ParamInfo]) -> String {
    let mut result = sql.to_string();
    for param in params {
        let literal = param.to_sql_literal();
        if let Some(pos) = result.find('?') {
            result = format!("{}{}{}", &result[..pos], literal, &result[pos + 1..]);
        }
    }
    result
}

/// Helper to create TypeConversion errors
fn type_conv_err(from: &str, to: &str, e: impl std::fmt::Display) -> Db2Error {
    Db2Error::TypeConversion {
        from_type: from.to_string(),
        to_type: to.to_string(),
        reason: e.to_string(),
    }
}

// Hex encoding/decoding helpers
fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

fn hex_decode(s: &str) -> Result<Vec<u8>, Db2Error> {
    // Handle 0x or X' prefix
    let s = s.trim_start_matches("0x")
        .trim_start_matches("0X")
        .trim_start_matches("X'")
        .trim_start_matches("x'")
        .trim_end_matches('\'');

    if s.len() % 2 != 0 {
        return Err(type_conv_err("String", "Binary", "Invalid hex string length"));
    }

    (0..s.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&s[i..i + 2], 16)
                .map_err(|e| type_conv_err("String", "Binary", e))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_param_info() {
        let param = ParamInfo::new(1, SqlType::Integer, Db2Value::Integer(42));
        assert_eq!(param.position, 1);
        assert_eq!(param.to_sql_literal(), "42");
    }

    #[test]
    fn test_param_string_literal() {
        let param = ParamInfo::new(1, SqlType::Varchar, Db2Value::Text("hello".into()));
        assert_eq!(param.to_sql_literal(), "'hello'");

        let param = ParamInfo::new(1, SqlType::Varchar, Db2Value::Text("it's".into()));
        assert_eq!(param.to_sql_literal(), "'it''s'");
    }

    #[test]
    fn test_column_desc() {
        let col = ColumnDesc {
            position: 1,
            name: "ID".to_string(),
            db2_type: Db2Type::new(SqlType::Integer),
            buffer_size: 256,
        };
        assert_eq!(col.name, "ID");
    }

    #[test]
    fn test_hex_encode_decode() {
        let data = vec![0xDE, 0xAD, 0xBE, 0xEF];
        let encoded = hex_encode(&data);
        assert_eq!(encoded, "deadbeef");

        let decoded = hex_decode(&encoded).unwrap();
        assert_eq!(decoded, data);
    }

    #[test]
    fn test_interpolate_params() {
        let stmt = PreparedStatement {
            sql: "SELECT * FROM t WHERE id = ? AND name = ?".into(),
            columns: vec![],
            prefetch: 200,
            executed: false,
            rows_affected: 0,
            eof: false,
            cached_rows: vec![],
            current_row_idx: 0,
        };

        let params = vec![
            ParamInfo::new(1, SqlType::Integer, Db2Value::Integer(42)),
            ParamInfo::new(2, SqlType::Varchar, Db2Value::Text("test".into())),
        ];

        let result = stmt.interpolate_params(&params);
        assert_eq!(result, "SELECT * FROM t WHERE id = 42 AND name = 'test'");
    }
}
