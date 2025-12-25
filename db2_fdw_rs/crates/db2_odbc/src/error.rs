//! Error types for DB2 ODBC operations

use thiserror::Error;

/// Result type alias for DB2 operations
pub type Db2Result<T> = Result<T, Db2Error>;

/// Comprehensive error type for all DB2 ODBC operations
#[derive(Error, Debug)]
pub enum Db2Error {
    /// ODBC-level error with diagnostic information
    #[error("ODBC error [{sqlstate}]: {message} (native error: {native_error})")]
    Odbc {
        sqlstate: String,
        native_error: i32,
        message: String,
    },

    /// Connection establishment failed
    #[error("Failed to connect to DB2 server '{server}': {reason}")]
    ConnectionFailed {
        server: String,
        reason: String,
    },

    /// Authentication error
    #[error("Authentication failed using {method}: {reason}")]
    AuthenticationFailed {
        method: &'static str,
        reason: String,
    },

    /// JWT token is invalid or expired
    #[error("JWT token error: {0}")]
    JwtTokenError(String),

    /// Environment allocation failed
    #[error("Failed to allocate ODBC environment: {0}")]
    EnvironmentAllocation(String),

    /// Statement execution error
    #[error("Statement execution failed: {0}")]
    StatementExecution(String),

    /// Query preparation error
    #[error("Failed to prepare query: {0}")]
    QueryPreparation(String),

    /// Fetch operation error
    #[error("Fetch error: {0}")]
    FetchError(String),

    /// No more data available (not really an error, but used for control flow)
    #[error("No more data")]
    NoData,

    /// Type conversion error
    #[error("Cannot convert {from_type} to {to_type}: {reason}")]
    TypeConversion {
        from_type: String,
        to_type: String,
        reason: String,
    },

    /// Buffer overflow would occur
    #[error("Buffer overflow: required {required} bytes, available {available}")]
    BufferOverflow {
        required: usize,
        available: usize,
    },

    /// NULL value encountered where not expected
    #[error("Unexpected NULL value in column {column}")]
    UnexpectedNull {
        column: String,
    },

    /// LOB (Large Object) handling error
    #[error("LOB error: {0}")]
    LobError(String),

    /// Transaction error
    #[error("Transaction error: {0}")]
    TransactionError(String),

    /// Savepoint error
    #[error("Savepoint '{name}' error: {reason}")]
    SavepointError {
        name: String,
        reason: String,
    },

    /// Invalid handle
    #[error("Invalid ODBC handle")]
    InvalidHandle,

    /// Query timeout
    #[error("Query timeout after {seconds} seconds")]
    Timeout {
        seconds: u64,
    },

    /// Encoding error
    #[error("Character encoding error: {0}")]
    EncodingError(String),

    /// Configuration error
    #[error("Configuration error: {0}")]
    ConfigurationError(String),

    /// Internal error (should not happen)
    #[error("Internal error: {0}")]
    Internal(String),
}

impl Db2Error {
    /// Create an ODBC error from diagnostic information
    pub fn from_odbc_diag(sqlstate: impl Into<String>, native_error: i32, message: impl Into<String>) -> Self {
        Db2Error::Odbc {
            sqlstate: sqlstate.into(),
            native_error,
            message: message.into(),
        }
    }

    /// Check if this error indicates no more data (normal end of result set)
    pub fn is_no_data(&self) -> bool {
        matches!(self, Db2Error::NoData)
    }

    /// Check if this is a connection-related error
    pub fn is_connection_error(&self) -> bool {
        matches!(
            self,
            Db2Error::ConnectionFailed { .. }
                | Db2Error::AuthenticationFailed { .. }
                | Db2Error::JwtTokenError(_)
        )
    }

    /// Check if this is a timeout error
    pub fn is_timeout(&self) -> bool {
        matches!(self, Db2Error::Timeout { .. })
    }
}

/// Convert from odbc-api errors
impl From<odbc_api::Error> for Db2Error {
    fn from(err: odbc_api::Error) -> Self {
        match err {
            odbc_api::Error::Diagnostics { record, function } => {
                Db2Error::Odbc {
                    sqlstate: String::from_utf8_lossy(&record.state).to_string(),
                    native_error: record.native_error,
                    message: format!("{}: {}", function, record.message),
                }
            }
            odbc_api::Error::NoDiagnostics(function) => {
                Db2Error::Internal(format!("ODBC function {} failed without diagnostics", function))
            }
            odbc_api::Error::AbortedConnectionStringCompletion => {
                Db2Error::ConnectionFailed {
                    server: String::new(),
                    reason: "Connection string completion aborted".into(),
                }
            }
            other => Db2Error::Internal(format!("ODBC error: {:?}", other)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = Db2Error::ConnectionFailed {
            server: "mydb".into(),
            reason: "timeout".into(),
        };
        assert!(err.to_string().contains("mydb"));
        assert!(err.to_string().contains("timeout"));
    }

    #[test]
    fn test_is_connection_error() {
        let conn_err = Db2Error::ConnectionFailed {
            server: "test".into(),
            reason: "test".into(),
        };
        assert!(conn_err.is_connection_error());

        let other_err = Db2Error::NoData;
        assert!(!other_err.is_connection_error());
    }
}
