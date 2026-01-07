//! Per-backend connection caching
//!
//! This module replaces the unsafe doubly-linked list in the C implementation
//! with a simple HashMap for connection caching.
//!
//! ## Important: PostgreSQL Threading Model
//!
//! PostgreSQL uses a **multi-process** architecture, NOT multi-threaded.
//! Each backend (client connection) runs in its own process with a single thread.
//! Therefore:
//! - No thread-safe data structures (DashMap, Mutex) are needed
//! - RefCell provides interior mutability safely in single-threaded context
//! - The connection cache is per-backend, not global across connections

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, info, warn, instrument};

use db2_odbc::{Db2Connection, Db2Environment, Db2Result};
use crate::FdwConnectionOptions;

thread_local! {
    /// Per-backend connection cache using thread_local RefCell.
    /// This is the safe replacement for the C globals `rootenvEntry` and `rootconnEntry`.
    static CONNECTION_CACHE: RefCell<ConnectionCache> = RefCell::new(ConnectionCache::new());
}

/// Key for connection lookup
///
/// Unlike the C implementation which could have key collision issues,
/// this key properly identifies unique connections.
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct ConnectionKey {
    /// Server/DSN name
    pub server: String,
    /// User identifier (username or "jwt" for JWT auth)
    pub user_id: String,
    /// NLS language setting (affects connection behavior)
    pub nls_lang: Option<String>,
}

impl ConnectionKey {
    /// Create a key from FDW options
    pub fn from_options(options: &FdwConnectionOptions) -> Self {
        let user_id = match &options.auth {
            db2_odbc::AuthMethod::Password { user, .. } => user.clone(),
            db2_odbc::AuthMethod::JwtToken { .. } => "jwt_token_user".to_string(),
        };

        Self {
            server: options.server.clone(),
            user_id,
            nls_lang: options.nls_lang.clone(),
        }
    }
}

/// Statistics for a cached connection
#[derive(Debug)]
struct ConnectionStats {
    /// When the connection was created
    created_at: Instant,
    /// Last time the connection was used
    last_used: Instant,
    /// Number of times this connection was reused
    use_count: u64,
}

impl Default for ConnectionStats {
    fn default() -> Self {
        let now = Instant::now();
        Self {
            created_at: now,
            last_used: now,
            use_count: 0,
        }
    }
}

/// A connection entry in the cache
struct CacheEntry {
    /// The actual connection
    connection: Arc<Db2Connection>,
    /// Usage statistics
    stats: ConnectionStats,
}

impl CacheEntry {
    fn new(connection: Db2Connection) -> Self {
        Self {
            connection: Arc::new(connection),
            stats: ConnectionStats::default(),
        }
    }

    fn touch(&mut self) {
        self.stats.last_used = Instant::now();
        self.stats.use_count += 1;
    }

    fn age(&self) -> Duration {
        self.stats.created_at.elapsed()
    }

    fn idle_time(&self) -> Duration {
        self.stats.last_used.elapsed()
    }

    fn use_count(&self) -> u64 {
        self.stats.use_count
    }
}

/// Per-backend connection cache
///
/// This replaces the C implementation's doubly-linked lists with a simple HashMap.
/// Benefits:
/// - No dangling pointers
/// - No use-after-free bugs
/// - Simple, correct implementation for single-threaded context
/// - Automatic cleanup on drop
pub struct ConnectionCache {
    /// Environment cache (by NLS setting)
    environments: HashMap<Option<String>, Arc<Db2Environment>>,
    /// Connection cache
    connections: HashMap<ConnectionKey, CacheEntry>,
    /// Maximum idle time before connection is closed
    max_idle_time: Duration,
    /// Maximum connection age
    max_age: Duration,
}

impl ConnectionCache {
    /// Create a new connection cache
    pub fn new() -> Self {
        Self {
            environments: HashMap::new(),
            connections: HashMap::new(),
            max_idle_time: Duration::from_secs(300), // 5 minutes
            max_age: Duration::from_secs(3600),      // 1 hour
        }
    }

    /// Create a cache with custom timeouts
    pub fn with_timeouts(max_idle_time: Duration, max_age: Duration) -> Self {
        Self {
            environments: HashMap::new(),
            connections: HashMap::new(),
            max_idle_time,
            max_age,
        }
    }

    /// Get or create an environment for the given NLS setting
    fn get_or_create_environment(
        &mut self,
        nls_lang: Option<&str>,
    ) -> Db2Result<Arc<Db2Environment>> {
        let key = nls_lang.map(String::from);

        if let Some(env) = self.environments.get(&key) {
            return Ok(Arc::clone(env));
        }

        // Create new environment
        debug!(nls_lang = ?nls_lang, "Creating new ODBC environment");
        let env = match nls_lang {
            Some(nls) => Db2Environment::with_nls_lang(nls)?,
            None => Db2Environment::new()?,
        };

        let env = Arc::new(env);
        self.environments.insert(key, Arc::clone(&env));

        Ok(env)
    }

    /// Get or create a connection
    ///
    /// This is the safe replacement for the C db2GetSession function.
    #[instrument(skip(self, options), fields(server = %options.server))]
    pub fn get_or_create(
        &mut self,
        options: &FdwConnectionOptions,
    ) -> Db2Result<Arc<Db2Connection>> {
        let key = ConnectionKey::from_options(options);

        // Check for existing valid connection
        if let Some(entry) = self.connections.get_mut(&key) {
            // Validate connection is still good
            if entry.connection.is_valid()
                && entry.idle_time() < self.max_idle_time
                && entry.age() < self.max_age
            {
                entry.touch();
                debug!(
                    server = %options.server,
                    use_count = entry.use_count(),
                    "Reusing cached connection"
                );
                return Ok(Arc::clone(&entry.connection));
            } else {
                // Connection is stale, will be replaced below
                debug!(
                    server = %options.server,
                    idle_time = ?entry.idle_time(),
                    age = ?entry.age(),
                    "Removing stale connection"
                );
            }
        }

        // Create new connection
        info!(server = %options.server, "Creating new connection");

        let env = self.get_or_create_environment(options.nls_lang.as_deref())?;
        let odbc_options = options.to_odbc_options();
        let connection = Db2Connection::connect(&env, &odbc_options)?;

        let mut entry = CacheEntry::new(connection);
        entry.touch();

        let conn = Arc::clone(&entry.connection);
        self.connections.insert(key, entry);

        Ok(conn)
    }

    /// Close all connections
    ///
    /// This is the safe replacement for db2CloseConnections.
    #[instrument(skip(self))]
    pub fn close_all(&mut self) {
        info!(
            connection_count = self.connections.len(),
            "Closing all connections"
        );

        self.connections.clear();
        self.environments.clear();
    }

    /// Close connections to a specific server
    pub fn close_server(&mut self, server: &str) {
        info!(server = %server, "Closing connections to server");

        self.connections.retain(|key, _| key.server != server);
    }

    /// Remove stale connections
    pub fn cleanup_stale(&mut self) {
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

    /// Get cache statistics
    pub fn stats(&self) -> CacheStats {
        CacheStats {
            environment_count: self.environments.len(),
            connection_count: self.connections.len(),
            total_use_count: self
                .connections
                .values()
                .map(|e| e.use_count())
                .sum(),
        }
    }
}

impl Default for ConnectionCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Cache statistics
#[derive(Debug, Clone)]
pub struct CacheStats {
    pub environment_count: usize,
    pub connection_count: usize,
    pub total_use_count: u64,
}

// =============================================================================
// Public API - Thread-local access functions
// =============================================================================

/// Get or create a connection (thread-local access)
///
/// This provides a clean API while using thread-local storage internally.
pub fn get_connection(options: &FdwConnectionOptions) -> Db2Result<Arc<Db2Connection>> {
    CONNECTION_CACHE.with(|cache| {
        cache.borrow_mut().get_or_create(options)
    })
}

/// Close all cached connections (thread-local access)
pub fn close_all_connections() {
    CONNECTION_CACHE.with(|cache| {
        cache.borrow_mut().close_all()
    })
}

/// Close connections to a specific server (thread-local access)
pub fn close_server_connections(server: &str) {
    CONNECTION_CACHE.with(|cache| {
        cache.borrow_mut().close_server(server)
    })
}

/// Cleanup stale connections (thread-local access)
pub fn cleanup_stale_connections() {
    CONNECTION_CACHE.with(|cache| {
        cache.borrow_mut().cleanup_stale()
    })
}

/// Get cache statistics (thread-local access)
pub fn get_cache_stats() -> CacheStats {
    CONNECTION_CACHE.with(|cache| {
        cache.borrow().stats()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_connection_key() {
        let opts1 = FdwConnectionOptions::with_password("server1", "user1", "pass1");
        let opts2 = FdwConnectionOptions::with_password("server1", "user1", "pass2");
        let opts3 = FdwConnectionOptions::with_password("server1", "user2", "pass1");

        let key1 = ConnectionKey::from_options(&opts1);
        let key2 = ConnectionKey::from_options(&opts2);
        let key3 = ConnectionKey::from_options(&opts3);

        // Same server+user should have same key (password not in key)
        assert_eq!(key1, key2);
        // Different user should have different key
        assert_ne!(key1, key3);
    }

    #[test]
    fn test_cache_creation() {
        let cache = ConnectionCache::new();
        assert_eq!(cache.stats().connection_count, 0);
        assert_eq!(cache.stats().environment_count, 0);
    }

    #[test]
    fn test_jwt_connection_key() {
        let opts = FdwConnectionOptions::with_jwt("server1", "my.jwt.token");
        let key = ConnectionKey::from_options(&opts);

        assert_eq!(key.server, "server1");
        assert_eq!(key.user_id, "jwt_token_user");
    }
}
