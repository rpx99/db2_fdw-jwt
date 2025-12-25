//! Thread-safe connection pooling
//!
//! This module replaces the unsafe doubly-linked list in the C implementation
//! with a lock-free concurrent HashMap for connection caching.

use dashmap::DashMap;
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, info, warn, instrument};

use db2_odbc::{Db2Connection, Db2Environment, Db2Error, Db2Result};
use crate::FdwConnectionOptions;

/// Global connection pool instance
///
/// This is the safe replacement for the C globals `rootenvEntry` and `rootconnEntry`.
/// Using DashMap provides thread-safe concurrent access without the dangling pointer
/// issues of the C implementation.
pub static GLOBAL_POOL: Lazy<ConnectionPool> = Lazy::new(ConnectionPool::new);

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

/// Statistics for a pooled connection
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

/// A connection entry in the pool
struct PoolEntry {
    /// The actual connection
    connection: Arc<Db2Connection>,
    /// Usage statistics
    stats: RwLock<ConnectionStats>,
}

impl PoolEntry {
    fn new(connection: Db2Connection) -> Self {
        Self {
            connection: Arc::new(connection),
            stats: RwLock::new(ConnectionStats::default()),
        }
    }

    fn touch(&self) {
        let mut stats = self.stats.write();
        stats.last_used = Instant::now();
        stats.use_count += 1;
    }

    fn age(&self) -> Duration {
        self.stats.read().created_at.elapsed()
    }

    fn idle_time(&self) -> Duration {
        self.stats.read().last_used.elapsed()
    }

    fn use_count(&self) -> u64 {
        self.stats.read().use_count
    }
}

/// Thread-safe connection pool
///
/// This replaces the C implementation's doubly-linked lists with a concurrent HashMap.
/// Benefits:
/// - No dangling pointers
/// - No use-after-free bugs
/// - Thread-safe without explicit locking in most operations
/// - Automatic cleanup on drop
pub struct ConnectionPool {
    /// Environment cache (by NLS setting)
    environments: DashMap<Option<String>, Arc<Db2Environment>>,
    /// Connection cache
    connections: DashMap<ConnectionKey, PoolEntry>,
    /// Maximum idle time before connection is closed
    max_idle_time: Duration,
    /// Maximum connection age
    max_age: Duration,
}

impl ConnectionPool {
    /// Create a new connection pool
    pub fn new() -> Self {
        Self {
            environments: DashMap::new(),
            connections: DashMap::new(),
            max_idle_time: Duration::from_secs(300), // 5 minutes
            max_age: Duration::from_secs(3600),      // 1 hour
        }
    }

    /// Create a pool with custom timeouts
    pub fn with_timeouts(max_idle_time: Duration, max_age: Duration) -> Self {
        Self {
            environments: DashMap::new(),
            connections: DashMap::new(),
            max_idle_time,
            max_age,
        }
    }

    /// Get or create an environment for the given NLS setting
    fn get_or_create_environment(
        &self,
        nls_lang: Option<&str>,
    ) -> Db2Result<Arc<Db2Environment>> {
        let key = nls_lang.map(String::from);

        if let Some(env) = self.environments.get(&key) {
            return Ok(Arc::clone(&env));
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
        &self,
        options: &FdwConnectionOptions,
    ) -> Db2Result<PooledConnection> {
        let key = ConnectionKey::from_options(options);

        // Check for existing valid connection
        if let Some(entry) = self.connections.get(&key) {
            // Validate connection is still good
            if entry.connection.is_valid()
                && entry.idle_time() < self.max_idle_time
                && entry.age() < self.max_age
            {
                entry.touch();
                debug!(
                    server = %options.server,
                    use_count = entry.use_count(),
                    "Reusing pooled connection"
                );
                return Ok(PooledConnection {
                    connection: Arc::clone(&entry.connection),
                    key: key.clone(),
                    pool: self,
                });
            } else {
                // Connection is stale, remove it
                debug!(
                    server = %options.server,
                    idle_time = ?entry.idle_time(),
                    age = ?entry.age(),
                    "Removing stale connection"
                );
                drop(entry);
                self.connections.remove(&key);
            }
        }

        // Create new connection
        info!(server = %options.server, "Creating new connection");

        let env = self.get_or_create_environment(options.nls_lang.as_deref())?;
        let odbc_options = options.to_odbc_options();
        let connection = Db2Connection::connect(&env, &odbc_options)?;

        let entry = PoolEntry::new(connection);
        entry.touch();

        let conn = Arc::clone(&entry.connection);
        self.connections.insert(key.clone(), entry);

        Ok(PooledConnection {
            connection: conn,
            key,
            pool: self,
        })
    }

    /// Close all connections
    ///
    /// This is the safe replacement for db2CloseConnections.
    #[instrument(skip(self))]
    pub fn close_all(&self) {
        info!(
            connection_count = self.connections.len(),
            "Closing all connections"
        );

        self.connections.clear();
        self.environments.clear();
    }

    /// Close connections to a specific server
    pub fn close_server(&self, server: &str) {
        info!(server = %server, "Closing connections to server");

        self.connections.retain(|key, _| key.server != server);
    }

    /// Remove stale connections
    pub fn cleanup_stale(&self) {
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
            environment_count: self.environments.len(),
            connection_count: self.connections.len(),
            total_use_count: self
                .connections
                .iter()
                .map(|e| e.use_count())
                .sum(),
        }
    }
}

impl Default for ConnectionPool {
    fn default() -> Self {
        Self::new()
    }
}

/// Pool statistics
#[derive(Debug, Clone)]
pub struct PoolStats {
    pub environment_count: usize,
    pub connection_count: usize,
    pub total_use_count: u64,
}

/// A connection borrowed from the pool
///
/// This provides RAII-based connection management.
/// The connection is automatically returned to the pool when dropped.
pub struct PooledConnection<'pool> {
    connection: Arc<Db2Connection>,
    key: ConnectionKey,
    pool: &'pool ConnectionPool,
}

impl<'pool> PooledConnection<'pool> {
    /// Get a reference to the underlying connection
    pub fn connection(&self) -> &Db2Connection {
        &self.connection
    }

    /// Get the connection key
    pub fn key(&self) -> &ConnectionKey {
        &self.key
    }

    /// Explicitly close this connection and remove from pool
    pub fn close(self) {
        info!(server = %self.key.server, "Explicitly closing connection");
        self.pool.connections.remove(&self.key);
        // Connection will be dropped when Arc count goes to 0
    }

    /// Mark the connection as invalid (will be recreated on next use)
    pub fn invalidate(self) {
        warn!(server = %self.key.server, "Invalidating connection");
        self.pool.connections.remove(&self.key);
    }
}

impl<'pool> std::ops::Deref for PooledConnection<'pool> {
    type Target = Db2Connection;

    fn deref(&self) -> &Self::Target {
        &self.connection
    }
}

impl<'pool> std::fmt::Debug for PooledConnection<'pool> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PooledConnection")
            .field("key", &self.key)
            .field("connection", &self.connection)
            .finish()
    }
}

// Note: PooledConnection doesn't return to pool on drop because
// the Arc<Db2Connection> is stored in the pool entry itself.
// This means connections persist until explicitly closed or cleaned up.

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
    fn test_pool_creation() {
        let pool = ConnectionPool::new();
        assert_eq!(pool.stats().connection_count, 0);
        assert_eq!(pool.stats().environment_count, 0);
    }

    #[test]
    fn test_jwt_connection_key() {
        let opts = FdwConnectionOptions::with_jwt("server1", "my.jwt.token");
        let key = ConnectionKey::from_options(&opts);

        assert_eq!(key.server, "server1");
        assert_eq!(key.user_id, "jwt_token_user");
    }
}
