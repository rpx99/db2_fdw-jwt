//! Session management for DB2 FDW
//!
//! A session represents an active query execution context with statement handles.
//! This is the safe replacement for the C HdlEntry/DB2ConnEntry combination.

use std::sync::Arc;
use tracing::{debug, info, instrument, warn};

use db2_odbc::{Db2Connection, Db2Error, Db2Result, Db2Value, PreparedStatement};
use db2_odbc::statement::ParamInfo;
use crate::pool::get_connection;
use crate::FdwConnectionOptions;

/// Session state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    /// Session is ready for a new query
    Ready,
    /// Statement is prepared but not executed
    Prepared,
    /// Statement is executing
    Executing,
    /// Fetching results
    Fetching,
    /// Session has encountered an error
    Error,
    /// Session is closed
    Closed,
}

/// A query execution session
///
/// This is the safe replacement for the C HdlEntry/DB2ConnEntry combination.
/// It provides RAII-based resource management for statements.
pub struct Db2Session {
    /// Connection handle (from per-backend cache)
    connection: Arc<Db2Connection>,
    /// Current session state
    state: SessionState,
    /// Current prepared statement (if any)
    statement: Option<PreparedStatement>,
    /// Prefetch row count
    prefetch: usize,
    /// Transaction savepoint counter
    savepoint_counter: u32,
}

impl Db2Session {
    /// Create a new session from connection options
    #[instrument(skip(options), fields(server = %options.server))]
    pub fn new(options: &FdwConnectionOptions) -> Db2Result<Self> {
        let connection = get_connection(options)?;

        Ok(Self {
            connection,
            state: SessionState::Ready,
            statement: None,
            prefetch: options.prefetch,
            savepoint_counter: 0,
        })
    }

    /// Create a session from an existing connection
    pub fn from_connection(connection: Arc<Db2Connection>, prefetch: usize) -> Self {
        Self {
            connection,
            state: SessionState::Ready,
            statement: None,
            prefetch,
            savepoint_counter: 0,
        }
    }

    /// Get the current state
    pub fn state(&self) -> SessionState {
        self.state
    }

    /// Get the current SQL query
    pub fn current_sql(&self) -> Option<&str> {
        self.statement.as_ref().map(|s| s.sql())
    }

    /// Get the prefetch size
    pub fn prefetch(&self) -> usize {
        self.prefetch
    }

    /// Set the prefetch size
    pub fn set_prefetch(&mut self, prefetch: usize) {
        self.prefetch = prefetch;
        if let Some(ref mut stmt) = self.statement {
            stmt.set_prefetch(prefetch);
        }
    }

    /// Get the connection reference
    pub fn connection(&self) -> &Db2Connection {
        &self.connection
    }

    /// Check if session is read-only
    pub fn is_read_only(&self) -> bool {
        self.connection.is_read_only()
    }

    /// Prepare a query for execution
    #[instrument(skip(self), fields(sql = %sql))]
    pub fn prepare(&mut self, sql: &str) -> Db2Result<()> {
        if self.state == SessionState::Closed {
            return Err(Db2Error::Internal("Session is closed".into()));
        }

        debug!("Preparing query");

        // Close any existing statement
        self.statement = None;

        // Create new prepared statement
        let mut stmt = PreparedStatement::prepare(&self.connection, sql)?;
        stmt.set_prefetch(self.prefetch);

        self.statement = Some(stmt);
        self.state = SessionState::Prepared;

        Ok(())
    }

    /// Execute the prepared query with parameters
    #[instrument(skip(self, params))]
    pub fn execute(&mut self, params: &[Db2Value]) -> Db2Result<()> {
        if self.state != SessionState::Prepared {
            return Err(Db2Error::Internal(
                "Must prepare query before executing".into(),
            ));
        }

        debug!(param_count = params.len(), "Executing query");
        self.state = SessionState::Executing;

        // Convert Db2Values to ParamInfos
        let param_infos: Vec<ParamInfo> = params
            .iter()
            .enumerate()
            .map(|(i, v)| ParamInfo::new((i + 1) as u16, v.sql_type(), v.clone()))
            .collect();

        // Execute with the connection
        if let Some(ref mut stmt) = self.statement {
            stmt.execute_with_connection(&self.connection, &param_infos)?;
        } else {
            return Err(Db2Error::Internal("No statement prepared".into()));
        }

        self.state = SessionState::Fetching;
        info!("Query executed successfully");
        Ok(())
    }

    /// Execute without parameters
    pub fn execute_no_params(&mut self) -> Db2Result<()> {
        self.execute(&[])
    }

    /// Prepare and execute in one step
    pub fn prepare_and_execute(&mut self, sql: &str, params: &[Db2Value]) -> Db2Result<()> {
        self.prepare(sql)?;
        self.execute(params)
    }

    /// Fetch the next row
    pub fn fetch_next(&mut self) -> Db2Result<Option<Vec<Db2Value>>> {
        if self.state != SessionState::Fetching {
            return Err(Db2Error::Internal("Not in fetching state".into()));
        }

        if let Some(ref mut stmt) = self.statement {
            let row = stmt.fetch_next()?;
            if row.is_none() {
                debug!("End of result set reached");
            }
            Ok(row)
        } else {
            Err(Db2Error::Internal("No statement available".into()))
        }
    }

    /// Fetch all remaining rows
    pub fn fetch_all(&mut self) -> Db2Result<Vec<Vec<Db2Value>>> {
        if self.state != SessionState::Fetching {
            return Err(Db2Error::Internal("Not in fetching state".into()));
        }

        if let Some(ref mut stmt) = self.statement {
            stmt.fetch_all()
        } else {
            Err(Db2Error::Internal("No statement available".into()))
        }
    }

    /// Get column count
    pub fn column_count(&self) -> usize {
        self.statement.as_ref().map(|s| s.column_count()).unwrap_or(0)
    }

    /// Get column descriptions
    pub fn columns(&self) -> &[db2_odbc::statement::ColumnDesc] {
        static EMPTY: &[db2_odbc::statement::ColumnDesc] = &[];
        self.statement.as_ref().map(|s| s.columns()).unwrap_or(EMPTY)
    }

    /// Get row count (for DML)
    pub fn row_count(&self) -> Db2Result<i64> {
        if let Some(ref stmt) = self.statement {
            stmt.row_count()
        } else {
            Ok(0)
        }
    }

    /// Close the current cursor (allows new query)
    pub fn close_cursor(&mut self) -> Db2Result<()> {
        if self.state == SessionState::Fetching || self.state == SessionState::Prepared {
            debug!("Closing cursor");
            if let Some(ref mut stmt) = self.statement {
                stmt.close_cursor()?;
            }
            self.state = SessionState::Ready;
        }
        Ok(())
    }

    /// Cancel the current operation
    pub fn cancel(&mut self) -> Db2Result<()> {
        if self.state == SessionState::Executing || self.state == SessionState::Fetching {
            debug!("Cancelling current operation");
            if let Some(ref stmt) = self.statement {
                stmt.cancel()?;
            }
            self.state = SessionState::Ready;
        }
        Ok(())
    }

    /// Create a savepoint
    pub fn create_savepoint(&mut self) -> Db2Result<String> {
        self.savepoint_counter += 1;
        let name = format!("fdw_sp_{}", self.savepoint_counter);
        self.connection.create_savepoint(&name)?;
        Ok(name)
    }

    /// Release a savepoint
    pub fn release_savepoint(&self, name: &str) -> Db2Result<()> {
        self.connection.release_savepoint(name)
    }

    /// Rollback to a savepoint
    pub fn rollback_to_savepoint(&self, name: &str) -> Db2Result<()> {
        self.connection.rollback_to_savepoint(name)
    }

    /// Commit the current transaction
    pub fn commit(&self) -> Db2Result<()> {
        self.connection.commit()
    }

    /// Rollback the current transaction
    pub fn rollback(&self) -> Db2Result<()> {
        self.connection.rollback()
    }

    /// Reset the session for reuse
    pub fn reset(&mut self) -> Db2Result<()> {
        self.close_cursor()?;
        self.statement = None;
        self.state = SessionState::Ready;
        Ok(())
    }

    /// Close the session
    pub fn close(&mut self) {
        debug!("Closing session");
        self.statement = None;
        self.state = SessionState::Closed;
    }

    /// Check if cursor is at end
    pub fn is_eof(&self) -> bool {
        self.statement.as_ref().map(|s| s.is_eof()).unwrap_or(true)
    }
}

impl Drop for Db2Session {
    fn drop(&mut self) {
        if self.state != SessionState::Closed {
            debug!("Session dropped without explicit close");
            // Statement cleanup happens automatically
            // Connection remains in cache for reuse
        }
    }
}

/// Session builder for convenient session creation
pub struct SessionBuilder {
    options: FdwConnectionOptions,
}

impl SessionBuilder {
    /// Start building a session with password auth
    pub fn password(
        server: impl Into<String>,
        user: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        Self {
            options: FdwConnectionOptions::with_password(server, user, password),
        }
    }

    /// Start building a session with JWT auth
    pub fn jwt(server: impl Into<String>, token: impl Into<String>) -> Self {
        Self {
            options: FdwConnectionOptions::with_jwt(server, token),
        }
    }

    /// Set NLS language
    pub fn nls_lang(mut self, nls: impl Into<String>) -> Self {
        self.options = self.options.nls_lang(nls);
        self
    }

    /// Set read-only mode
    pub fn read_only(mut self, read_only: bool) -> Self {
        self.options = self.options.read_only(read_only);
        self
    }

    /// Set prefetch size
    pub fn prefetch(mut self, prefetch: usize) -> Self {
        self.options = self.options.prefetch(prefetch);
        self
    }

    /// Build the session
    pub fn build(self) -> Db2Result<Db2Session> {
        Db2Session::new(&self.options)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_state_transitions() {
        let state = SessionState::Ready;
        assert_eq!(state, SessionState::Ready);
    }

    #[test]
    fn test_session_builder() {
        let builder = SessionBuilder::password("server", "user", "pass")
            .nls_lang("en_US.UTF-8")
            .read_only(true)
            .prefetch(100);

        assert_eq!(builder.options.server, "server");
        assert_eq!(builder.options.nls_lang, Some("en_US.UTF-8".to_string()));
        assert!(builder.options.read_only);
        assert_eq!(builder.options.prefetch, 100);
    }
}
