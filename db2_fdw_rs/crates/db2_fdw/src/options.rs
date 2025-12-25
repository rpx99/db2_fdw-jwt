//! FDW Option handling
//!
//! Parses and validates options for foreign servers, tables, and user mappings.

use pgrx::prelude::*;
use std::collections::HashMap;
use thiserror::Error;

use db2_connection::FdwConnectionOptions;
use db2_odbc::AuthMethod;

/// Errors during option validation
#[derive(Error, Debug)]
pub enum OptionError {
    #[error("Invalid option '{name}' for {context}")]
    InvalidOption { name: String, context: String },

    #[error("Missing required option '{name}' for {context}")]
    MissingRequired { name: String, context: String },

    #[error("Invalid value for option '{name}': {reason}")]
    InvalidValue { name: String, reason: String },

    #[error("Conflicting options: {0}")]
    ConflictingOptions(String),
}

/// Result type for option operations
pub type OptionResult<T> = Result<T, OptionError>;

/// Context where options are defined
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionContext {
    ForeignDataWrapper,
    ForeignServer,
    ForeignTable,
    UserMapping,
    AttributeMapping,
}

impl OptionContext {
    /// Get context from catalog OID
    pub fn from_catalog_oid(oid: pg_sys::Oid) -> Self {
        match oid {
            pg_sys::ForeignDataWrapperRelationId => OptionContext::ForeignDataWrapper,
            pg_sys::ForeignServerRelationId => OptionContext::ForeignServer,
            pg_sys::ForeignTableRelationId => OptionContext::ForeignTable,
            pg_sys::UserMappingRelationId => OptionContext::UserMapping,
            pg_sys::AttributeRelationId => OptionContext::AttributeMapping,
            _ => OptionContext::ForeignTable, // Default
        }
    }

    /// Get display name
    pub fn name(&self) -> &'static str {
        match self {
            OptionContext::ForeignDataWrapper => "foreign data wrapper",
            OptionContext::ForeignServer => "foreign server",
            OptionContext::ForeignTable => "foreign table",
            OptionContext::UserMapping => "user mapping",
            OptionContext::AttributeMapping => "column",
        }
    }
}

/// All FDW options with their metadata
#[derive(Debug, Clone)]
pub struct OptionDef {
    pub name: &'static str,
    pub contexts: &'static [OptionContext],
    pub required: bool,
    pub default: Option<&'static str>,
    pub description: &'static str,
}

/// All supported options
pub static OPTIONS: &[OptionDef] = &[
    OptionDef {
        name: "dbserver",
        contexts: &[OptionContext::ForeignServer],
        required: true,
        default: None,
        description: "DB2 DSN or connection string",
    },
    OptionDef {
        name: "table",
        contexts: &[OptionContext::ForeignTable],
        required: true,
        default: None,
        description: "Remote table name",
    },
    OptionDef {
        name: "schema",
        contexts: &[OptionContext::ForeignTable],
        required: false,
        default: None,
        description: "Remote schema name",
    },
    OptionDef {
        name: "user",
        contexts: &[OptionContext::UserMapping],
        required: false,
        default: None,
        description: "DB2 username",
    },
    OptionDef {
        name: "password",
        contexts: &[OptionContext::UserMapping],
        required: false,
        default: None,
        description: "DB2 password",
    },
    OptionDef {
        name: "jwt_token",
        contexts: &[OptionContext::UserMapping],
        required: false,
        default: None,
        description: "JWT token for authentication (alternative to user/password)",
    },
    OptionDef {
        name: "nls_lang",
        contexts: &[OptionContext::ForeignDataWrapper, OptionContext::ForeignServer],
        required: false,
        default: None,
        description: "NLS language setting (e.g., 'en_US.UTF-8')",
    },
    OptionDef {
        name: "max_long",
        contexts: &[OptionContext::ForeignTable],
        required: false,
        default: Some("32767"),
        description: "Maximum size for LONG columns",
    },
    OptionDef {
        name: "readonly",
        contexts: &[OptionContext::ForeignTable],
        required: false,
        default: Some("off"),
        description: "Treat table as read-only",
    },
    OptionDef {
        name: "key",
        contexts: &[OptionContext::AttributeMapping],
        required: false,
        default: Some("off"),
        description: "Mark column as part of primary key",
    },
    OptionDef {
        name: "sample_percent",
        contexts: &[OptionContext::ForeignTable],
        required: false,
        default: None,
        description: "Percentage of rows to sample for ANALYZE",
    },
    OptionDef {
        name: "prefetch",
        contexts: &[OptionContext::ForeignTable, OptionContext::ForeignServer],
        required: false,
        default: Some("200"),
        description: "Number of rows to prefetch",
    },
    OptionDef {
        name: "batch_size",
        contexts: &[OptionContext::ForeignServer, OptionContext::ForeignTable],
        required: false,
        default: Some("1"),
        description: "Batch size for INSERT operations",
    },
    OptionDef {
        name: "no_encoding_error",
        contexts: &[
            OptionContext::ForeignDataWrapper,
            OptionContext::ForeignTable,
            OptionContext::AttributeMapping,
        ],
        required: false,
        default: Some("off"),
        description: "Handle encoding errors gracefully",
    },
    OptionDef {
        name: "strip_zeros",
        contexts: &[OptionContext::ForeignTable, OptionContext::AttributeMapping],
        required: false,
        default: Some("off"),
        description: "Strip trailing zeros from DECIMAL columns",
    },
];

/// Parsed FDW options
#[derive(Debug, Clone, Default)]
pub struct FdwOptions {
    // Server options
    pub dbserver: Option<String>,
    pub nls_lang: Option<String>,
    pub prefetch: usize,
    pub batch_size: usize,

    // Table options
    pub table: Option<String>,
    pub schema: Option<String>,
    pub max_long: usize,
    pub readonly: bool,
    pub sample_percent: Option<f64>,
    pub no_encoding_error: bool,
    pub strip_zeros: bool,

    // User mapping options
    pub user: Option<String>,
    pub password: Option<String>,
    pub jwt_token: Option<String>,

    // Column options (keyed by column name)
    pub key_columns: Vec<String>,
}

impl FdwOptions {
    /// Create new options with defaults
    pub fn new() -> Self {
        Self {
            prefetch: 200,
            batch_size: 1,
            max_long: 32767,
            readonly: false,
            no_encoding_error: false,
            strip_zeros: false,
            ..Default::default()
        }
    }

    /// Parse options from a key-value map
    pub fn parse(options: &HashMap<String, String>) -> OptionResult<Self> {
        let mut opts = Self::new();

        for (key, value) in options {
            match key.as_str() {
                "dbserver" => opts.dbserver = Some(value.clone()),
                "table" => opts.table = Some(value.clone()),
                "schema" => opts.schema = Some(value.clone()),
                "user" => opts.user = Some(value.clone()),
                "password" => opts.password = Some(value.clone()),
                "jwt_token" => opts.jwt_token = Some(value.clone()),
                "nls_lang" => opts.nls_lang = Some(value.clone()),
                "max_long" => {
                    opts.max_long = value.parse().map_err(|_| OptionError::InvalidValue {
                        name: key.clone(),
                        reason: "must be a positive integer".into(),
                    })?;
                }
                "readonly" => {
                    opts.readonly = parse_bool(value).ok_or_else(|| OptionError::InvalidValue {
                        name: key.clone(),
                        reason: "must be 'on' or 'off'".into(),
                    })?;
                }
                "prefetch" => {
                    opts.prefetch = value.parse().map_err(|_| OptionError::InvalidValue {
                        name: key.clone(),
                        reason: "must be a positive integer".into(),
                    })?;
                }
                "batch_size" => {
                    opts.batch_size = value.parse().map_err(|_| OptionError::InvalidValue {
                        name: key.clone(),
                        reason: "must be a positive integer".into(),
                    })?;
                }
                "sample_percent" => {
                    let pct: f64 = value.parse().map_err(|_| OptionError::InvalidValue {
                        name: key.clone(),
                        reason: "must be a number between 0 and 100".into(),
                    })?;
                    if !(0.0..=100.0).contains(&pct) {
                        return Err(OptionError::InvalidValue {
                            name: key.clone(),
                            reason: "must be between 0 and 100".into(),
                        });
                    }
                    opts.sample_percent = Some(pct);
                }
                "no_encoding_error" => {
                    opts.no_encoding_error =
                        parse_bool(value).ok_or_else(|| OptionError::InvalidValue {
                            name: key.clone(),
                            reason: "must be 'on' or 'off'".into(),
                        })?;
                }
                "strip_zeros" => {
                    opts.strip_zeros =
                        parse_bool(value).ok_or_else(|| OptionError::InvalidValue {
                            name: key.clone(),
                            reason: "must be 'on' or 'off'".into(),
                        })?;
                }
                "key" => {
                    if parse_bool(value).unwrap_or(false) {
                        // This is set per-column, handled separately
                    }
                }
                _ => {
                    // Unknown option - will be caught by validator
                }
            }
        }

        Ok(opts)
    }

    /// Get the authentication method
    pub fn auth_method(&self) -> Option<AuthMethod> {
        if let Some(ref token) = self.jwt_token {
            Some(AuthMethod::jwt(token))
        } else if let (Some(ref user), Some(ref pass)) = (&self.user, &self.password) {
            Some(AuthMethod::password(user, pass))
        } else {
            None
        }
    }

    /// Convert to connection options
    pub fn to_connection_options(&self) -> Option<FdwConnectionOptions> {
        let server = self.dbserver.as_ref()?;
        let auth = self.auth_method()?;

        let mut opts = match auth {
            AuthMethod::Password { user, password } => {
                FdwConnectionOptions::with_password(server, user, password)
            }
            AuthMethod::JwtToken { token } => FdwConnectionOptions::with_jwt(server, token),
        };

        if let Some(ref nls) = self.nls_lang {
            opts = opts.nls_lang(nls);
        }

        opts = opts.read_only(self.readonly).prefetch(self.prefetch);

        Some(opts)
    }
}

/// Validate options for a given context
pub fn validate_options(options: &[String], context: OptionContext) -> OptionResult<()> {
    // Parse options into key-value pairs
    let parsed: HashMap<String, String> = options
        .chunks(2)
        .filter_map(|chunk| {
            if chunk.len() == 2 {
                Some((chunk[0].clone(), chunk[1].clone()))
            } else {
                None
            }
        })
        .collect();

    // Check each option is valid for this context
    for key in parsed.keys() {
        let valid = OPTIONS.iter().any(|opt| opt.name == key && opt.contexts.contains(&context));

        if !valid {
            return Err(OptionError::InvalidOption {
                name: key.clone(),
                context: context.name().into(),
            });
        }
    }

    // For user mapping, check authentication options
    if context == OptionContext::UserMapping {
        let has_jwt = parsed.contains_key("jwt_token");
        let has_user = parsed.contains_key("user");
        let has_pass = parsed.contains_key("password");

        // Must have either JWT or (user AND password)
        if has_jwt && (has_user || has_pass) {
            return Err(OptionError::ConflictingOptions(
                "Cannot use both jwt_token and user/password authentication".into(),
            ));
        }

        if !has_jwt && (!has_user || !has_pass) && (has_user || has_pass) {
            return Err(OptionError::MissingRequired {
                name: if !has_user { "user" } else { "password" }.into(),
                context: context.name().into(),
            });
        }
    }

    Ok(())
}

/// Parse a boolean option value
fn parse_bool(value: &str) -> Option<bool> {
    match value.to_lowercase().as_str() {
        "on" | "true" | "yes" | "1" => Some(true),
        "off" | "false" | "no" | "0" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_bool() {
        assert_eq!(parse_bool("on"), Some(true));
        assert_eq!(parse_bool("off"), Some(false));
        assert_eq!(parse_bool("TRUE"), Some(true));
        assert_eq!(parse_bool("invalid"), None);
    }

    #[test]
    fn test_options_parse() {
        let mut map = HashMap::new();
        map.insert("dbserver".into(), "mydb".into());
        map.insert("table".into(), "mytable".into());
        map.insert("prefetch".into(), "100".into());
        map.insert("readonly".into(), "on".into());

        let opts = FdwOptions::parse(&map).unwrap();
        assert_eq!(opts.dbserver, Some("mydb".into()));
        assert_eq!(opts.table, Some("mytable".into()));
        assert_eq!(opts.prefetch, 100);
        assert!(opts.readonly);
    }

    #[test]
    fn test_auth_method() {
        let mut opts = FdwOptions::new();
        opts.user = Some("user".into());
        opts.password = Some("pass".into());
        assert!(matches!(opts.auth_method(), Some(AuthMethod::Password { .. })));

        let mut opts = FdwOptions::new();
        opts.jwt_token = Some("token".into());
        assert!(matches!(opts.auth_method(), Some(AuthMethod::JwtToken { .. })));
    }
}
