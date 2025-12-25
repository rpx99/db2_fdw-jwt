//! FFI helper utilities
//!
//! This module provides helper functions for FFI boundary handling.

use std::ffi::{CStr, CString};

/// Safely convert C string to Rust string
///
/// Returns None if the pointer is null or the string is invalid UTF-8.
pub fn c_str_to_string(ptr: *const libc::c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }

    unsafe { CStr::from_ptr(ptr) }
        .to_str()
        .ok()
        .map(String::from)
}

/// Safely convert Rust string to C string
///
/// Returns null pointer if the string contains null bytes.
pub fn string_to_c_str(s: &str) -> *const libc::c_char {
    CString::new(s)
        .map(|cs| cs.into_raw() as *const libc::c_char)
        .unwrap_or(std::ptr::null())
}

/// Free a C string allocated by Rust
///
/// # Safety
/// The pointer must have been allocated by `string_to_c_str` or be null.
#[no_mangle]
pub unsafe extern "C" fn db2_free_string(ptr: *mut libc::c_char) {
    if !ptr.is_null() {
        let _ = unsafe { CString::from_raw(ptr) };
    }
}

/// Wrapper macro for FFI functions to handle panics
#[macro_export]
macro_rules! ffi_try {
    ($expr:expr, $error_ret:expr) => {
        match std::panic::catch_unwind(|| $expr) {
            Ok(Ok(val)) => val,
            Ok(Err(e)) => {
                $crate::error::set_last_error(&e.to_string());
                return $error_ret;
            }
            Err(_) => {
                $crate::error::set_last_error("Panic in FFI call");
                return $error_ret;
            }
        }
    };
}
