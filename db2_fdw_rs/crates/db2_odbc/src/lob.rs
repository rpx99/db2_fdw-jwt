//! Large Object (LOB) handling
//!
//! This module provides safe handling for BLOB, CLOB, and DBCLOB types.
//! It replaces the unsafe C implementation with proper memory management.

use std::io::{Read, Write};
use tracing::debug;

use crate::error::{Db2Error, Db2Result};
use crate::LOB_CHUNK_SIZE;

/// Binary Large Object (BLOB)
///
/// Provides streaming access to binary data stored in DB2.
#[derive(Debug, Clone)]
pub struct Blob {
    /// The actual binary data
    data: Vec<u8>,
    /// Whether the LOB was truncated due to size limits
    truncated: bool,
    /// Original size if known (may differ from data.len() if truncated)
    original_size: Option<usize>,
}

impl Blob {
    /// Create a new empty BLOB
    pub fn new() -> Self {
        Self {
            data: Vec::new(),
            truncated: false,
            original_size: None,
        }
    }

    /// Create a BLOB from existing data
    pub fn from_bytes(data: Vec<u8>) -> Self {
        Self {
            data,
            truncated: false,
            original_size: None,
        }
    }

    /// Create a BLOB with a maximum size limit
    ///
    /// If the LOB exceeds max_size, it will be truncated
    pub fn from_bytes_with_limit(data: Vec<u8>, max_size: usize) -> Self {
        if data.len() > max_size {
            let mut truncated_data = data;
            truncated_data.truncate(max_size);
            Self {
                original_size: Some(truncated_data.len() + (data.len() - max_size)),
                data: truncated_data,
                truncated: true,
            }
        } else {
            Self::from_bytes(data)
        }
    }

    /// Get the binary data
    pub fn as_bytes(&self) -> &[u8] {
        &self.data
    }

    /// Consume and return the binary data
    pub fn into_bytes(self) -> Vec<u8> {
        self.data
    }

    /// Get the length of the stored data
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Check if the BLOB is empty
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Check if the BLOB was truncated
    pub fn is_truncated(&self) -> bool {
        self.truncated
    }

    /// Get the original size (if truncated and known)
    pub fn original_size(&self) -> Option<usize> {
        self.original_size
    }

    /// Append data to the BLOB (used during streaming reads)
    pub fn append(&mut self, chunk: &[u8]) {
        self.data.extend_from_slice(chunk);
    }

    /// Read the BLOB in chunks for streaming
    pub fn chunks(&self, chunk_size: usize) -> impl Iterator<Item = &[u8]> {
        self.data.chunks(chunk_size)
    }
}

impl Default for Blob {
    fn default() -> Self {
        Self::new()
    }
}

impl AsRef<[u8]> for Blob {
    fn as_ref(&self) -> &[u8] {
        &self.data
    }
}

impl From<Vec<u8>> for Blob {
    fn from(data: Vec<u8>) -> Self {
        Self::from_bytes(data)
    }
}

/// Character Large Object (CLOB)
///
/// Provides streaming access to text data stored in DB2.
#[derive(Debug, Clone)]
pub struct Clob {
    /// The text data
    data: String,
    /// Whether the LOB was truncated
    truncated: bool,
    /// Original character count if known
    original_length: Option<usize>,
}

impl Clob {
    /// Create a new empty CLOB
    pub fn new() -> Self {
        Self {
            data: String::new(),
            truncated: false,
            original_length: None,
        }
    }

    /// Create a CLOB from existing text
    pub fn from_string(data: String) -> Self {
        Self {
            data,
            truncated: false,
            original_length: None,
        }
    }

    /// Create a CLOB with a maximum character limit
    pub fn from_string_with_limit(mut data: String, max_chars: usize) -> Self {
        if data.chars().count() > max_chars {
            let original_len = data.chars().count();
            // Truncate at character boundary
            let truncate_at: usize = data
                .char_indices()
                .take(max_chars)
                .last()
                .map(|(i, c)| i + c.len_utf8())
                .unwrap_or(0);
            data.truncate(truncate_at);
            Self {
                data,
                truncated: true,
                original_length: Some(original_len),
            }
        } else {
            Self::from_string(data)
        }
    }

    /// Get the text data
    pub fn as_str(&self) -> &str {
        &self.data
    }

    /// Consume and return the text data
    pub fn into_string(self) -> String {
        self.data
    }

    /// Get the character count
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Get the character count (not bytes)
    pub fn char_count(&self) -> usize {
        self.data.chars().count()
    }

    /// Check if the CLOB is empty
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Check if the CLOB was truncated
    pub fn is_truncated(&self) -> bool {
        self.truncated
    }

    /// Get the original character count (if truncated)
    pub fn original_length(&self) -> Option<usize> {
        self.original_length
    }

    /// Append text to the CLOB
    pub fn append(&mut self, text: &str) {
        self.data.push_str(text);
    }
}

impl Default for Clob {
    fn default() -> Self {
        Self::new()
    }
}

impl AsRef<str> for Clob {
    fn as_ref(&self) -> &str {
        &self.data
    }
}

impl From<String> for Clob {
    fn from(data: String) -> Self {
        Self::from_string(data)
    }
}

/// LOB reader for streaming large objects from DB2
///
/// This is used internally to read LOBs in chunks, preventing
/// the buffer overflow issues present in the C implementation.
pub struct LobReader {
    /// Chunk size for reading
    chunk_size: usize,
    /// Maximum total size to read (0 = unlimited)
    max_size: usize,
    /// Total bytes read so far
    bytes_read: usize,
    /// Whether we've reached the end
    at_end: bool,
}

impl LobReader {
    /// Create a new LOB reader
    pub fn new(max_size: usize) -> Self {
        Self {
            chunk_size: LOB_CHUNK_SIZE,
            max_size,
            bytes_read: 0,
            at_end: false,
        }
    }

    /// Set the chunk size
    pub fn with_chunk_size(mut self, size: usize) -> Self {
        self.chunk_size = size;
        self
    }

    /// Check if we've reached the maximum size
    pub fn is_at_limit(&self) -> bool {
        self.max_size > 0 && self.bytes_read >= self.max_size
    }

    /// Check if we've reached the end of the LOB
    pub fn is_at_end(&self) -> bool {
        self.at_end
    }

    /// Get total bytes read
    pub fn bytes_read(&self) -> usize {
        self.bytes_read
    }

    /// Read a BLOB chunk by chunk
    ///
    /// This is a safe replacement for the C db2GetLob function.
    /// It uses proper bounds checking and prevents buffer overflows.
    pub fn read_blob<F>(&mut self, mut read_chunk: F) -> Db2Result<Blob>
    where
        F: FnMut(&mut [u8]) -> Db2Result<Option<usize>>,
    {
        let mut blob = Blob::new();
        let mut buffer = vec![0u8; self.chunk_size];

        loop {
            // Check size limit
            if self.is_at_limit() {
                blob.truncated = true;
                blob.original_size = Some(self.bytes_read);
                debug!(bytes_read = self.bytes_read, "LOB truncated at size limit");
                break;
            }

            // Adjust buffer size if near limit
            let remaining = if self.max_size > 0 {
                self.max_size - self.bytes_read
            } else {
                self.chunk_size
            };
            let read_size = remaining.min(self.chunk_size);

            // Read chunk
            match read_chunk(&mut buffer[..read_size])? {
                Some(n) if n > 0 => {
                    blob.append(&buffer[..n]);
                    self.bytes_read += n;
                }
                _ => {
                    self.at_end = true;
                    break;
                }
            }
        }

        Ok(blob)
    }

    /// Read a CLOB chunk by chunk
    pub fn read_clob<F>(&mut self, mut read_chunk: F) -> Db2Result<Clob>
    where
        F: FnMut(&mut [u8]) -> Db2Result<Option<usize>>,
    {
        let blob = self.read_blob(read_chunk)?;

        // Convert to string, handling encoding errors
        let text = String::from_utf8(blob.into_bytes()).map_err(|e| {
            Db2Error::EncodingError(format!("Invalid UTF-8 in CLOB: {}", e))
        })?;

        Ok(Clob::from_string(text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_blob_from_bytes() {
        let data = vec![1, 2, 3, 4, 5];
        let blob = Blob::from_bytes(data.clone());
        assert_eq!(blob.as_bytes(), &data);
        assert!(!blob.is_truncated());
    }

    #[test]
    fn test_blob_truncation() {
        let data = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        let blob = Blob::from_bytes_with_limit(data, 5);
        assert_eq!(blob.len(), 5);
        assert!(blob.is_truncated());
    }

    #[test]
    fn test_clob_from_string() {
        let text = "Hello, World!".to_string();
        let clob = Clob::from_string(text.clone());
        assert_eq!(clob.as_str(), &text);
        assert!(!clob.is_truncated());
    }

    #[test]
    fn test_clob_truncation() {
        let text = "Hello, World!".to_string();
        let clob = Clob::from_string_with_limit(text, 5);
        assert_eq!(clob.as_str(), "Hello");
        assert!(clob.is_truncated());
    }

    #[test]
    fn test_lob_reader() {
        let mut reader = LobReader::new(100);
        let test_data = vec![1, 2, 3, 4, 5];
        let mut pos = 0;

        let blob = reader
            .read_blob(|buf| {
                if pos >= test_data.len() {
                    Ok(None)
                } else {
                    let end = (pos + buf.len()).min(test_data.len());
                    let n = end - pos;
                    buf[..n].copy_from_slice(&test_data[pos..end]);
                    pos = end;
                    Ok(Some(n))
                }
            })
            .unwrap();

        assert_eq!(blob.as_bytes(), &test_data);
    }
}
