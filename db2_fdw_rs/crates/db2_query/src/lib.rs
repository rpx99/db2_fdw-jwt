//! Query processing and SQL generation for DB2 FDW
//!
//! This crate handles SQL generation, expression deparsing, and type conversion
//! between PostgreSQL and DB2.

pub mod deparse;
pub mod convert;
pub mod pushdown;

pub use deparse::{Deparser, DeparseContext};
pub use convert::{TypeConverter, ConversionError};
pub use pushdown::{PushdownChecker, PushdownCapability};
