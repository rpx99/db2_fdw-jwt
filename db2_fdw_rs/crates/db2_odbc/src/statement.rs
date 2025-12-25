//! ODBC Statement handling
//!
//! This module provides safe prepared statement management with automatic
//! resource cleanup via RAII.

use std::marker::PhantomData;
use tracing::{debug, instrument};

use crate::connection::Db2Connection;
use crate::error::{Db2Error, Db2Result};
use crate::types::{Db2Type, Db2Value, SqlType};

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
}

/// A prepared SQL statement
///
/// This is a safe wrapper around ODBC statement handles.
/// Unlike the C implementation, resource cleanup is automatic.
pub struct PreparedStatement<'conn> {
    /// SQL text for debugging
    sql: String,
    /// Column descriptions (populated after describe)
    columns: Vec<ColumnDesc>,
    /// Prefetch row count
    prefetch: usize,
    /// Statement has been executed
    executed: bool,
    /// Phantom data for connection lifetime
    _conn: PhantomData<&'conn Db2Connection>,
}

impl<'conn> PreparedStatement<'conn> {
    /// Prepare a new statement
    #[instrument(skip(conn), fields(sql = %sql))]
    pub fn prepare(conn: &'conn Db2Connection, sql: &str) -> Db2Result<Self> {
        debug!("Preparing statement");

        // Real implementation would:
        // 1. Allocate statement handle (SQLAllocHandle)
        // 2. Prepare the statement (SQLPrepare)
        // 3. Get column count and descriptions (SQLNumResultCols, SQLDescribeCol)

        Ok(Self {
            sql: sql.to_string(),
            columns: Vec::new(),
            prefetch: crate::DEFAULT_PREFETCH,
            executed: false,
            _conn: PhantomData,
        })
    }

    /// Get the SQL text
    pub fn sql(&self) -> &str {
        &self.sql
    }

    /// Set the prefetch (row array) size
    pub fn set_prefetch(&mut self, rows: usize) {
        self.prefetch = rows;
        debug!(prefetch = rows, "Set prefetch size");
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
    ///
    /// This should be called after prepare to get column metadata.
    pub fn describe(&mut self) -> Db2Result<&[ColumnDesc]> {
        debug!("Describing result set");

        // Real implementation would use SQLNumResultCols and SQLDescribeCol
        // For now, return empty (actual implementation needed)

        Ok(&self.columns)
    }

    /// Bind parameters and execute the statement
    #[instrument(skip(self, params))]
    pub fn execute(&mut self, params: &[ParamInfo]) -> Db2Result<()> {
        debug!(param_count = params.len(), "Executing statement");

        // Real implementation would:
        // 1. Bind each parameter (SQLBindParameter)
        // 2. Execute the statement (SQLExecute)
        // 3. Handle any errors

        for param in params {
            self.bind_parameter(param)?;
        }

        // Execute
        self.executed = true;

        Ok(())
    }

    /// Execute without parameters
    pub fn execute_no_params(&mut self) -> Db2Result<()> {
        self.execute(&[])
    }

    /// Bind a single parameter
    fn bind_parameter(&self, param: &ParamInfo) -> Db2Result<()> {
        debug!(
            position = param.position,
            sql_type = ?param.sql_type,
            "Binding parameter"
        );

        // Real implementation would use SQLBindParameter
        // Different binding based on type:
        // - SQL_C_CHAR for strings
        // - SQL_C_SLONG for integers
        // - SQL_C_DOUBLE for floats
        // - SQL_C_BINARY for binary data
        // - etc.

        Ok(())
    }

    /// Fetch the next row of results
    ///
    /// Returns None when no more rows are available.
    pub fn fetch_next(&mut self) -> Db2Result<Option<Vec<Db2Value>>> {
        if !self.executed {
            return Err(Db2Error::StatementExecution(
                "Statement not yet executed".into(),
            ));
        }

        // Real implementation would:
        // 1. Call SQLFetch
        // 2. For each column, call SQLGetData or read from bound buffers
        // 3. Convert to Db2Value
        // 4. Return None on SQL_NO_DATA

        debug!("Fetching next row");

        // Placeholder - real implementation needed
        Ok(None)
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
        // Real implementation would use SQLRowCount
        Ok(0)
    }

    /// Close the cursor (allows re-execution)
    pub fn close_cursor(&mut self) -> Db2Result<()> {
        debug!("Closing cursor");
        self.executed = false;
        // Real implementation would use SQLCloseCursor
        Ok(())
    }

    /// Cancel the current operation
    pub fn cancel(&self) -> Db2Result<()> {
        debug!("Cancelling statement");
        // Real implementation would use SQLCancel
        Ok(())
    }
}

impl<'conn> Drop for PreparedStatement<'conn> {
    fn drop(&mut self) {
        debug!(sql = %self.sql, "Dropping prepared statement");
        // RAII: Statement handle is automatically freed
        // This prevents the memory leaks possible in the C implementation
    }
}

/// A simple (non-prepared) statement for one-time execution
pub struct Db2Statement<'conn> {
    _conn: PhantomData<&'conn Db2Connection>,
}

impl<'conn> Db2Statement<'conn> {
    /// Execute a SQL statement directly without preparing
    #[instrument(skip(conn))]
    pub fn execute_direct(conn: &'conn Db2Connection, sql: &str) -> Db2Result<Self> {
        debug!(sql = %sql, "Executing direct statement");

        // Real implementation would:
        // 1. Allocate statement handle
        // 2. Call SQLExecDirect
        // 3. Handle results

        Ok(Self { _conn: PhantomData })
    }

    /// Execute and return the affected row count
    pub fn execute_for_count(conn: &'conn Db2Connection, sql: &str) -> Db2Result<i64> {
        let _stmt = Self::execute_direct(conn, sql)?;
        // Real implementation would use SQLRowCount
        Ok(0)
    }
}

/// Batch insert support for efficient bulk operations
pub struct BatchInsert<'conn> {
    statement: PreparedStatement<'conn>,
    batch_size: usize,
    buffered_rows: Vec<Vec<Db2Value>>,
}

impl<'conn> BatchInsert<'conn> {
    /// Create a new batch insert
    pub fn new(
        conn: &'conn Db2Connection,
        sql: &str,
        batch_size: usize,
    ) -> Db2Result<Self> {
        let statement = PreparedStatement::prepare(conn, sql)?;

        Ok(Self {
            statement,
            batch_size,
            buffered_rows: Vec::with_capacity(batch_size),
        })
    }

    /// Add a row to the batch
    pub fn add_row(&mut self, values: Vec<Db2Value>) -> Db2Result<Option<usize>> {
        self.buffered_rows.push(values);

        if self.buffered_rows.len() >= self.batch_size {
            let count = self.flush()?;
            return Ok(Some(count));
        }

        Ok(None)
    }

    /// Flush the current batch to the database
    pub fn flush(&mut self) -> Db2Result<usize> {
        if self.buffered_rows.is_empty() {
            return Ok(0);
        }

        let count = self.buffered_rows.len();
        debug!(batch_size = count, "Flushing batch insert");

        // Real implementation would:
        // 1. Use SQLSetStmtAttr to set SQL_ATTR_PARAMSET_SIZE
        // 2. Bind arrays of values
        // 3. Execute once for the whole batch

        self.buffered_rows.clear();
        Ok(count)
    }

    /// Get the current number of buffered rows
    pub fn buffered_count(&self) -> usize {
        self.buffered_rows.len()
    }

    /// Finish the batch operation, flushing any remaining rows
    pub fn finish(mut self) -> Db2Result<usize> {
        self.flush()
    }
}

impl<'conn> Drop for BatchInsert<'conn> {
    fn drop(&mut self) {
        if !self.buffered_rows.is_empty() {
            tracing::warn!(
                unflushed = self.buffered_rows.len(),
                "BatchInsert dropped with unflushed rows"
            );
            // Could panic or log - we choose to log a warning
            // The data is lost, but we don't crash
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_param_info() {
        let param = ParamInfo::new(1, SqlType::Integer, Db2Value::Integer(42));
        assert_eq!(param.position, 1);
    }

    #[test]
    fn test_column_desc() {
        let col = ColumnDesc {
            position: 1,
            name: "ID".to_string(),
            db2_type: Db2Type::new(SqlType::Integer),
        };
        assert_eq!(col.name, "ID");
    }
}
