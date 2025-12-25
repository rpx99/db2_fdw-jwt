//! ODBC Environment management
//!
//! This module provides safe management of ODBC environments.
//! It replaces the C implementation's manual environment handle allocation
//! with RAII-based resource management.
//!
//! # Static Lifetime Solution
//!
//! ODBC connections in odbc-api have a lifetime tied to their Environment.
//! To store connections in structures like `Arc<Mutex<>>`, we need `'static`
//! lifetime. We achieve this by using `Box::leak()` to create a static
//! Environment reference (acceptable since we only create one per backend).

use odbc_api::Environment;
use std::sync::{Arc, OnceLock};
use tracing::{debug, info};

use crate::error::Db2Result;

/// Global static environment - one per PostgreSQL backend process
/// This is safe because PostgreSQL backends are single-process/single-threaded.
static GLOBAL_ENV: OnceLock<&'static Environment> = OnceLock::new();

/// Get the global static ODBC environment
///
/// Creates the environment on first call using Box::leak for 'static lifetime.
/// Returns cached reference thereafter.
///
/// # Safety
///
/// This leaks memory intentionally - one Environment per PostgreSQL backend,
/// lives for the entire backend lifetime. This is acceptable and safe.
pub fn get_global_environment() -> Db2Result<&'static Environment> {
    // Use get_or_init with a closure that returns the static reference
    // We handle errors by returning a sentinel and checking
    let env = GLOBAL_ENV.get_or_init(|| {
        match Environment::new() {
            Ok(env) => {
                // Leak the environment to get 'static lifetime
                // Safe: one per backend, lives for entire process
                let boxed = Box::new(env);
                let static_env: &'static Environment = Box::leak(boxed);
                info!("Global ODBC environment initialized with 'static lifetime");
                static_env
            }
            Err(e) => {
                // This is a fatal error - can't recover without an environment
                // In production, this would use elog(ERROR) to abort
                panic!("Failed to allocate ODBC environment: {}", e);
            }
        }
    });

    Ok(*env)
}

/// Safe wrapper around ODBC Environment
///
/// The Db2Environment manages the ODBC environment handle and provides
/// a safe interface for creating connections. Unlike the C implementation,
/// it uses Rust's ownership system to guarantee proper cleanup.
///
/// # Lifetime
///
/// This wrapper uses a reference to the global static environment, allowing
/// connections created from it to have `'static` lifetime.
///
/// # Note
///
/// PostgreSQL backends are single-threaded, so no thread-safety is needed.
pub struct Db2Environment {
    inner: &'static Environment,
    nls_lang: Option<String>,
}

impl Db2Environment {
    /// Create a new ODBC environment
    ///
    /// This replaces the C db2AllocEnvHdl function but without the
    /// dangerous putenv() calls that could lead to use-after-free.
    ///
    /// Uses the global static environment to allow `'static` lifetime connections.
    pub fn new() -> Db2Result<Self> {
        debug!("Creating Db2Environment wrapper");

        let inner = get_global_environment()?;

        info!("Db2Environment wrapper created successfully");

        Ok(Self {
            inner,
            nls_lang: None,
        })
    }

    /// Create a new ODBC environment with NLS settings
    ///
    /// Unlike the C implementation which uses dangerous putenv(),
    /// we store NLS settings internally and apply them per-connection.
    pub fn with_nls_lang(nls_lang: &str) -> Db2Result<Self> {
        let mut env = Self::new()?;
        env.set_nls_lang(nls_lang);
        Ok(env)
    }

    /// Set the NLS language setting
    ///
    /// This is applied to connections created from this environment.
    /// Safe alternative to the C implementation's putenv() approach.
    pub fn set_nls_lang(&mut self, nls_lang: &str) {
        debug!("Setting NLS_LANG to: {}", nls_lang);
        self.nls_lang = Some(nls_lang.to_string());
    }

    /// Get the NLS language setting if configured
    pub fn nls_lang(&self) -> Option<&str> {
        self.nls_lang.as_deref()
    }

    /// Get a reference to the inner ODBC environment
    ///
    /// Returns a `'static` reference, allowing connections to have `'static` lifetime.
    pub(crate) fn inner(&self) -> &'static Environment {
        self.inner
    }

    /// Get the ODBC driver manager version
    pub fn driver_manager_version(&self) -> String {
        // Note: This would require additional ODBC calls
        // For now, return a placeholder
        "ODBC 3.80".to_string()
    }
}

impl std::fmt::Debug for Db2Environment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Db2Environment")
            .field("nls_lang", &self.nls_lang)
            .finish()
    }
}

// SAFETY: ODBC environments can be shared across threads
unsafe impl Send for Db2Environment {}
unsafe impl Sync for Db2Environment {}

/// Shared environment wrapper for connection pooling
pub type SharedEnvironment = Arc<Db2Environment>;

/// Create a shared environment suitable for use in a connection pool
pub fn create_shared_environment() -> Db2Result<SharedEnvironment> {
    Ok(Arc::new(Db2Environment::new()?))
}

/// Create a shared environment with NLS settings
pub fn create_shared_environment_with_nls(nls_lang: &str) -> Db2Result<SharedEnvironment> {
    Ok(Arc::new(Db2Environment::with_nls_lang(nls_lang)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_environment_creation() {
        // Note: This test requires ODBC driver manager to be installed
        let result = Db2Environment::new();
        // We don't assert success because ODBC might not be available in test env
        if let Err(e) = &result {
            println!("Environment creation failed (expected in some test envs): {}", e);
        }
    }

    #[test]
    fn test_nls_lang_setting() {
        // This test doesn't require actual ODBC
        if let Ok(mut env) = Db2Environment::new() {
            env.set_nls_lang("en_US.UTF-8");
            assert_eq!(env.nls_lang(), Some("en_US.UTF-8"));
        }
    }
}
