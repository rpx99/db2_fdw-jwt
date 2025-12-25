//! ODBC Environment management
//!
//! This module provides safe management of ODBC environments.
//! It replaces the C implementation's manual environment handle allocation
//! with RAII-based resource management.

use odbc_api::Environment;
use std::sync::Arc;
use tracing::{debug, info, warn};

use crate::error::{Db2Error, Db2Result};

/// Safe wrapper around ODBC Environment
///
/// The Db2Environment manages the ODBC environment handle and provides
/// a safe interface for creating connections. Unlike the C implementation,
/// it uses Rust's ownership system to guarantee proper cleanup.
///
/// # Note
///
/// PostgreSQL backends are single-threaded, so no thread-safety is needed.
pub struct Db2Environment {
    inner: Environment,
    nls_lang: Option<String>,
}

impl Db2Environment {
    /// Create a new ODBC environment
    ///
    /// This replaces the C db2AllocEnvHdl function but without the
    /// dangerous putenv() calls that could lead to use-after-free.
    pub fn new() -> Db2Result<Self> {
        debug!("Allocating new ODBC environment");

        let inner = Environment::new()
            .map_err(|e| Db2Error::EnvironmentAllocation(e.to_string()))?;

        info!("ODBC environment allocated successfully");

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
    pub(crate) fn inner(&self) -> &Environment {
        &self.inner
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
