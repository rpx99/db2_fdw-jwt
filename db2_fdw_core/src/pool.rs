//! Thread-safe connection pool
//!
//! This module replaces the unsafe doubly-linked list in the C implementation
//! with a lock-free concurrent HashMap.
//!
//! The original C code had these issues:
//! - Doubly-linked list with manual pointer manipulation
//! - No validation of prev/next pointers
//! - Race conditions in multi-backend scenarios
//!
//! This implementation uses DashMap for thread-safe concurrent access.

use dashmap::DashMap;
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use std::ffi::CStr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, info};

use crate::connection::{AuthMethod, ConnectOptions, Db2ConnHandle, Db2Connection};
use crate::error::{set_last_error, Db2Error, ErrorCode};
use crate::Result;

/// Global connection pool
pub static GLOBAL_POOL: Lazy<ConnectionPool> = Lazy::new(ConnectionPool::new);

/// Key for connection lookup
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct ConnectionKey {
    pub dsn: String,
    pub user_id: String,
}

impl ConnectionKey {
    pub fn new(dsn: &str, user_id: &str) -> Self {
        Self {
            dsn: dsn.to_string(),
            user_id: user_id.to_string(),
        }
    }
}

/// Connection entry with metadata
struct PoolEntry {
    connection: Arc<Db2Connection>,
    created_at: Instant,
    last_used: RwLock<Instant>,
    use_count: std::sync::atomic::AtomicU64,
}

impl PoolEntry {
    fn new(connection: Db2Connection) -> Self {
        let now = Instant::now();
        Self {
            connection: Arc::new(connection),
            created_at: now,
            last_used: RwLock::new(now),
            use_count: std::sync::atomic::AtomicU64::new(0),
        }
    }

    fn touch(&self) {
        *self.last_used.write() = Instant::now();
        self.use_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    fn idle_time(&self) -> Duration {
        self.last_used.read().elapsed()
    }

    fn age(&self) -> Duration {
        self.created_at.elapsed()
    }

    fn use_count(&self) -> u64 {
        self.use_count.load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// Thread-safe connection pool
pub struct ConnectionPool {
    connections: DashMap<ConnectionKey, PoolEntry>,
    max_idle_time: Duration,
    max_age: Duration,
}

impl ConnectionPool {
    /// Create a new pool with default settings
    pub fn new() -> Self {
        Self {
            connections: DashMap::new(),
            max_idle_time: Duration::from_secs(300),  // 5 minutes
            max_age: Duration::from_secs(3600),       // 1 hour
        }
    }

    /// Get or create a connection
    pub fn get_or_create(&self, options: &ConnectOptions) -> Result<Arc<Db2Connection>> {
        let user_id = match &options.auth {
            AuthMethod::Password { user, .. } => user.clone(),
            AuthMethod::JwtToken { .. } => "jwt_token_user".to_string(),
        };

        let key = ConnectionKey::new(&options.dsn, &user_id);

        // Check for existing valid connection
        if let Some(entry) = self.connections.get(&key) {
            if entry.connection.is_valid()
                && entry.idle_time() < self.max_idle_time
                && entry.age() < self.max_age
            {
                entry.touch();
                debug!(
                    dsn = %options.dsn,
                    use_count = entry.use_count(),
                    "Reusing pooled connection"
                );
                return Ok(Arc::clone(&entry.connection));
            } else {
                debug!(
                    dsn = %options.dsn,
                    idle_time = ?entry.idle_time(),
                    age = ?entry.age(),
                    "Removing stale connection"
                );
                drop(entry);
                self.connections.remove(&key);
            }
        }

        // Create new connection
        info!(dsn = %options.dsn, "Creating new pooled connection");
        let connection = Db2Connection::connect(options)?;
        let entry = PoolEntry::new(connection);
        entry.touch();
        let conn = Arc::clone(&entry.connection);
        self.connections.insert(key, entry);

        Ok(conn)
    }

    /// Close all connections
    pub fn close_all(&self) {
        info!(count = self.connections.len(), "Closing all pooled connections");
        self.connections.clear();
    }

    /// Close connections to a specific DSN
    pub fn close_dsn(&self, dsn: &str) {
        info!(dsn = %dsn, "Closing connections to DSN");
        self.connections.retain(|key, _| key.dsn != dsn);
    }

    /// Clean up stale connections
    pub fn cleanup(&self) {
        let before = self.connections.len();

        self.connections.retain(|_key, entry| {
            let valid = entry.connection.is_valid()
                && entry.idle_time() < self.max_idle_time
                && entry.age() < self.max_age;

            if !valid {
                debug!(
                    idle_time = ?entry.idle_time(),
                    age = ?entry.age(),
                    "Removing stale connection"
                );
            }
            valid
        });

        let removed = before - self.connections.len();
        if removed > 0 {
            info!(removed = removed, "Cleaned up stale connections");
        }
    }

    /// Get pool statistics
    pub fn stats(&self) -> PoolStats {
        PoolStats {
            connection_count: self.connections.len(),
            total_use_count: self.connections.iter().map(|e| e.use_count()).sum(),
        }
    }
}

impl Default for ConnectionPool {
    fn default() -> Self {
        Self::new()
    }
}

/// Pool statistics
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct PoolStats {
    pub connection_count: usize,
    pub total_use_count: u64,
}

// ============================================================================
// FFI Exports
// ============================================================================

/// Get a pooled connection with password auth
///
/// # Safety
/// All string parameters must be valid null-terminated C strings.
#[no_mangle]
pub unsafe extern "C" fn db2_pool_get_password(
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

        GLOBAL_POOL.get_or_create(&options)
    });

    match result {
        Ok(Ok(conn)) => Arc::into_raw(conn) as Db2ConnHandle,
        Ok(Err(e)) => {
            set_last_error(&e.to_string());
            std::ptr::null_mut()
        }
        Err(_) => {
            set_last_error("Panic in db2_pool_get_password");
            std::ptr::null_mut()
        }
    }
}

/// Get a pooled connection with JWT auth
///
/// # Safety
/// All string parameters must be valid null-terminated C strings.
#[no_mangle]
pub unsafe extern "C" fn db2_pool_get_jwt(
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

        GLOBAL_POOL.get_or_create(&options)
    });

    match result {
        Ok(Ok(conn)) => Arc::into_raw(conn) as Db2ConnHandle,
        Ok(Err(e)) => {
            set_last_error(&e.to_string());
            std::ptr::null_mut()
        }
        Err(_) => {
            set_last_error("Panic in db2_pool_get_jwt");
            std::ptr::null_mut()
        }
    }
}

/// Release a pooled connection (returns it to the pool)
///
/// Note: For pooled connections, this just decrements the Arc count.
/// The connection stays in the pool for reuse.
///
/// # Safety
/// `handle` must be a valid handle from `db2_pool_get_*`.
#[no_mangle]
pub unsafe extern "C" fn db2_pool_release(handle: Db2ConnHandle) -> i32 {
    if handle.is_null() {
        return ErrorCode::Success as i32;
    }

    let result = std::panic::catch_unwind(|| {
        // Decrement Arc count
        let _ = unsafe { Arc::from_raw(handle as *const Db2Connection) };
    });

    match result {
        Ok(()) => ErrorCode::Success as i32,
        Err(_) => {
            set_last_error("Panic in db2_pool_release");
            ErrorCode::InternalError as i32
        }
    }
}

/// Close all pooled connections
#[no_mangle]
pub extern "C" fn db2_pool_close_all() -> i32 {
    let result = std::panic::catch_unwind(|| {
        GLOBAL_POOL.close_all();
    });

    match result {
        Ok(()) => ErrorCode::Success as i32,
        Err(_) => ErrorCode::InternalError as i32,
    }
}

/// Clean up stale connections
#[no_mangle]
pub extern "C" fn db2_pool_cleanup() -> i32 {
    let result = std::panic::catch_unwind(|| {
        GLOBAL_POOL.cleanup();
    });

    match result {
        Ok(()) => ErrorCode::Success as i32,
        Err(_) => ErrorCode::InternalError as i32,
    }
}

/// Get pool statistics
#[no_mangle]
pub extern "C" fn db2_pool_stats() -> PoolStats {
    std::panic::catch_unwind(|| GLOBAL_POOL.stats()).unwrap_or(PoolStats {
        connection_count: 0,
        total_use_count: 0,
    })
}
