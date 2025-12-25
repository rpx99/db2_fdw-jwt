//! DB2 FDW Core - Safe Rust implementation for critical FDW components
//!
//! This library replaces the crash-prone C code with memory-safe Rust.
//! It exports C-compatible functions that can be called from the existing FDW.
//!
//! # Safety
//!
//! All exported functions use `catch_unwind` to prevent panics from
//! crossing the FFI boundary, which would be undefined behavior.

#![allow(clippy::missing_safety_doc)]

pub mod connection;
pub mod environment;
pub mod error;
pub mod ffi;
pub mod lob;
pub mod pool;

use std::panic::catch_unwind;

/// Result type alias for this crate
pub type Result<T> = std::result::Result<T, error::Db2Error>;

/// Initialize the library (call once at startup)
///
/// # Safety
/// Safe to call multiple times, only initializes once.
#[no_mangle]
pub extern "C" fn db2_core_init() -> i32 {
    let result = catch_unwind(|| {
        #[cfg(feature = "logging")]
        {
            use tracing_subscriber::{fmt, EnvFilter};
            let _ = fmt()
                .with_env_filter(EnvFilter::from_default_env())
                .try_init();
        }
        0
    });

    result.unwrap_or(-1)
}

/// Shutdown the library (call at PostgreSQL shutdown)
///
/// # Safety
/// Closes all connections and frees resources.
#[no_mangle]
pub extern "C" fn db2_core_shutdown() -> i32 {
    let result = catch_unwind(|| {
        pool::GLOBAL_POOL.close_all();
        0
    });

    result.unwrap_or(-1)
}

/// Get library version string
#[no_mangle]
pub extern "C" fn db2_core_version() -> *const libc::c_char {
    static VERSION: &[u8] = b"1.0.0\0";
    VERSION.as_ptr() as *const libc::c_char
}
