//! ODBC Environment management
//!
//! This module safely handles ODBC environment creation and NLS_LANG settings.
//! It fixes the use-after-free bug in the original C code's putenv() usage.

use once_cell::sync::OnceCell;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::sync::Arc;
use tracing::{debug, info};

use odbc_api::Environment;

use crate::error::{set_last_error, Db2Error, ErrorCode};
use crate::Result;

/// Global ODBC environment
static ODBC_ENV: OnceCell<Environment> = OnceCell::new();

/// NLS_LANG settings storage
///
/// IMPORTANT: This fixes the C bug where putenv() was called with a malloc'd
/// string that was later freed. putenv() stores the pointer, not a copy!
///
/// We store NLS strings here permanently to avoid use-after-free.
static NLS_STORAGE: OnceCell<RwLock<HashMap<String, CString>>> = OnceCell::new();

fn nls_storage() -> &'static RwLock<HashMap<String, CString>> {
    NLS_STORAGE.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Get or create the global ODBC environment
pub fn get_environment() -> Result<&'static Environment> {
    ODBC_ENV.get_or_try_init(|| {
        info!("Creating global ODBC environment");
        Environment::new().map_err(|e| Db2Error::OdbcError(e.to_string()))
    })
}

/// Set NLS_LANG environment variable safely
///
/// This is the safe replacement for the buggy C code that did:
/// ```c
/// char *nls = malloc(...);
/// sprintf(nls, "NLS_LANG=%s", value);
/// putenv(nls);
/// free(nls);  // BUG! putenv stores the pointer!
/// ```
pub fn set_nls_lang(value: &str) -> Result<()> {
    let env_string = format!("NLS_LANG={}", value);

    // Store the string permanently
    let c_string = CString::new(env_string.clone())
        .map_err(|_| Db2Error::InvalidParameter("NLS_LANG contains null byte".into()))?;

    let mut storage = nls_storage().write();

    // Check if already set to same value
    if let Some(existing) = storage.get(value) {
        debug!(nls_lang = %value, "NLS_LANG already set");
        return Ok(());
    }

    // Store and set
    storage.insert(value.to_string(), c_string);

    // Get pointer to the stored string (lives forever)
    let ptr = storage.get(value).unwrap().as_ptr();

    // SAFETY: The string is stored in our static HashMap and will never be freed
    unsafe {
        if libc::putenv(ptr as *mut libc::c_char) != 0 {
            return Err(Db2Error::Internal("putenv failed".into()));
        }
    }

    info!(nls_lang = %value, "NLS_LANG set successfully");
    Ok(())
}

// ============================================================================
// FFI Exports
// ============================================================================

/// Opaque handle for environment (for future use)
#[repr(C)]
pub struct Db2EnvHandle {
    _private: [u8; 0],
}

/// Initialize ODBC environment
///
/// # Safety
/// Safe to call multiple times.
#[no_mangle]
pub extern "C" fn db2_env_init() -> i32 {
    match std::panic::catch_unwind(|| get_environment()) {
        Ok(Ok(_)) => ErrorCode::Success as i32,
        Ok(Err(e)) => {
            set_last_error(&e.to_string());
            e.to_code() as i32
        }
        Err(_) => {
            set_last_error("Panic in db2_env_init");
            ErrorCode::InternalError as i32
        }
    }
}

/// Set NLS_LANG safely
///
/// # Safety
/// `nls_lang` must be a valid null-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn db2_env_set_nls_lang(nls_lang: *const libc::c_char) -> i32 {
    if nls_lang.is_null() {
        set_last_error("nls_lang is null");
        return ErrorCode::InvalidParameter as i32;
    }

    let result = std::panic::catch_unwind(|| {
        let c_str = unsafe { CStr::from_ptr(nls_lang) };
        let value = c_str
            .to_str()
            .map_err(|_| Db2Error::EncodingError("Invalid UTF-8 in NLS_LANG".into()))?;
        set_nls_lang(value)
    });

    match result {
        Ok(Ok(())) => ErrorCode::Success as i32,
        Ok(Err(e)) => {
            set_last_error(&e.to_string());
            e.to_code() as i32
        }
        Err(_) => {
            set_last_error("Panic in db2_env_set_nls_lang");
            ErrorCode::InternalError as i32
        }
    }
}
