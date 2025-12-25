//! SQL type definitions and value representations
//!
//! This module provides type-safe representations of DB2 SQL types
//! and handles conversions between DB2, Rust, and PostgreSQL types.

use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use rust_decimal::Decimal;

/// SQL data types as defined by DB2 ODBC
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i16)]
pub enum SqlType {
    // Character types
    Char = 1,
    Varchar = 12,
    LongVarchar = -1,

    // Binary types
    Binary = -2,
    VarBinary = -3,
    LongVarBinary = -4,

    // Numeric types
    Decimal = 3,
    Numeric = 2,
    SmallInt = 5,
    Integer = 4,
    BigInt = -5,
    Real = 7,
    Float = 6,
    Double = 8,

    // Date/Time types
    Date = 91,
    Time = 92,
    Timestamp = 93,

    // Large Object types
    Blob = -98,
    Clob = -99,
    DbClob = -350,

    // XML type
    Xml = -370,

    // Boolean (DB2 11.1+)
    Boolean = 16,

    // Unknown/default
    Unknown = 0,
}

impl SqlType {
    /// Create SqlType from ODBC type constant
    pub fn from_odbc(odbc_type: i16) -> Self {
        match odbc_type {
            1 => SqlType::Char,
            12 => SqlType::Varchar,
            -1 => SqlType::LongVarchar,
            -2 => SqlType::Binary,
            -3 => SqlType::VarBinary,
            -4 => SqlType::LongVarBinary,
            3 => SqlType::Decimal,
            2 => SqlType::Numeric,
            5 => SqlType::SmallInt,
            4 => SqlType::Integer,
            -5 => SqlType::BigInt,
            7 => SqlType::Real,
            6 => SqlType::Float,
            8 => SqlType::Double,
            91 => SqlType::Date,
            92 => SqlType::Time,
            93 => SqlType::Timestamp,
            -98 => SqlType::Blob,
            -99 => SqlType::Clob,
            -350 => SqlType::DbClob,
            -370 => SqlType::Xml,
            16 => SqlType::Boolean,
            _ => SqlType::Unknown,
        }
    }

    /// Check if this type is a character type
    pub fn is_character(&self) -> bool {
        matches!(self, SqlType::Char | SqlType::Varchar | SqlType::LongVarchar | SqlType::Clob | SqlType::DbClob | SqlType::Xml)
    }

    /// Check if this type is a binary type
    pub fn is_binary(&self) -> bool {
        matches!(self, SqlType::Binary | SqlType::VarBinary | SqlType::LongVarBinary | SqlType::Blob)
    }

    /// Check if this type is a numeric type
    pub fn is_numeric(&self) -> bool {
        matches!(self,
            SqlType::Decimal | SqlType::Numeric | SqlType::SmallInt |
            SqlType::Integer | SqlType::BigInt | SqlType::Real |
            SqlType::Float | SqlType::Double
        )
    }

    /// Check if this type is a date/time type
    pub fn is_datetime(&self) -> bool {
        matches!(self, SqlType::Date | SqlType::Time | SqlType::Timestamp)
    }

    /// Check if this type is a LOB type
    pub fn is_lob(&self) -> bool {
        matches!(self, SqlType::Blob | SqlType::Clob | SqlType::DbClob)
    }

    /// Get the PostgreSQL OID equivalent (approximate mapping)
    pub fn to_pg_oid(&self) -> u32 {
        match self {
            SqlType::Char | SqlType::Varchar | SqlType::LongVarchar => 25,      // TEXT
            SqlType::Binary | SqlType::VarBinary | SqlType::LongVarBinary => 17, // BYTEA
            SqlType::SmallInt => 21,                                             // INT2
            SqlType::Integer => 23,                                              // INT4
            SqlType::BigInt => 20,                                               // INT8
            SqlType::Decimal | SqlType::Numeric => 1700,                         // NUMERIC
            SqlType::Real => 700,                                                // FLOAT4
            SqlType::Float | SqlType::Double => 701,                             // FLOAT8
            SqlType::Date => 1082,                                               // DATE
            SqlType::Time => 1083,                                               // TIME
            SqlType::Timestamp => 1114,                                          // TIMESTAMP
            SqlType::Blob => 17,                                                 // BYTEA
            SqlType::Clob | SqlType::DbClob | SqlType::Xml => 25,               // TEXT
            SqlType::Boolean => 16,                                              // BOOL
            SqlType::Unknown => 25,                                              // Default to TEXT
        }
    }
}

/// DB2 type information with precision and scale
#[derive(Debug, Clone)]
pub struct Db2Type {
    pub sql_type: SqlType,
    pub precision: u32,
    pub scale: i16,
    pub nullable: bool,
    pub column_size: usize,
}

impl Db2Type {
    pub fn new(sql_type: SqlType) -> Self {
        Self {
            sql_type,
            precision: 0,
            scale: 0,
            nullable: true,
            column_size: 0,
        }
    }

    pub fn with_precision(mut self, precision: u32, scale: i16) -> Self {
        self.precision = precision;
        self.scale = scale;
        self
    }

    pub fn with_size(mut self, size: usize) -> Self {
        self.column_size = size;
        self
    }

    pub fn not_null(mut self) -> Self {
        self.nullable = false;
        self
    }
}

/// Rust representation of DB2 values
///
/// This enum provides type-safe storage for values retrieved from DB2.
/// It eliminates the buffer overflow and type confusion bugs from the C implementation.
#[derive(Debug, Clone, PartialEq)]
pub enum Db2Value {
    /// SQL NULL
    Null,

    /// Character/string data (CHAR, VARCHAR, CLOB)
    Text(String),

    /// Binary data (BINARY, VARBINARY, BLOB)
    Binary(Vec<u8>),

    /// 16-bit signed integer (SMALLINT)
    SmallInt(i16),

    /// 32-bit signed integer (INTEGER)
    Integer(i32),

    /// 64-bit signed integer (BIGINT)
    BigInt(i64),

    /// Single-precision float (REAL)
    Real(f32),

    /// Double-precision float (FLOAT, DOUBLE)
    Double(f64),

    /// Decimal/Numeric with arbitrary precision
    Decimal(Decimal),

    /// Date value
    Date(NaiveDate),

    /// Time value
    Time(NaiveTime),

    /// Timestamp value
    Timestamp(NaiveDateTime),

    /// Boolean value (DB2 11.1+)
    Boolean(bool),

    /// XML data stored as text
    Xml(String),
}

impl Db2Value {
    /// Check if this value is NULL
    pub fn is_null(&self) -> bool {
        matches!(self, Db2Value::Null)
    }

    /// Try to get as string reference
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Db2Value::Text(s) | Db2Value::Xml(s) => Some(s),
            _ => None,
        }
    }

    /// Try to get as i64
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Db2Value::SmallInt(v) => Some(*v as i64),
            Db2Value::Integer(v) => Some(*v as i64),
            Db2Value::BigInt(v) => Some(*v),
            _ => None,
        }
    }

    /// Try to get as f64
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Db2Value::Real(v) => Some(*v as f64),
            Db2Value::Double(v) => Some(*v),
            Db2Value::Decimal(d) => d.to_string().parse().ok(),
            _ => None,
        }
    }

    /// Try to get as bytes
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Db2Value::Binary(b) => Some(b),
            Db2Value::Text(s) => Some(s.as_bytes()),
            _ => None,
        }
    }

    /// Get the SQL type of this value
    pub fn sql_type(&self) -> SqlType {
        match self {
            Db2Value::Null => SqlType::Unknown,
            Db2Value::Text(_) => SqlType::Varchar,
            Db2Value::Binary(_) => SqlType::VarBinary,
            Db2Value::SmallInt(_) => SqlType::SmallInt,
            Db2Value::Integer(_) => SqlType::Integer,
            Db2Value::BigInt(_) => SqlType::BigInt,
            Db2Value::Real(_) => SqlType::Real,
            Db2Value::Double(_) => SqlType::Double,
            Db2Value::Decimal(_) => SqlType::Decimal,
            Db2Value::Date(_) => SqlType::Date,
            Db2Value::Time(_) => SqlType::Time,
            Db2Value::Timestamp(_) => SqlType::Timestamp,
            Db2Value::Boolean(_) => SqlType::Boolean,
            Db2Value::Xml(_) => SqlType::Xml,
        }
    }
}

impl Default for Db2Value {
    fn default() -> Self {
        Db2Value::Null
    }
}

impl std::fmt::Display for Db2Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Db2Value::Null => write!(f, "NULL"),
            Db2Value::Text(s) => write!(f, "'{}'", s.replace('\'', "''")),
            Db2Value::Binary(b) => write!(f, "X'{}'", hex::encode(b)),
            Db2Value::SmallInt(v) => write!(f, "{}", v),
            Db2Value::Integer(v) => write!(f, "{}", v),
            Db2Value::BigInt(v) => write!(f, "{}", v),
            Db2Value::Real(v) => write!(f, "{}", v),
            Db2Value::Double(v) => write!(f, "{}", v),
            Db2Value::Decimal(d) => write!(f, "{}", d),
            Db2Value::Date(d) => write!(f, "DATE '{}'", d),
            Db2Value::Time(t) => write!(f, "TIME '{}'", t),
            Db2Value::Timestamp(ts) => write!(f, "TIMESTAMP '{}'", ts),
            Db2Value::Boolean(b) => write!(f, "{}", if *b { "TRUE" } else { "FALSE" }),
            Db2Value::Xml(x) => write!(f, "XMLPARSE(DOCUMENT '{}')", x.replace('\'', "''")),
        }
    }
}

// Hex encoding helper (simple implementation)
mod hex {
    pub fn encode(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{:02x}", b)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sql_type_classification() {
        assert!(SqlType::Varchar.is_character());
        assert!(SqlType::Blob.is_binary());
        assert!(SqlType::Integer.is_numeric());
        assert!(SqlType::Timestamp.is_datetime());
        assert!(SqlType::Clob.is_lob());
    }

    #[test]
    fn test_db2_value_display() {
        let text = Db2Value::Text("hello".into());
        assert_eq!(text.to_string(), "'hello'");

        let num = Db2Value::Integer(42);
        assert_eq!(num.to_string(), "42");
    }

    #[test]
    fn test_db2_value_null() {
        let null = Db2Value::Null;
        assert!(null.is_null());

        let not_null = Db2Value::Integer(0);
        assert!(!not_null.is_null());
    }
}
