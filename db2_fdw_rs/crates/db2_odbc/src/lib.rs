//! Safe ODBC wrapper for DB2 database connections
//!
//! This crate provides a safe, Rust-idiomatic interface to DB2 via ODBC.
//! It eliminates the memory safety issues present in the C implementation
//! by leveraging Rust's ownership system.

pub mod environment;
pub mod connection;
pub mod statement;
pub mod types;
pub mod error;
pub mod lob;

pub use environment::{Db2Environment, get_global_environment};
pub use connection::{Db2Connection, Db2ConnectionOptions, AuthMethod};
pub use statement::{Db2Statement, PreparedStatement, ParamInfo, ColumnDesc, BatchInsert};
pub use types::{Db2Value, Db2Type, SqlType};
pub use error::{Db2Error, Db2Result};
pub use lob::{Blob, Clob};

/// Default chunk size for LOB retrieval (8KB)
pub const LOB_CHUNK_SIZE: usize = 8192;

/// Default maximum size for LONG columns
pub const DEFAULT_MAX_LONG: usize = 32767;

/// Default prefetch row count
pub const DEFAULT_PREFETCH: usize = 200;
