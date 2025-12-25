//! Connection cache and session management for DB2 FDW
//!
//! This crate provides per-backend connection caching and session management,
//! replacing the unsafe doubly-linked list implementation in C.
//!
//! ## PostgreSQL Threading Model
//!
//! PostgreSQL uses a **multi-process** architecture, NOT multi-threaded.
//! Each backend (client connection) runs in its own process with a single thread.
//! Therefore:
//! - No thread-safe data structures (DashMap, Mutex) are needed
//! - We use `thread_local!` + `RefCell<HashMap>` for state
//! - Connection cache is per-backend, not global across connections
//!
//! ## Memory Model
//!
//! - **Rust Heap**: Used for ODBC buffers, LOB data, internal state
//! - **PostgreSQL palloc**: Used for data returned to PostgreSQL (via pgrx)
//!
//! When data needs to go to PostgreSQL, pgrx handles the conversion from
//! Rust types to PostgreSQL Datums in the appropriate memory context.

pub mod pool;
pub mod session;

pub use pool::{
    get_connection, close_all_connections, close_server_connections,
    cleanup_stale_connections, get_cache_stats,
    ConnectionCache, ConnectionKey, CacheStats,
};
pub use session::{Db2Session, SessionState};

use db2_odbc::{AuthMethod, Db2ConnectionOptions};

/// FDW options that affect connection management
#[derive(Debug, Clone)]
pub struct FdwConnectionOptions {
    /// Server/DSN name
    pub server: String,
    /// Authentication method
    pub auth: AuthMethod,
    /// NLS language setting
    pub nls_lang: Option<String>,
    /// Connection is read-only
    pub read_only: bool,
    /// Prefetch row count
    pub prefetch: usize,
}

impl FdwConnectionOptions {
    /// Create new connection options with password authentication
    pub fn with_password(
        server: impl Into<String>,
        user: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        Self {
            server: server.into(),
            auth: AuthMethod::password(user, password),
            nls_lang: None,
            read_only: false,
            prefetch: db2_odbc::DEFAULT_PREFETCH,
        }
    }

    /// Create new connection options with JWT authentication
    pub fn with_jwt(server: impl Into<String>, token: impl Into<String>) -> Self {
        Self {
            server: server.into(),
            auth: AuthMethod::jwt(token),
            nls_lang: None,
            read_only: false,
            prefetch: db2_odbc::DEFAULT_PREFETCH,
        }
    }

    /// Set NLS language
    pub fn nls_lang(mut self, nls: impl Into<String>) -> Self {
        self.nls_lang = Some(nls.into());
        self
    }

    /// Set read-only mode
    pub fn read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }

    /// Set prefetch size
    pub fn prefetch(mut self, prefetch: usize) -> Self {
        self.prefetch = prefetch;
        self
    }

    /// Convert to ODBC connection options
    pub fn to_odbc_options(&self) -> Db2ConnectionOptions {
        let mut opts = match &self.auth {
            AuthMethod::Password { user, password } => {
                Db2ConnectionOptions::new(&self.server, user, password)
            }
            AuthMethod::JwtToken { token } => {
                Db2ConnectionOptions::with_jwt(&self.server, token)
            }
        };

        opts = opts.read_only(self.read_only);
        opts
    }
}
