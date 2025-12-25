//! Type conversion between PostgreSQL and DB2
//!
//! This module handles safe type conversions, eliminating the buffer overflow
//! and type confusion bugs from the C implementation.

use thiserror::Error;
use db2_odbc::{Db2Value, SqlType};
use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use rust_decimal::Decimal;

/// Errors that can occur during type conversion
#[derive(Error, Debug)]
pub enum ConversionError {
    #[error("Cannot convert {from} to {to}")]
    IncompatibleTypes { from: String, to: String },

    #[error("Value out of range for {target_type}: {value}")]
    OutOfRange { target_type: String, value: String },

    #[error("Parse error for {target_type}: {message}")]
    ParseError { target_type: String, message: String },

    #[error("NULL value not allowed")]
    NullNotAllowed,

    #[error("Invalid encoding: {0}")]
    InvalidEncoding(String),
}

/// Result type for conversion operations
pub type ConversionResult<T> = Result<T, ConversionError>;

/// Type converter for PostgreSQL <-> DB2 conversions
pub struct TypeConverter {
    /// Whether to handle encoding errors gracefully
    pub no_encoding_error: bool,
    /// Maximum size for LOB data
    pub max_long: usize,
}

impl TypeConverter {
    /// Create a new type converter with default settings
    pub fn new() -> Self {
        Self {
            no_encoding_error: false,
            max_long: db2_odbc::DEFAULT_MAX_LONG,
        }
    }

    /// Configure encoding error handling
    pub fn with_encoding_handling(mut self, no_error: bool) -> Self {
        self.no_encoding_error = no_error;
        self
    }

    /// Configure max LOB size
    pub fn with_max_long(mut self, max: usize) -> Self {
        self.max_long = max;
        self
    }

    /// Convert a Db2Value to a String (for TEXT columns)
    pub fn to_string(&self, value: &Db2Value) -> ConversionResult<Option<String>> {
        match value {
            Db2Value::Null => Ok(None),
            Db2Value::Text(s) => Ok(Some(s.clone())),
            Db2Value::Xml(s) => Ok(Some(s.clone())),
            Db2Value::SmallInt(n) => Ok(Some(n.to_string())),
            Db2Value::Integer(n) => Ok(Some(n.to_string())),
            Db2Value::BigInt(n) => Ok(Some(n.to_string())),
            Db2Value::Real(n) => Ok(Some(n.to_string())),
            Db2Value::Double(n) => Ok(Some(n.to_string())),
            Db2Value::Decimal(d) => Ok(Some(d.to_string())),
            Db2Value::Date(d) => Ok(Some(d.to_string())),
            Db2Value::Time(t) => Ok(Some(t.to_string())),
            Db2Value::Timestamp(ts) => Ok(Some(ts.to_string())),
            Db2Value::Boolean(b) => Ok(Some(if *b { "t" } else { "f" }.to_string())),
            Db2Value::Binary(b) => {
                // Try to convert binary to string
                match String::from_utf8(b.clone()) {
                    Ok(s) => Ok(Some(s)),
                    Err(e) if self.no_encoding_error => {
                        Ok(Some(String::from_utf8_lossy(b).into_owned()))
                    }
                    Err(e) => Err(ConversionError::InvalidEncoding(e.to_string())),
                }
            }
        }
    }

    /// Convert a Db2Value to i16
    pub fn to_i16(&self, value: &Db2Value) -> ConversionResult<Option<i16>> {
        match value {
            Db2Value::Null => Ok(None),
            Db2Value::SmallInt(n) => Ok(Some(*n)),
            Db2Value::Integer(n) => {
                i16::try_from(*n).map(Some).map_err(|_| ConversionError::OutOfRange {
                    target_type: "i16".into(),
                    value: n.to_string(),
                })
            }
            Db2Value::BigInt(n) => {
                i16::try_from(*n).map(Some).map_err(|_| ConversionError::OutOfRange {
                    target_type: "i16".into(),
                    value: n.to_string(),
                })
            }
            Db2Value::Text(s) => s
                .parse::<i16>()
                .map(Some)
                .map_err(|e| ConversionError::ParseError {
                    target_type: "i16".into(),
                    message: e.to_string(),
                }),
            other => Err(ConversionError::IncompatibleTypes {
                from: format!("{:?}", other.sql_type()),
                to: "i16".into(),
            }),
        }
    }

    /// Convert a Db2Value to i32
    pub fn to_i32(&self, value: &Db2Value) -> ConversionResult<Option<i32>> {
        match value {
            Db2Value::Null => Ok(None),
            Db2Value::SmallInt(n) => Ok(Some(*n as i32)),
            Db2Value::Integer(n) => Ok(Some(*n)),
            Db2Value::BigInt(n) => {
                i32::try_from(*n).map(Some).map_err(|_| ConversionError::OutOfRange {
                    target_type: "i32".into(),
                    value: n.to_string(),
                })
            }
            Db2Value::Text(s) => s
                .parse::<i32>()
                .map(Some)
                .map_err(|e| ConversionError::ParseError {
                    target_type: "i32".into(),
                    message: e.to_string(),
                }),
            other => Err(ConversionError::IncompatibleTypes {
                from: format!("{:?}", other.sql_type()),
                to: "i32".into(),
            }),
        }
    }

    /// Convert a Db2Value to i64
    pub fn to_i64(&self, value: &Db2Value) -> ConversionResult<Option<i64>> {
        match value {
            Db2Value::Null => Ok(None),
            Db2Value::SmallInt(n) => Ok(Some(*n as i64)),
            Db2Value::Integer(n) => Ok(Some(*n as i64)),
            Db2Value::BigInt(n) => Ok(Some(*n)),
            Db2Value::Text(s) => s
                .parse::<i64>()
                .map(Some)
                .map_err(|e| ConversionError::ParseError {
                    target_type: "i64".into(),
                    message: e.to_string(),
                }),
            other => Err(ConversionError::IncompatibleTypes {
                from: format!("{:?}", other.sql_type()),
                to: "i64".into(),
            }),
        }
    }

    /// Convert a Db2Value to f32
    pub fn to_f32(&self, value: &Db2Value) -> ConversionResult<Option<f32>> {
        match value {
            Db2Value::Null => Ok(None),
            Db2Value::Real(n) => Ok(Some(*n)),
            Db2Value::Double(n) => Ok(Some(*n as f32)),
            Db2Value::SmallInt(n) => Ok(Some(*n as f32)),
            Db2Value::Integer(n) => Ok(Some(*n as f32)),
            Db2Value::BigInt(n) => Ok(Some(*n as f32)),
            Db2Value::Decimal(d) => d
                .to_string()
                .parse::<f32>()
                .map(Some)
                .map_err(|e| ConversionError::ParseError {
                    target_type: "f32".into(),
                    message: e.to_string(),
                }),
            Db2Value::Text(s) => s
                .parse::<f32>()
                .map(Some)
                .map_err(|e| ConversionError::ParseError {
                    target_type: "f32".into(),
                    message: e.to_string(),
                }),
            other => Err(ConversionError::IncompatibleTypes {
                from: format!("{:?}", other.sql_type()),
                to: "f32".into(),
            }),
        }
    }

    /// Convert a Db2Value to f64
    pub fn to_f64(&self, value: &Db2Value) -> ConversionResult<Option<f64>> {
        match value {
            Db2Value::Null => Ok(None),
            Db2Value::Real(n) => Ok(Some(*n as f64)),
            Db2Value::Double(n) => Ok(Some(*n)),
            Db2Value::SmallInt(n) => Ok(Some(*n as f64)),
            Db2Value::Integer(n) => Ok(Some(*n as f64)),
            Db2Value::BigInt(n) => Ok(Some(*n as f64)),
            Db2Value::Decimal(d) => d
                .to_string()
                .parse::<f64>()
                .map(Some)
                .map_err(|e| ConversionError::ParseError {
                    target_type: "f64".into(),
                    message: e.to_string(),
                }),
            Db2Value::Text(s) => s
                .parse::<f64>()
                .map(Some)
                .map_err(|e| ConversionError::ParseError {
                    target_type: "f64".into(),
                    message: e.to_string(),
                }),
            other => Err(ConversionError::IncompatibleTypes {
                from: format!("{:?}", other.sql_type()),
                to: "f64".into(),
            }),
        }
    }

    /// Convert a Db2Value to Decimal
    pub fn to_decimal(&self, value: &Db2Value) -> ConversionResult<Option<Decimal>> {
        match value {
            Db2Value::Null => Ok(None),
            Db2Value::Decimal(d) => Ok(Some(*d)),
            Db2Value::SmallInt(n) => Ok(Some(Decimal::from(*n))),
            Db2Value::Integer(n) => Ok(Some(Decimal::from(*n))),
            Db2Value::BigInt(n) => Ok(Some(Decimal::from(*n))),
            Db2Value::Text(s) => s
                .parse::<Decimal>()
                .map(Some)
                .map_err(|e| ConversionError::ParseError {
                    target_type: "Decimal".into(),
                    message: e.to_string(),
                }),
            other => Err(ConversionError::IncompatibleTypes {
                from: format!("{:?}", other.sql_type()),
                to: "Decimal".into(),
            }),
        }
    }

    /// Convert a Db2Value to bytes
    pub fn to_bytes(&self, value: &Db2Value) -> ConversionResult<Option<Vec<u8>>> {
        match value {
            Db2Value::Null => Ok(None),
            Db2Value::Binary(b) => Ok(Some(b.clone())),
            Db2Value::Text(s) => Ok(Some(s.as_bytes().to_vec())),
            other => Err(ConversionError::IncompatibleTypes {
                from: format!("{:?}", other.sql_type()),
                to: "bytes".into(),
            }),
        }
    }

    /// Convert a Db2Value to bool
    pub fn to_bool(&self, value: &Db2Value) -> ConversionResult<Option<bool>> {
        match value {
            Db2Value::Null => Ok(None),
            Db2Value::Boolean(b) => Ok(Some(*b)),
            Db2Value::SmallInt(n) => Ok(Some(*n != 0)),
            Db2Value::Integer(n) => Ok(Some(*n != 0)),
            Db2Value::Text(s) => {
                let s = s.to_lowercase();
                match s.as_str() {
                    "t" | "true" | "y" | "yes" | "1" | "on" => Ok(Some(true)),
                    "f" | "false" | "n" | "no" | "0" | "off" => Ok(Some(false)),
                    _ => Err(ConversionError::ParseError {
                        target_type: "bool".into(),
                        message: format!("Invalid boolean value: {}", s),
                    }),
                }
            }
            other => Err(ConversionError::IncompatibleTypes {
                from: format!("{:?}", other.sql_type()),
                to: "bool".into(),
            }),
        }
    }

    /// Convert a Db2Value to NaiveDate
    pub fn to_date(&self, value: &Db2Value) -> ConversionResult<Option<NaiveDate>> {
        match value {
            Db2Value::Null => Ok(None),
            Db2Value::Date(d) => Ok(Some(*d)),
            Db2Value::Timestamp(ts) => Ok(Some(ts.date())),
            Db2Value::Text(s) => NaiveDate::parse_from_str(s, "%Y-%m-%d")
                .map(Some)
                .map_err(|e| ConversionError::ParseError {
                    target_type: "date".into(),
                    message: e.to_string(),
                }),
            other => Err(ConversionError::IncompatibleTypes {
                from: format!("{:?}", other.sql_type()),
                to: "date".into(),
            }),
        }
    }

    /// Convert a Db2Value to NaiveTime
    pub fn to_time(&self, value: &Db2Value) -> ConversionResult<Option<NaiveTime>> {
        match value {
            Db2Value::Null => Ok(None),
            Db2Value::Time(t) => Ok(Some(*t)),
            Db2Value::Timestamp(ts) => Ok(Some(ts.time())),
            Db2Value::Text(s) => NaiveTime::parse_from_str(s, "%H:%M:%S")
                .or_else(|_| NaiveTime::parse_from_str(s, "%H:%M:%S%.f"))
                .map(Some)
                .map_err(|e| ConversionError::ParseError {
                    target_type: "time".into(),
                    message: e.to_string(),
                }),
            other => Err(ConversionError::IncompatibleTypes {
                from: format!("{:?}", other.sql_type()),
                to: "time".into(),
            }),
        }
    }

    /// Convert a Db2Value to NaiveDateTime
    pub fn to_timestamp(&self, value: &Db2Value) -> ConversionResult<Option<NaiveDateTime>> {
        match value {
            Db2Value::Null => Ok(None),
            Db2Value::Timestamp(ts) => Ok(Some(*ts)),
            Db2Value::Date(d) => Ok(Some(d.and_hms_opt(0, 0, 0).unwrap())),
            Db2Value::Text(s) => {
                // Try multiple formats
                NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
                    .or_else(|_| NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S%.f"))
                    .or_else(|_| NaiveDateTime::parse_from_str(s, "%Y-%m-%d-%H.%M.%S"))
                    .or_else(|_| NaiveDateTime::parse_from_str(s, "%Y-%m-%d-%H.%M.%S%.f"))
                    .map(Some)
                    .map_err(|e| ConversionError::ParseError {
                        target_type: "timestamp".into(),
                        message: e.to_string(),
                    })
            }
            other => Err(ConversionError::IncompatibleTypes {
                from: format!("{:?}", other.sql_type()),
                to: "timestamp".into(),
            }),
        }
    }

    /// Get the appropriate PostgreSQL type OID for a DB2 SQL type
    pub fn pg_type_oid(sql_type: SqlType) -> u32 {
        sql_type.to_pg_oid()
    }
}

impl Default for TypeConverter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_string() {
        let converter = TypeConverter::new();

        assert_eq!(
            converter.to_string(&Db2Value::Text("hello".into())).unwrap(),
            Some("hello".into())
        );
        assert_eq!(
            converter.to_string(&Db2Value::Integer(42)).unwrap(),
            Some("42".into())
        );
        assert_eq!(converter.to_string(&Db2Value::Null).unwrap(), None);
    }

    #[test]
    fn test_to_i32() {
        let converter = TypeConverter::new();

        assert_eq!(
            converter.to_i32(&Db2Value::Integer(42)).unwrap(),
            Some(42)
        );
        assert_eq!(
            converter.to_i32(&Db2Value::SmallInt(10)).unwrap(),
            Some(10)
        );
        assert_eq!(converter.to_i32(&Db2Value::Null).unwrap(), None);
    }

    #[test]
    fn test_to_bool() {
        let converter = TypeConverter::new();

        assert_eq!(
            converter.to_bool(&Db2Value::Boolean(true)).unwrap(),
            Some(true)
        );
        assert_eq!(
            converter.to_bool(&Db2Value::Text("yes".into())).unwrap(),
            Some(true)
        );
        assert_eq!(
            converter.to_bool(&Db2Value::Text("no".into())).unwrap(),
            Some(false)
        );
    }

    #[test]
    fn test_range_error() {
        let converter = TypeConverter::new();

        let result = converter.to_i16(&Db2Value::Integer(100000));
        assert!(result.is_err());
    }
}
