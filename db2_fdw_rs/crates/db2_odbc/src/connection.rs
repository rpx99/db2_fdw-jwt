//! DB2 ODBC Connection management
//!
//! This module provides safe connection handling with support for both
//! traditional username/password authentication and JWT token authentication.
//!
//! # Implementation
//!
//! Uses the odbc-api crate for safe ODBC bindings. All memory is managed
//! by Rust's ownership system - no manual cleanup required.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use parking_lot::Mutex;
use tracing::{debug, info, warn, instrument};

use odbc_api::{Connection, ConnectionOptions, CursorImpl};
use odbc_api::handles::StatementImpl;

use crate::environment::Db2Environment;
use crate::error::{Db2Error, Db2Result};

/// Authentication method for DB2 connections
#[derive(Debug, Clone)]
pub enum AuthMethod {
    /// Traditional username and password authentication
    Password {
        user: String,
        password: String,
    },
    /// JWT token authentication (DB2 11.5.4+)
    JwtToken {
        token: String,
    },
}

impl AuthMethod {
    /// Create password authentication
    pub fn password(user: impl Into<String>, password: impl Into<String>) -> Self {
        AuthMethod::Password {
            user: user.into(),
            password: password.into(),
        }
    }

    /// Create JWT token authentication
    pub fn jwt(token: impl Into<String>) -> Self {
        AuthMethod::JwtToken {
            token: token.into(),
        }
    }

    /// Get the authentication method name for error messages
    pub fn method_name(&self) -> &'static str {
        match self {
            AuthMethod::Password { .. } => "password",
            AuthMethod::JwtToken { .. } => "JWT token",
        }
    }

    /// Check if this is JWT authentication
    pub fn is_jwt(&self) -> bool {
        matches!(self, AuthMethod::JwtToken { .. })
    }
}

/// Connection options for DB2
#[derive(Debug, Clone)]
pub struct Db2ConnectionOptions {
    /// Server/DSN name
    pub server: String,
    /// Authentication method
    pub auth: AuthMethod,
    /// Connection timeout in seconds (0 = no timeout)
    pub timeout: u32,
    /// Auto-commit mode
    pub auto_commit: bool,
    /// Read-only mode
    pub read_only: bool,
}

impl Db2ConnectionOptions {
    /// Create new connection options with password auth
    pub fn new(server: impl Into<String>, user: impl Into<String>, password: impl Into<String>) -> Self {
        Self {
            server: server.into(),
            auth: AuthMethod::password(user, password),
            timeout: 0,
            auto_commit: false,
            read_only: false,
        }
    }

    /// Create new connection options with JWT auth
    pub fn with_jwt(server: impl Into<String>, token: impl Into<String>) -> Self {
        Self {
            server: server.into(),
            auth: AuthMethod::jwt(token),
            timeout: 0,
            auto_commit: false,
            read_only: false,
        }
    }

    /// Set connection timeout
    pub fn timeout(mut self, seconds: u32) -> Self {
        self.timeout = seconds;
        self
    }

    /// Enable auto-commit
    pub fn auto_commit(mut self, enabled: bool) -> Self {
        self.auto_commit = enabled;
        self
    }

    /// Enable read-only mode
    pub fn read_only(mut self, enabled: bool) -> Self {
        self.read_only = enabled;
        self
    }

    /// Build the ODBC connection string
    pub fn build_connection_string(&self) -> String {
        let mut conn_str = format!("DSN={}", self.server);

        match &self.auth {
            AuthMethod::Password { user, password } => {
                conn_str.push_str(&format!(";UID={};PWD={}", user, password));
            }
            AuthMethod::JwtToken { token } => {
                // JWT authentication for DB2 11.5.4+
                conn_str.push_str(&format!(
                    ";AUTHENTICATION=TOKEN;ACCESSTOKEN={};ACCESSTOKENTYPE=JWT",
                    token
                ));
            }
        }

        if self.timeout > 0 {
            conn_str.push_str(&format!(";CONNECTTIMEOUT={}", self.timeout));
        }

        conn_str
    }
}

/// Inner connection state
struct ConnectionInner<'env> {
    connection: Connection<'env>,
}

/// Safe wrapper around an ODBC connection to DB2
///
/// This replaces the C db2AllocConnHdl with a safe, RAII-based connection.
/// Memory safety is guaranteed through Rust's ownership system.
pub struct Db2Connection {
    /// Connection identifier for logging/debugging
    id: u32,
    /// Server name for this connection
    server: String,
    /// Current transaction nesting level
    xact_level: AtomicU32,
    /// Whether connection is read-only
    read_only: bool,
    /// Connection string for reference
    connection_string: String,
    /// The actual ODBC connection (None if using stubs)
    #[cfg(feature = "real_odbc")]
    inner: Option<Arc<Mutex<ConnectionInner<'static>>>>,
}

// SAFETY: We protect access to the connection through Arc<Mutex>
unsafe impl Send for Db2Connection {}
unsafe impl Sync for Db2Connection {}

/// Global connection ID counter
static NEXT_CONNECTION_ID: AtomicU32 = AtomicU32::new(1);

impl Db2Connection {
    /// Connect to DB2 using the specified options
    ///
    /// This is the safe replacement for the C db2AllocConnHdl function.
    /// It handles both password and JWT authentication securely.
    #[instrument(skip(env, options), fields(server = %options.server))]
    pub fn connect(env: &Db2Environment, options: &Db2ConnectionOptions) -> Db2Result<Self> {
        let id = NEXT_CONNECTION_ID.fetch_add(1, Ordering::Relaxed);
        debug!(connection_id = id, "Connecting to DB2");

        let conn_str = options.build_connection_string();

        #[cfg(feature = "real_odbc")]
        {
            // Real ODBC connection using odbc-api
            let connection = env.inner()
                .connect_with_connection_string(&conn_str, ConnectionOptions::default())
                .map_err(|e| {
                    warn!(connection_id = id, error = %e, "Connection failed");
                    Db2Error::ConnectionFailed {
                        server: options.server.clone(),
                        reason: e.to_string(),
                    }
                })?;

            info!(connection_id = id, server = %options.server, "Connected to DB2");

            Ok(Self {
                id,
                server: options.server.clone(),
                xact_level: AtomicU32::new(0),
                read_only: options.read_only,
                connection_string: conn_str,
                inner: Some(Arc::new(Mutex::new(ConnectionInner { connection }))),
            })
        }

        #[cfg(not(feature = "real_odbc"))]
        {
            // Stub mode for testing without ODBC
            info!(connection_id = id, server = %options.server, "Connection initialized (stub mode)");

            Ok(Self {
                id,
                server: options.server.clone(),
                xact_level: AtomicU32::new(0),
                read_only: options.read_only,
                connection_string: conn_str,
            })
        }
    }

    /// Connect with password authentication (convenience method)
    pub fn connect_with_password(
        env: &Db2Environment,
        server: &str,
        user: &str,
        password: &str,
    ) -> Db2Result<Self> {
        let options = Db2ConnectionOptions::new(server, user, password);
        Self::connect(env, &options)
    }

    /// Connect with JWT token authentication (convenience method)
    pub fn connect_with_jwt(
        env: &Db2Environment,
        server: &str,
        token: &str,
    ) -> Db2Result<Self> {
        let options = Db2ConnectionOptions::with_jwt(server, token);
        Self::connect(env, &options)
    }

    /// Get the connection ID
    pub fn id(&self) -> u32 {
        self.id
    }

    /// Get the server name
    pub fn server(&self) -> &str {
        &self.server
    }

    /// Check if connection is read-only
    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// Get the current transaction nesting level
    pub fn xact_level(&self) -> u32 {
        self.xact_level.load(Ordering::Acquire)
    }

    /// Increment transaction nesting level
    pub fn begin_transaction(&self) -> u32 {
        let new_level = self.xact_level.fetch_add(1, Ordering::AcqRel) + 1;
        debug!(connection_id = self.id, xact_level = new_level, "Transaction started");
        new_level
    }

    /// Decrement transaction nesting level
    pub fn end_transaction(&self) -> u32 {
        let new_level = self.xact_level.fetch_sub(1, Ordering::AcqRel).saturating_sub(1);
        debug!(connection_id = self.id, xact_level = new_level, "Transaction ended");
        new_level
    }

    /// Commit the current transaction
    #[instrument(skip(self), fields(connection_id = self.id))]
    pub fn commit(&self) -> Db2Result<()> {
        debug!("Committing transaction");

        #[cfg(feature = "real_odbc")]
        if let Some(ref inner) = self.inner {
            let guard = inner.lock();
            guard.connection.commit().map_err(|e| Db2Error::TransactionError(e.to_string()))?;
        }

        Ok(())
    }

    /// Rollback the current transaction
    #[instrument(skip(self), fields(connection_id = self.id))]
    pub fn rollback(&self) -> Db2Result<()> {
        debug!("Rolling back transaction");

        #[cfg(feature = "real_odbc")]
        if let Some(ref inner) = self.inner {
            let guard = inner.lock();
            guard.connection.rollback().map_err(|e| Db2Error::TransactionError(e.to_string()))?;
        }

        Ok(())
    }

    /// Create a savepoint
    ///
    /// Note: DB2 requires "ON ROLLBACK RETAIN CURSORS" clause for savepoints
    pub fn create_savepoint(&self, name: &str) -> Db2Result<()> {
        let sql = format!("SAVEPOINT {} ON ROLLBACK RETAIN CURSORS", name);
        debug!(connection_id = self.id, savepoint = name, "Creating savepoint");
        self.execute_immediate(&sql)?;
        Ok(())
    }

    /// Release a savepoint
    pub fn release_savepoint(&self, name: &str) -> Db2Result<()> {
        let sql = format!("RELEASE SAVEPOINT {}", name);
        debug!(connection_id = self.id, savepoint = name, "Releasing savepoint");
        self.execute_immediate(&sql)?;
        Ok(())
    }

    /// Rollback to a savepoint
    pub fn rollback_to_savepoint(&self, name: &str) -> Db2Result<()> {
        let sql = format!("ROLLBACK TO SAVEPOINT {}", name);
        debug!(connection_id = self.id, savepoint = name, "Rolling back to savepoint");
        self.execute_immediate(&sql)?;
        Ok(())
    }

    /// Execute a SQL statement immediately (no result set)
    pub fn execute_immediate(&self, sql: &str) -> Db2Result<()> {
        debug!(connection_id = self.id, sql = %sql, "Executing immediate");

        #[cfg(feature = "real_odbc")]
        if let Some(ref inner) = self.inner {
            let guard = inner.lock();
            guard.connection
                .execute(sql, ())
                .map_err(|e| Db2Error::StatementExecution(e.to_string()))?;
        }

        Ok(())
    }

    /// Execute a query with a callback to process results
    ///
    /// This provides safe, scoped access to the cursor without lifetime issues.
    /// The callback must process all results before returning, as the cursor
    /// is tied to the connection guard's lifetime.
    #[cfg(feature = "real_odbc")]
    pub fn execute_query<F, R>(&self, sql: &str, f: F) -> Db2Result<R>
    where
        F: FnOnce(CursorImpl<StatementImpl<'_>>) -> Db2Result<R>,
    {
        debug!(connection_id = self.id, sql = %sql, "Executing query");

        if let Some(ref inner) = self.inner {
            let guard = inner.lock();
            // Execute and process within the same scope to satisfy lifetimes
            let result = guard.connection.execute(sql, ());
            let cursor = match result {
                Ok(Some(cursor)) => cursor,
                Ok(None) => return Err(Db2Error::StatementExecution("No result set returned".into())),
                Err(e) => return Err(Db2Error::StatementExecution(e.to_string())),
            };
            f(cursor)
        } else {
            Err(Db2Error::Internal("Not connected".into()))
        }
    }

    /// Execute a query (stub mode - always returns error)
    #[cfg(not(feature = "real_odbc"))]
    pub fn execute_query<F, R>(&self, _sql: &str, _f: F) -> Db2Result<R>
    where
        F: FnOnce(()) -> Db2Result<R>,
    {
        Err(Db2Error::Internal("Not connected (stub mode)".into()))
    }

    /// Execute an update statement (INSERT, UPDATE, DELETE) with row count callback
    #[cfg(feature = "real_odbc")]
    pub fn execute_update<F>(&self, sql: &str, f: F) -> Db2Result<()>
    where
        F: FnOnce(i64) -> Db2Result<()>,
    {
        debug!(connection_id = self.id, sql = %sql, "Executing update");

        if let Some(ref inner) = self.inner {
            let guard = inner.lock();
            // Execute and handle result within the same scope
            let result = guard.connection.execute(sql, ());
            match result {
                Ok(Some(_cursor)) => {
                    // For DML, the row count is returned via ODBC, but we can't easily access it
                    // from this cursor type. Return 0 for now.
                    // TODO: Use preallocated statement to get affected row count
                    f(0)
                }
                Ok(None) => {
                    // No cursor returned, assume success with 0 rows
                    f(0)
                }
                Err(e) => Err(Db2Error::StatementExecution(e.to_string())),
            }
        } else {
            Err(Db2Error::Internal("Not connected".into()))
        }
    }

    /// Execute an update statement (stub mode)
    #[cfg(not(feature = "real_odbc"))]
    pub fn execute_update<F>(&self, _sql: &str, f: F) -> Db2Result<()>
    where
        F: FnOnce(i64) -> Db2Result<()>,
    {
        debug!(connection_id = self.id, "Update in stub mode");
        f(0)
    }

    /// Get the connection string (for debugging)
    pub(crate) fn connection_string(&self) -> &str {
        &self.connection_string
    }

    /// Check if the connection is still valid
    pub fn is_valid(&self) -> bool {
        #[cfg(feature = "real_odbc")]
        if let Some(ref inner) = self.inner {
            let guard = inner.lock();
            return guard.connection.is_dead().map(|dead| !dead).unwrap_or(false);
        }

        true
    }
}

impl std::fmt::Debug for Db2Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Db2Connection")
            .field("id", &self.id)
            .field("server", &self.server)
            .field("xact_level", &self.xact_level.load(Ordering::Relaxed))
            .field("read_only", &self.read_only)
            .finish()
    }
}

impl Drop for Db2Connection {
    fn drop(&mut self) {
        debug!(connection_id = self.id, server = %self.server, "Closing connection");
        // RAII: Connection is automatically closed when dropped
        // No manual cleanup needed - this is a huge improvement over C!
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_connection_options_password() {
        let opts = Db2ConnectionOptions::new("mydb", "user", "pass");
        let conn_str = opts.build_connection_string();
        assert!(conn_str.contains("DSN=mydb"));
        assert!(conn_str.contains("UID=user"));
        assert!(conn_str.contains("PWD=pass"));
    }

    #[test]
    fn test_connection_options_jwt() {
        let opts = Db2ConnectionOptions::with_jwt("mydb", "my.jwt.token");
        let conn_str = opts.build_connection_string();
        assert!(conn_str.contains("DSN=mydb"));
        assert!(conn_str.contains("AUTHENTICATION=TOKEN"));
        assert!(conn_str.contains("ACCESSTOKEN=my.jwt.token"));
        assert!(conn_str.contains("ACCESSTOKENTYPE=JWT"));
    }

    #[test]
    fn test_auth_method() {
        let pwd_auth = AuthMethod::password("user", "pass");
        assert!(!pwd_auth.is_jwt());
        assert_eq!(pwd_auth.method_name(), "password");

        let jwt_auth = AuthMethod::jwt("token");
        assert!(jwt_auth.is_jwt());
        assert_eq!(jwt_auth.method_name(), "JWT token");
    }
}
