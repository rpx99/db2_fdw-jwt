//! Error types for DB2 FDW Core

use std::ffi::CString;
use thiserror::Error;

/// Error codes returned to C code
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    Success = 0,
    ConnectionFailed = -1,
    InvalidHandle = -2,
    QueryFailed = -3,
    OutOfMemory = -4,
    InvalidParameter = -5,
    Timeout = -6,
    NotConnected = -7,
    LobError = -8,
    EncodingError = -9,
    InternalError = -99,
}

/// Main error type
#[derive(Error, Debug)]
pub enum Db2Error {
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),

    #[error("Invalid handle")]
    InvalidHandle,

    #[error("Query failed: {0}")]
    QueryFailed(String),

    #[error("Out of memory")]
    OutOfMemory,

    #[error("Invalid parameter: {0}")]
    InvalidParameter(String),

    #[error("Connection timeout")]
    Timeout,

    #[error("Not connected")]
    NotConnected,

    #[error("LOB error: {0}")]
    LobError(String),

    #[error("Encoding error: {0}")]
    EncodingError(String),

    #[error("ODBC error: {0}")]
    OdbcError(String),

    #[error("Internal error: {0}")]
    Internal(String),
}

impl Db2Error {
    /// Convert to C error code
    pub fn to_code(&self) -> ErrorCode {
        match self {
            Db2Error::ConnectionFailed(_) => ErrorCode::ConnectionFailed,
            Db2Error::InvalidHandle => ErrorCode::InvalidHandle,
            Db2Error::QueryFailed(_) => ErrorCode::QueryFailed,
            Db2Error::OutOfMemory => ErrorCode::OutOfMemory,
            Db2Error::InvalidParameter(_) => ErrorCode::InvalidParameter,
            Db2Error::Timeout => ErrorCode::Timeout,
            Db2Error::NotConnected => ErrorCode::NotConnected,
            Db2Error::LobError(_) => ErrorCode::LobError,
            Db2Error::EncodingError(_) => ErrorCode::EncodingError,
            Db2Error::OdbcError(_) | Db2Error::Internal(_) => ErrorCode::InternalError,
        }
    }
}

impl From<odbc_api::Error> for Db2Error {
    fn from(err: odbc_api::Error) -> Self {
        Db2Error::OdbcError(err.to_string())
    }
}

/// Thread-local storage for last error message
thread_local! {
    static LAST_ERROR: std::cell::RefCell<Option<CString>> = const { std::cell::RefCell::new(None) };
}

/// Set the last error message (for retrieval by C code)
pub fn set_last_error(msg: &str) {
    LAST_ERROR.with(|e| {
        *e.borrow_mut() = CString::new(msg).ok();
    });
}

/// Get the last error message
///
/// # Safety
/// Returns a pointer valid until the next error occurs on this thread.
#[no_mangle]
pub extern "C" fn db2_core_last_error() -> *const libc::c_char {
    LAST_ERROR.with(|e| {
        e.borrow()
            .as_ref()
            .map(|s| s.as_ptr())
            .unwrap_or(std::ptr::null())
    })
}
