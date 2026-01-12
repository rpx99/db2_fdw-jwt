//! Safe DB2 connection handling
//!
//! This module provides memory-safe connection management with support
//! for both password and JWT authentication.

use std::ffi::CStr;
use std::sync::atomic::{AtomicU64, Ordering};
use tracing::{debug, info, instrument, warn};

use odbc_api::{Connection, ConnectionOptions};

use crate::environment::get_environment;
use crate::error::{set_last_error, Db2Error, ErrorCode};
use crate::Result;

/// Connection ID counter
static NEXT_CONN_ID: AtomicU64 = AtomicU64::new(1);

/// Authentication method
#[derive(Debug, Clone)]
pub enum AuthMethod {
    Password { user: String, password: String },
    JwtToken { token: String },
}

/// Connection options
#[derive(Debug, Clone)]
pub struct ConnectOptions {
    pub dsn: String,
    pub auth: AuthMethod,
    pub timeout_seconds: u32,
}

impl ConnectOptions {
    /// Build ODBC connection string
    pub fn connection_string(&self) -> String {
        let mut conn_str = format!("DSN={}", self.dsn);

        match &self.auth {
            AuthMethod::Password { user, password } => {
                conn_str.push_str(&format!(";UID={};PWD={}", user, password));
            }
            AuthMethod::JwtToken { token } => {
                conn_str.push_str(&format!(
                    ";AUTHENTICATION=TOKEN;ACCESSTOKEN={};ACCESSTOKENTYPE=JWT",
                    token
                ));
            }
        }

        if self.timeout_seconds > 0 {
            conn_str.push_str(&format!(";CONNECTTIMEOUT={}", self.timeout_seconds));
        }

        conn_str
    }
}

/// Safe wrapper around ODBC connection
pub struct Db2Connection {
    id: u64,
    inner: Connection<'static>,
    dsn: String,
    xact_level: AtomicU64,
}

// SAFETY: ODBC connections are thread-safe when used correctly.
// We ensure single-threaded access through the pool.
unsafe impl Send for Db2Connection {}
unsafe impl Sync for Db2Connection {}

impl Db2Connection {
    /// Create a new connection
    #[instrument(skip(options), fields(dsn = %options.dsn))]
    pub fn connect(options: &ConnectOptions) -> Result<Self> {
        let id = NEXT_CONN_ID.fetch_add(1, Ordering::Relaxed);
        debug!(connection_id = id, "Connecting to DB2");

        let env = get_environment()?;
        let conn_str = options.connection_string();

        let inner = env
            .connect_with_connection_string(&conn_str, ConnectionOptions::default())
            .map_err(|e| {
                warn!(connection_id = id, error = %e, "Connection failed");
                Db2Error::ConnectionFailed(e.to_string())
            })?;

        info!(connection_id = id, dsn = %options.dsn, "Connected successfully");

        Ok(Self {
            id,
            inner,
            dsn: options.dsn.clone(),
            xact_level: AtomicU64::new(0),
        })
    }

    /// Get connection ID
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Get DSN
    pub fn dsn(&self) -> &str {
        &self.dsn
    }

    /// Get reference to inner ODBC connection
    pub fn inner(&self) -> &Connection<'static> {
        &self.inner
    }

    /// Check if connection is valid
    pub fn is_valid(&self) -> bool {
        self.inner.is_dead().map(|dead| !dead).unwrap_or(false)
    }

    /// Get transaction nesting level
    pub fn xact_level(&self) -> u64 {
        self.xact_level.load(Ordering::Acquire)
    }

    /// Increment transaction level
    pub fn begin_xact(&self) -> u64 {
        self.xact_level.fetch_add(1, Ordering::AcqRel) + 1
    }

    /// Decrement transaction level
    pub fn end_xact(&self) -> u64 {
        self.xact_level.fetch_sub(1, Ordering::AcqRel).saturating_sub(1)
    }
}

impl Drop for Db2Connection {
    fn drop(&mut self) {
        debug!(connection_id = self.id, dsn = %self.dsn, "Closing connection");
        // RAII: Connection is automatically closed by odbc-api
    }
}

impl std::fmt::Debug for Db2Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Db2Connection")
            .field("id", &self.id)
            .field("dsn", &self.dsn)
            .field("xact_level", &self.xact_level.load(Ordering::Relaxed))
            .finish()
    }
}

// ============================================================================
// FFI Exports
// ============================================================================

/// Opaque connection handle for C code
pub type Db2ConnHandle = *mut Db2Connection;

/// Connect with password authentication
///
/// # Safety
/// All string parameters must be valid null-terminated C strings.
/// Returns a handle that must be freed with `db2_conn_close`.
#[no_mangle]
pub unsafe extern "C" fn db2_conn_connect_password(
    dsn: *const libc::c_char,
    user: *const libc::c_char,
    password: *const libc::c_char,
    timeout_seconds: u32,
) -> Db2ConnHandle {
    if dsn.is_null() || user.is_null() || password.is_null() {
        set_last_error("Null parameter");
        return std::ptr::null_mut();
    }

    let result = std::panic::catch_unwind(|| {
        let dsn = unsafe { CStr::from_ptr(dsn) }.to_str().map_err(|_| Db2Error::EncodingError("Invalid DSN".into()))?;
        let user = unsafe { CStr::from_ptr(user) }.to_str().map_err(|_| Db2Error::EncodingError("Invalid user".into()))?;
        let password = unsafe { CStr::from_ptr(password) }.to_str().map_err(|_| Db2Error::EncodingError("Invalid password".into()))?;

        let options = ConnectOptions {
            dsn: dsn.to_string(),
            auth: AuthMethod::Password {
                user: user.to_string(),
                password: password.to_string(),
            },
            timeout_seconds,
        };

        Db2Connection::connect(&options)
    });

    match result {
        Ok(Ok(conn)) => Box::into_raw(Box::new(conn)),
        Ok(Err(e)) => {
            set_last_error(&e.to_string());
            std::ptr::null_mut()
        }
        Err(_) => {
            set_last_error("Panic in db2_conn_connect_password");
            std::ptr::null_mut()
        }
    }
}

/// Connect with JWT token authentication
///
/// # Safety
/// All string parameters must be valid null-terminated C strings.
/// Returns a handle that must be freed with `db2_conn_close`.
#[no_mangle]
pub unsafe extern "C" fn db2_conn_connect_jwt(
    dsn: *const libc::c_char,
    token: *const libc::c_char,
    timeout_seconds: u32,
) -> Db2ConnHandle {
    if dsn.is_null() || token.is_null() {
        set_last_error("Null parameter");
        return std::ptr::null_mut();
    }

    let result = std::panic::catch_unwind(|| {
        let dsn = unsafe { CStr::from_ptr(dsn) }.to_str().map_err(|_| Db2Error::EncodingError("Invalid DSN".into()))?;
        let token = unsafe { CStr::from_ptr(token) }.to_str().map_err(|_| Db2Error::EncodingError("Invalid token".into()))?;

        let options = ConnectOptions {
            dsn: dsn.to_string(),
            auth: AuthMethod::JwtToken {
                token: token.to_string(),
            },
            timeout_seconds,
        };

        Db2Connection::connect(&options)
    });

    match result {
        Ok(Ok(conn)) => Box::into_raw(Box::new(conn)),
        Ok(Err(e)) => {
            set_last_error(&e.to_string());
            std::ptr::null_mut()
        }
        Err(_) => {
            set_last_error("Panic in db2_conn_connect_jwt");
            std::ptr::null_mut()
        }
    }
}

/// Close a connection
///
/// # Safety
/// `handle` must be a valid handle from `db2_conn_connect_*` or null.
#[no_mangle]
pub unsafe extern "C" fn db2_conn_close(handle: Db2ConnHandle) -> i32 {
    if handle.is_null() {
        return ErrorCode::Success as i32;
    }

    let result = std::panic::catch_unwind(|| {
        let _ = unsafe { Box::from_raw(handle) };
        // Connection is dropped here
    });

    match result {
        Ok(()) => ErrorCode::Success as i32,
        Err(_) => {
            set_last_error("Panic in db2_conn_close");
            ErrorCode::InternalError as i32
        }
    }
}

/// Check if connection is valid
///
/// # Safety
/// `handle` must be a valid connection handle.
#[no_mangle]
pub unsafe extern "C" fn db2_conn_is_valid(handle: Db2ConnHandle) -> i32 {
    if handle.is_null() {
        return 0;
    }

    let result = std::panic::catch_unwind(|| {
        let conn = unsafe { &*handle };
        conn.is_valid()
    });

    match result {
        Ok(true) => 1,
        Ok(false) => 0,
        Err(_) => 0,
    }
}

/// Get connection ID
///
/// # Safety
/// `handle` must be a valid connection handle.
#[no_mangle]
pub unsafe extern "C" fn db2_conn_get_id(handle: Db2ConnHandle) -> u64 {
    if handle.is_null() {
        return 0;
    }

    std::panic::catch_unwind(|| {
        let conn = unsafe { &*handle };
        conn.id()
    })
    .unwrap_or(0)
}
