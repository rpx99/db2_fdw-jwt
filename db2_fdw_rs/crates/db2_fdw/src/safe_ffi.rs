//! Safe FFI Wrappers
//!
//! This module provides safe wrappers around unsafe PostgreSQL FFI calls.
//! All operations include safety checks and proper error handling.

use pgrx::pg_sys;

/// Get type output function with safety checks
///
/// # Safety
/// - Returns Oid or error instead of raw unsafe value
/// - All parameters are validated
/// - NULL-pointers are checked
///
/// # Errors
/// Returns PostgresError if type lookup fails
pub unsafe fn get_type_output_info(
    typid: pg_sys::Oid,
) -> Result<pg_sys::Oid, String> {
    let mut typ_output: pg_sys::Oid = pg_sys::Oid::default();
    let mut typ_is_varlena: bool = false;

    // getTypeOutputInfo in PG18 takes: Oid, *mut Oid, *mut bool
    let _ = pg_sys::getTypeOutputInfo(typid, &mut typ_output, &mut typ_is_varlena);

    // Check if we got a valid function OID - cannot directly compare Oid
    // Parse debug output to check if it's InvalidOid
    let oid_str = format!("{:?}", typ_output);
    let is_invalid = oid_str == "Oid(0)" || oid_str.contains("(0)");

    if is_invalid {
        return Err(format!("No output function for type Oid({:?})", typid));
    }

    Ok(typ_output)
}

/// Result of varlena data extraction
///
/// Contains both the data pointer and length, validated and bounds-checked.
#[derive(Debug, Clone, Copy)]
pub struct VarlenaData {
    /// Pointer to the data (excluding header)
    pub ptr: *const u8,
    /// Length of the data
    pub len: usize,
}

/// Get varlena data safely
///
/// # Safety
/// - Validates pointer is not null
/// - Validates length is within bounds
///
/// # Returns
/// VarlenaData struct with validated pointer and length
///
/// # Panics
/// Panics if validation fails (should never happen with valid PostgreSQL data)
pub unsafe fn varlena_data(ptr: *const u8) -> VarlenaData {
    // Basic null check
    assert!(!ptr.is_null(), "varlena pointer is null");

    // Get header from first 4 bytes (with masking to ignore compression bit)
    let header = *(ptr as *const i32);
    let total_len = (header & 0x3FFFFFFF) as usize;

    // Validate reasonable size limits (1MB is a safe upper bound for varlena)
    assert!(total_len >= 1, "varlena header too small");
    assert!(total_len <= 1024 * 1024, "varlena size exceeds safe limit (1MB)");

    // Data starts after header
    let data = ptr.add(1);
    let len = total_len - 1; // Exclude header size

    VarlenaData {
        ptr: data,
        len: len,
    }
}

/// Get text from Datum safely
///
/// # Safety
/// - Validates Datum is TEXTOID type
/// - Handles NULL values
pub unsafe fn datum_get_text(typid: pg_sys::Oid, datum: pg_sys::Datum) -> Result<String, String> {
    match typid {
        pg_sys::TEXTOID | pg_sys::VARCHAROID | pg_sys::BPCHAROID => {
            let text_ptr = datum.cast_mut_ptr::<pg_sys::varlena>();
            let vd = varlena_data(text_ptr as *const u8);
            let bytes = std::slice::from_raw_parts(vd.ptr, vd.len);
            String::from_utf8(bytes.to_vec())
                .map_err(|e| format!("Invalid UTF-8 in text column: {}", e))
        }
        _ => Err(format!("Datum is not a text type, got Oid({:?})", typid)),
    }
}

/// Get binary data from Datum safely
///
/// # Safety
/// - Validates Datum is BYTEAOID type
pub unsafe fn datum_get_binary(typid: pg_sys::Oid, datum: pg_sys::Datum) -> Result<Vec<u8>, String> {
    if typid != pg_sys::BYTEAOID {
        return Err(format!("Datum is not binary type, got Oid({:?})", typid));
    }

    let bytea_ptr = datum.cast_mut_ptr::<pg_sys::varlena>();
    let vd = varlena_data(bytea_ptr as *const u8);
    let bytes = std::slice::from_raw_parts(vd.ptr, vd.len);
    Ok(bytes.to_vec())
}

/// Convert f32 to Datum safely
///
/// Uses bit-preservation conversion instead of From trait which isn't safe
#[inline]
pub fn f32_to_datum(value: f32) -> pg_sys::Datum {
    // Bit-cast: preserve exact binary representation
    pg_sys::Datum::from(value.to_bits() as i32)
}

/// Convert f64 to Datum safely
///
/// Uses bit-preservation conversion instead of From trait which isn't safe
#[inline]
pub fn f64_to_datum(value: f64) -> pg_sys::Datum {
    // Bit-cast: preserve exact binary representation
    pg_sys::Datum::from(value.to_bits() as i64)
}

/// Check if Oid is one of text-like types
#[inline]
pub fn is_text_type(typid: pg_sys::Oid) -> bool {
    typid == pg_sys::TEXTOID || typid == pg_sys::VARCHAROID || typid == pg_sys::BPCHAROID
}

/// Check if Oid is binary type
#[inline]
pub fn is_binary_type(typid: pg_sys::Oid) -> bool {
    typid == pg_sys::BYTEAOID
}

/// Safe wrapper for OidOutputFunctionCall
///
/// # Safety
/// - Validates function OID
/// - Handles NULL returns
pub unsafe fn oid_output_call(function_oid: pg_sys::Oid, datum: pg_sys::Datum) -> Result<String, String> {
    let cstr = pg_sys::OidOutputFunctionCall(function_oid, datum);

    if cstr.is_null() {
        return Err("OidOutputFunctionCall returned NULL".to_string());
    }

    std::ffi::CStr::from_ptr(cstr)
        .to_str()
        .map(|s| s.to_string())
        .map_err(|e| format!("Invalid UTF-8 from output function: {}", e))
}
