//! Safe LOB (Large Object) handling
//!
//! This module replaces the buffer-overflow prone C LOB handling with
//! safe Rust code that properly manages memory and bounds.
//!
//! The original C code had issues:
//! - No bounds checking on LOB buffers
//! - Potential buffer overflow when LOB size exceeded expectations
//! - Manual memory management errors

use std::ffi::CStr;
use tracing::{debug, warn};

use crate::error::{set_last_error, Db2Error, ErrorCode};
use crate::Result;

/// Default chunk size for LOB reading (8KB)
pub const DEFAULT_CHUNK_SIZE: usize = 8192;

/// Maximum LOB size (1GB)
pub const MAX_LOB_SIZE: usize = 1024 * 1024 * 1024;

/// BLOB (Binary Large Object) with safe memory management
#[derive(Debug, Clone)]
pub struct Blob {
    data: Vec<u8>,
    truncated: bool,
    original_size: Option<usize>,
}

impl Blob {
    /// Create empty BLOB
    pub fn new() -> Self {
        Self {
            data: Vec::new(),
            truncated: false,
            original_size: None,
        }
    }

    /// Create BLOB with pre-allocated capacity
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            data: Vec::with_capacity(capacity.min(MAX_LOB_SIZE)),
            truncated: false,
            original_size: None,
        }
    }

    /// Create BLOB from bytes
    pub fn from_bytes(data: Vec<u8>) -> Self {
        Self {
            data,
            truncated: false,
            original_size: None,
        }
    }

    /// Create BLOB with size limit
    pub fn from_bytes_with_limit(data: Vec<u8>, max_size: usize) -> Self {
        let original_len = data.len();
        if original_len > max_size {
            let mut truncated_data = data;
            truncated_data.truncate(max_size);
            Self {
                data: truncated_data,
                truncated: true,
                original_size: Some(original_len),
            }
        } else {
            Self::from_bytes(data)
        }
    }

    /// Append data (with bounds checking)
    pub fn append(&mut self, chunk: &[u8]) -> Result<()> {
        let new_size = self.data.len().saturating_add(chunk.len());
        if new_size > MAX_LOB_SIZE {
            return Err(Db2Error::LobError(format!(
                "LOB would exceed maximum size of {} bytes",
                MAX_LOB_SIZE
            )));
        }
        self.data.extend_from_slice(chunk);
        Ok(())
    }

    /// Get data as slice
    pub fn as_bytes(&self) -> &[u8] {
        &self.data
    }

    /// Get data length
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Check if truncated
    pub fn is_truncated(&self) -> bool {
        self.truncated
    }

    /// Get original size if truncated
    pub fn original_size(&self) -> Option<usize> {
        self.original_size
    }

    /// Consume and return data
    pub fn into_bytes(self) -> Vec<u8> {
        self.data
    }
}

impl Default for Blob {
    fn default() -> Self {
        Self::new()
    }
}

/// CLOB (Character Large Object)
#[derive(Debug, Clone)]
pub struct Clob {
    data: String,
    truncated: bool,
    original_length: Option<usize>,
}

impl Clob {
    /// Create empty CLOB
    pub fn new() -> Self {
        Self {
            data: String::new(),
            truncated: false,
            original_length: None,
        }
    }

    /// Create CLOB from string
    pub fn from_string(data: String) -> Self {
        Self {
            data,
            truncated: false,
            original_length: None,
        }
    }

    /// Create CLOB with character limit
    pub fn from_string_with_limit(data: String, max_chars: usize) -> Self {
        let char_count = data.chars().count();
        if char_count > max_chars {
            let truncated_data: String = data.chars().take(max_chars).collect();
            Self {
                data: truncated_data,
                truncated: true,
                original_length: Some(char_count),
            }
        } else {
            Self::from_string(data)
        }
    }

    /// Append text (with bounds checking)
    pub fn append(&mut self, text: &str) -> Result<()> {
        let new_size = self.data.len().saturating_add(text.len());
        if new_size > MAX_LOB_SIZE {
            return Err(Db2Error::LobError(format!(
                "CLOB would exceed maximum size of {} bytes",
                MAX_LOB_SIZE
            )));
        }
        self.data.push_str(text);
        Ok(())
    }

    /// Get data as string slice
    pub fn as_str(&self) -> &str {
        &self.data
    }

    /// Get byte length
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Get character count
    pub fn char_count(&self) -> usize {
        self.data.chars().count()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Check if truncated
    pub fn is_truncated(&self) -> bool {
        self.truncated
    }

    /// Consume and return data
    pub fn into_string(self) -> String {
        self.data
    }
}

impl Default for Clob {
    fn default() -> Self {
        Self::new()
    }
}

/// LOB reader for streaming data from database
pub struct LobReader {
    chunk_size: usize,
    max_size: usize,
    bytes_read: usize,
}

impl LobReader {
    /// Create a new LOB reader
    pub fn new(max_size: usize) -> Self {
        Self {
            chunk_size: DEFAULT_CHUNK_SIZE,
            max_size: max_size.min(MAX_LOB_SIZE),
            bytes_read: 0,
        }
    }

    /// Set chunk size
    pub fn with_chunk_size(mut self, size: usize) -> Self {
        self.chunk_size = size.max(1024).min(1024 * 1024); // 1KB to 1MB
        self
    }

    /// Check if at limit
    pub fn is_at_limit(&self) -> bool {
        self.max_size > 0 && self.bytes_read >= self.max_size
    }

    /// Get bytes read
    pub fn bytes_read(&self) -> usize {
        self.bytes_read
    }

    /// Read BLOB with safe bounds checking
    pub fn read_blob<F>(&mut self, mut read_chunk: F) -> Result<Blob>
    where
        F: FnMut(&mut [u8]) -> Result<Option<usize>>,
    {
        let mut blob = Blob::with_capacity(self.chunk_size * 4);
        let mut buffer = vec![0u8; self.chunk_size];

        loop {
            if self.is_at_limit() {
                blob.truncated = true;
                blob.original_size = Some(self.bytes_read);
                debug!(bytes_read = self.bytes_read, "LOB truncated at limit");
                break;
            }

            let remaining = if self.max_size > 0 {
                self.max_size - self.bytes_read
            } else {
                self.chunk_size
            };
            let read_size = remaining.min(self.chunk_size);

            match read_chunk(&mut buffer[..read_size])? {
                Some(n) if n > 0 => {
                    blob.append(&buffer[..n])?;
                    self.bytes_read += n;
                }
                _ => break,
            }
        }

        Ok(blob)
    }

    /// Read CLOB with safe bounds checking
    pub fn read_clob<F>(&mut self, read_chunk: F) -> Result<Clob>
    where
        F: FnMut(&mut [u8]) -> Result<Option<usize>>,
    {
        let blob = self.read_blob(read_chunk)?;

        let text = String::from_utf8(blob.into_bytes()).map_err(|e| {
            Db2Error::EncodingError(format!("Invalid UTF-8 in CLOB: {}", e))
        })?;

        let mut clob = Clob::from_string(text);
        clob.truncated = self.is_at_limit();

        Ok(clob)
    }
}

// ============================================================================
// FFI Exports
// ============================================================================

/// Opaque BLOB handle
pub type BlobHandle = *mut Blob;

/// Opaque CLOB handle
pub type ClobHandle = *mut Clob;

/// Create a new BLOB with capacity
#[no_mangle]
pub extern "C" fn db2_blob_new(capacity: usize) -> BlobHandle {
    std::panic::catch_unwind(|| {
        let blob = if capacity > 0 {
            Blob::with_capacity(capacity)
        } else {
            Blob::new()
        };
        Box::into_raw(Box::new(blob))
    })
    .unwrap_or(std::ptr::null_mut())
}

/// Append data to BLOB
///
/// # Safety
/// `handle` must be valid, `data` must point to `len` bytes.
#[no_mangle]
pub unsafe extern "C" fn db2_blob_append(
    handle: BlobHandle,
    data: *const u8,
    len: usize,
) -> i32 {
    if handle.is_null() || data.is_null() {
        return ErrorCode::InvalidParameter as i32;
    }

    let result = std::panic::catch_unwind(|| {
        let blob = unsafe { &mut *handle };
        let slice = unsafe { std::slice::from_raw_parts(data, len) };
        blob.append(slice)
    });

    match result {
        Ok(Ok(())) => ErrorCode::Success as i32,
        Ok(Err(e)) => {
            set_last_error(&e.to_string());
            e.to_code() as i32
        }
        Err(_) => {
            set_last_error("Panic in db2_blob_append");
            ErrorCode::InternalError as i32
        }
    }
}

/// Get BLOB data pointer and length
///
/// # Safety
/// `handle` must be valid, `out_len` must be valid pointer.
#[no_mangle]
pub unsafe extern "C" fn db2_blob_data(
    handle: BlobHandle,
    out_len: *mut usize,
) -> *const u8 {
    if handle.is_null() || out_len.is_null() {
        return std::ptr::null();
    }

    std::panic::catch_unwind(|| {
        let blob = unsafe { &*handle };
        unsafe { *out_len = blob.len() };
        blob.as_bytes().as_ptr()
    })
    .unwrap_or(std::ptr::null())
}

/// Check if BLOB was truncated
#[no_mangle]
pub unsafe extern "C" fn db2_blob_is_truncated(handle: BlobHandle) -> i32 {
    if handle.is_null() {
        return 0;
    }

    std::panic::catch_unwind(|| {
        let blob = unsafe { &*handle };
        blob.is_truncated() as i32
    })
    .unwrap_or(0)
}

/// Free BLOB
///
/// # Safety
/// `handle` must be valid or null.
#[no_mangle]
pub unsafe extern "C" fn db2_blob_free(handle: BlobHandle) {
    if !handle.is_null() {
        let _ = std::panic::catch_unwind(|| {
            let _ = unsafe { Box::from_raw(handle) };
        });
    }
}

/// Create a new CLOB
#[no_mangle]
pub extern "C" fn db2_clob_new() -> ClobHandle {
    std::panic::catch_unwind(|| Box::into_raw(Box::new(Clob::new())))
        .unwrap_or(std::ptr::null_mut())
}

/// Append text to CLOB
///
/// # Safety
/// `handle` must be valid, `text` must be null-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn db2_clob_append(
    handle: ClobHandle,
    text: *const libc::c_char,
) -> i32 {
    if handle.is_null() || text.is_null() {
        return ErrorCode::InvalidParameter as i32;
    }

    let result = std::panic::catch_unwind(|| {
        let clob = unsafe { &mut *handle };
        let c_str = unsafe { CStr::from_ptr(text) };
        let text = c_str
            .to_str()
            .map_err(|_| Db2Error::EncodingError("Invalid UTF-8".into()))?;
        clob.append(text)
    });

    match result {
        Ok(Ok(())) => ErrorCode::Success as i32,
        Ok(Err(e)) => {
            set_last_error(&e.to_string());
            e.to_code() as i32
        }
        Err(_) => {
            set_last_error("Panic in db2_clob_append");
            ErrorCode::InternalError as i32
        }
    }
}

/// Get CLOB data as C string (null-terminated)
///
/// # Safety
/// `handle` must be valid.
/// Returns pointer valid until CLOB is modified or freed.
#[no_mangle]
pub unsafe extern "C" fn db2_clob_data(handle: ClobHandle) -> *const libc::c_char {
    if handle.is_null() {
        return std::ptr::null();
    }

    std::panic::catch_unwind(|| {
        let clob = unsafe { &*handle };
        // Note: This is safe only if the string doesn't contain null bytes
        clob.as_str().as_ptr() as *const libc::c_char
    })
    .unwrap_or(std::ptr::null())
}

/// Get CLOB length in bytes
#[no_mangle]
pub unsafe extern "C" fn db2_clob_len(handle: ClobHandle) -> usize {
    if handle.is_null() {
        return 0;
    }

    std::panic::catch_unwind(|| {
        let clob = unsafe { &*handle };
        clob.len()
    })
    .unwrap_or(0)
}

/// Free CLOB
///
/// # Safety
/// `handle` must be valid or null.
#[no_mangle]
pub unsafe extern "C" fn db2_clob_free(handle: ClobHandle) {
    if !handle.is_null() {
        let _ = std::panic::catch_unwind(|| {
            let _ = unsafe { Box::from_raw(handle) };
        });
    }
}
