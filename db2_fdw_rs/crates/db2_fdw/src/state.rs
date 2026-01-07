//! FDW State management
//!
//! Manages state passed between planning and execution phases.

use serde::{Deserialize, Serialize};

use crate::options::FdwOptions;
use db2_connection::Db2Session;
use db2_odbc::Db2Value;

/// State passed from planner to executor
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FdwPlanState {
    /// The SQL query to execute
    pub sql: String,
    /// Column names to retrieve
    pub columns: Vec<String>,
    /// Parameter values for the query
    #[serde(skip)]
    pub params: Vec<Db2Value>,
    /// Estimated number of rows
    pub estimated_rows: f64,
    /// Estimated startup cost
    pub startup_cost: f64,
    /// Estimated total cost
    pub total_cost: f64,
    /// Whether this is a parameterized path
    pub is_parameterized: bool,
}

impl Default for FdwPlanState {
    fn default() -> Self {
        Self {
            sql: String::new(),
            columns: Vec::new(),
            params: Vec::new(),
            estimated_rows: 1000.0,
            startup_cost: 10.0,
            total_cost: 1000.0,
            is_parameterized: false,
        }
    }
}

impl FdwPlanState {
    /// Create new plan state
    pub fn new(sql: String, columns: Vec<String>) -> Self {
        Self {
            sql,
            columns,
            ..Default::default()
        }
    }

    /// Set cost estimates
    pub fn with_costs(mut self, startup: f64, total: f64, rows: f64) -> Self {
        self.startup_cost = startup;
        self.total_cost = total;
        self.estimated_rows = rows;
        self
    }

    /// Serialize to bytes for passing through PostgreSQL
    pub fn serialize(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }

    /// Deserialize from bytes
    pub fn deserialize(data: &[u8]) -> Option<Self> {
        serde_json::from_slice(data).ok()
    }
}

/// Execution state for foreign scan
pub struct FdwScanState {
    /// Parsed options
    pub options: FdwOptions,
    /// The active session
    pub session: Option<Db2Session>,
    /// Plan state
    pub plan: FdwPlanState,
    /// Number of rows fetched
    pub rows_fetched: u64,
    /// Current row buffer
    pub current_row: Option<Vec<Db2Value>>,
    /// Whether the scan is finished
    pub finished: bool,
    /// Whether the scan needs to be reinitialized
    pub needs_reinit: bool,
}

impl FdwScanState {
    /// Create new scan state
    pub fn new(options: FdwOptions, plan: FdwPlanState) -> Self {
        Self {
            options,
            session: None,
            plan,
            rows_fetched: 0,
            current_row: None,
            finished: false,
            needs_reinit: false,
        }
    }

    /// Initialize the session
    pub fn init_session(&mut self) -> Result<(), db2_odbc::Db2Error> {
        if self.session.is_some() {
            return Ok(());
        }

        let conn_opts = self.options.to_connection_options().ok_or_else(|| {
            db2_odbc::Db2Error::ConfigurationError("Missing connection options".into())
        })?;

        let session = Db2Session::new(&conn_opts)?;
        self.session = Some(session);
        Ok(())
    }

    /// Get the session reference
    pub fn session(&self) -> Option<&Db2Session> {
        self.session.as_ref()
    }

    /// Get the session mutably
    pub fn session_mut(&mut self) -> Option<&mut Db2Session> {
        self.session.as_mut()
    }

    /// Mark as finished
    pub fn mark_finished(&mut self) {
        self.finished = true;
    }
}

/// Execution state for foreign modify (INSERT/UPDATE/DELETE)
pub struct FdwModifyState {
    /// Parsed options
    pub options: FdwOptions,
    /// The active session
    pub session: Option<Db2Session>,
    /// The SQL for modification
    pub sql: String,
    /// Target columns
    pub target_columns: Vec<String>,
    /// Key columns for UPDATE/DELETE
    pub key_columns: Vec<String>,
    /// Number of rows affected
    pub rows_affected: u64,
    /// Batch buffer for batch insert
    pub batch_buffer: Vec<Vec<Db2Value>>,
    /// Batch size
    pub batch_size: usize,
}

impl FdwModifyState {
    /// Create new modify state
    pub fn new(options: FdwOptions, sql: String) -> Self {
        let batch_size = options.batch_size;
        Self {
            options,
            session: None,
            sql,
            target_columns: Vec::new(),
            key_columns: Vec::new(),
            rows_affected: 0,
            batch_buffer: Vec::with_capacity(batch_size),
            batch_size,
        }
    }

    /// Initialize the session
    pub fn init_session(&mut self) -> Result<(), db2_odbc::Db2Error> {
        if self.session.is_some() {
            return Ok(());
        }

        let conn_opts = self.options.to_connection_options().ok_or_else(|| {
            db2_odbc::Db2Error::ConfigurationError("Missing connection options".into())
        })?;

        let session = Db2Session::new(&conn_opts)?;
        self.session = Some(session);
        Ok(())
    }

    /// Add a row to the batch buffer
    pub fn add_to_batch(&mut self, row: Vec<Db2Value>) -> bool {
        self.batch_buffer.push(row);
        self.batch_buffer.len() >= self.batch_size
    }

    /// Flush the batch buffer
    pub fn flush_batch(&mut self) -> Result<usize, db2_odbc::Db2Error> {
        if self.batch_buffer.is_empty() {
            return Ok(0);
        }

        let count = self.batch_buffer.len();
        // Real implementation would execute batch insert here
        self.batch_buffer.clear();
        self.rows_affected += count as u64;
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plan_state_serialization() {
        let state = FdwPlanState::new(
            "SELECT * FROM test".into(),
            vec!["col1".into(), "col2".into()],
        );

        let bytes = state.serialize();
        let restored = FdwPlanState::deserialize(&bytes).unwrap();

        assert_eq!(restored.sql, state.sql);
        assert_eq!(restored.columns, state.columns);
    }

    #[test]
    fn test_scan_state_creation() {
        let options = FdwOptions::new();
        let plan = FdwPlanState::default();
        let state = FdwScanState::new(options, plan);

        assert_eq!(state.rows_fetched, 0);
        assert!(!state.finished);
    }

    #[test]
    fn test_modify_state_batch() {
        let mut options = FdwOptions::new();
        options.batch_size = 2;

        let mut state = FdwModifyState::new(options, "INSERT INTO test".into());

        // First row shouldn't trigger flush
        assert!(!state.add_to_batch(vec![Db2Value::Integer(1)]));

        // Second row should trigger flush
        assert!(state.add_to_batch(vec![Db2Value::Integer(2)]));
    }
}
