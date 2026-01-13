//! IMPORT FOREIGN SCHEMA implementation
//!
//! Imports table definitions from DB2 into PostgreSQL.

use pgrx::pg_sys;

/// Case folding options for identifiers
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CaseFolding {
    /// Keep original case
    Keep,
    /// Fold to lowercase
    Lower,
    /// Smart folding: lowercase if all uppercase, else keep
    Smart,
}

/// Import a foreign schema
///
/// PostgreSQL FDW callback: ImportForeignSchema
#[allow(non_snake_case)]
pub unsafe extern "C-unwind" fn import_foreign_schema(
    stmt: *mut pg_sys::ImportForeignSchemaStmt,
    serverOid: pg_sys::Oid,
) -> *mut pg_sys::List {
    // debug!("import_foreign_schema called");

    unsafe {
        if stmt.is_null() {
            return std::ptr::null_mut();
        }

        // Get the remote schema name
        let remote_schema = if (*stmt).remote_schema.is_null() {
            return std::ptr::null_mut();
        } else {
            std::ffi::CStr::from_ptr((*stmt).remote_schema)
                .to_string_lossy()
                .to_string()
        };

        // Get the local schema name
        let local_schema = if (*stmt).local_schema.is_null() {
            "public".to_string()
        } else {
            std::ffi::CStr::from_ptr((*stmt).local_schema)
                .to_string_lossy()
                .to_string()
        };

        // Get the server and its options
        let server = pg_sys::GetForeignServer(serverOid);
        if server.is_null() {
            pgrx::error!("Could not get foreign server");
        }

        let server_name = std::ffi::CStr::from_ptr((*server).servername)
            .to_string_lossy()
            .to_string();

        // Get user mapping
        let user_id = pg_sys::GetUserId();
        let mapping = pg_sys::GetUserMapping(user_id, serverOid);

        // Get FDW
        let wrapper = pg_sys::GetForeignDataWrapper((*server).fdwid);
        if wrapper.is_null() {
            pgrx::error!("Could not get foreign data wrapper");
        }

        // Collect all options from wrapper, server, and user mapping
        let mut dbserver: Option<String> = None;
        let mut user: Option<String> = None;
        let mut password: Option<String> = None;
        let mut jwt_token: Option<String> = None;
        let mut nls_lang: Option<String> = None;
        let mut case_folding = CaseFolding::Smart;
        let mut readonly = false;

        // Parse wrapper options
        parse_options((*wrapper).options, &mut dbserver, &mut user, &mut password, &mut jwt_token, &mut nls_lang);

        // Parse server options (override wrapper)
        parse_options((*server).options, &mut dbserver, &mut user, &mut password, &mut jwt_token, &mut nls_lang);

        // Parse user mapping options (override server)
        if !mapping.is_null() {
            parse_options((*mapping).options, &mut dbserver, &mut user, &mut password, &mut jwt_token, &mut nls_lang);
        }

        // Parse IMPORT FOREIGN SCHEMA statement options
        let stmt_options = (*stmt).options;
        if !stmt_options.is_null() {
            let list_len = (*stmt_options).length;
            for i in 0..list_len {
                let cell = pg_sys::list_nth_cell(stmt_options, i);
                if cell.is_null() {
                    continue;
                }

                let def = (*cell).ptr_value as *mut pg_sys::DefElem;
                if def.is_null() || (*def).defname.is_null() {
                    continue;
                }

                let defname = std::ffi::CStr::from_ptr((*def).defname)
                    .to_string_lossy()
                    .to_lowercase();

                let defval = if (*def).arg.is_null() {
                    String::new()
                } else {
                    let val = (*def).arg as *mut pg_sys::String;
                    if !val.is_null() && !(*val).sval.is_null() {
                        std::ffi::CStr::from_ptr((*val).sval)
                            .to_string_lossy()
                            .to_string()
                    } else {
                        String::new()
                    }
                };

                match defname.as_str() {
                    "case" => {
                        case_folding = match defval.to_lowercase().as_str() {
                            "keep" => CaseFolding::Keep,
                            "lower" => CaseFolding::Lower,
                            "smart" => CaseFolding::Smart,
                            _ => {
                                pgrx::error!("invalid value for option \"case\": valid values are keep, lower, smart");
                            }
                        };
                    }
                    "readonly" => {
                        readonly = matches!(defval.to_lowercase().as_str(), "on" | "true" | "yes" | "1");
                    }
                    _ => {
                        pgrx::error!("invalid option \"{}\": valid options are case, readonly", defname);
                    }
                }
            }
        }

        // info!(
        //     remote_schema = %remote_schema,
        //     local_schema = %local_schema,
        //     server = %server_name,
        //     "Importing foreign schema"
        // );

        // Get import type and table list
        let import_type = (*stmt).list_type;
        let table_list = (*stmt).table_list;
        let mut limit_tables: Vec<String> = Vec::new();

        if !table_list.is_null() {
            let list_len = (*table_list).length;
            for i in 0..list_len {
                let cell = pg_sys::list_nth_cell(table_list, i);
                if cell.is_null() {
                    continue;
                }

                let rv = (*cell).ptr_value as *mut pg_sys::RangeVar;
                if rv.is_null() || (*rv).relname.is_null() {
                    continue;
                }

                let table_name = std::ffi::CStr::from_ptr((*rv).relname)
                    .to_string_lossy()
                    .to_string();

                limit_tables.push(table_name);
            }
        }

        // Build table list filter for SQL
        let table_filter = if limit_tables.is_empty() {
            String::new()
        } else {
            let quoted: Vec<String> = limit_tables.iter()
                .map(|t| format!("'{}'", t.replace('\'', "''")))
                .collect();
            format!(" AND TABNAME IN ({})", quoted.join(", "))
        };

        // Try to connect to DB2 and query the catalog
        let connection_string = dbserver.unwrap_or_default();
        if connection_string.is_empty() {
            pgrx::error!("dbserver option is required for IMPORT FOREIGN SCHEMA");
        }

        // CRITICAL DEBUG: Track progress to narrow down malloc issue
        pgrx::info!("Starting import for schema: {} tables: {}", remote_schema, limit_tables.len());

        // Connect and query
        let mut result: *mut pg_sys::List = std::ptr::null_mut();

        match try_import_schema(
            &connection_string,
            user.as_deref(),
            password.as_deref(),
            &remote_schema,
            &local_schema,
            &server_name,
            &table_filter,
            import_type as i32,
            &limit_tables,
            case_folding,
            readonly,
        ) {
            Ok(commands) => {
                // Convert commands to PostgreSQL list
                for cmd in commands {
                    let cstr = std::ffi::CString::new(cmd.as_str()).unwrap_or_default();
                    let pg_str = pg_sys::pstrdup(cstr.as_ptr());
                    if pg_str.is_null() {
                        pgrx::error!("Failed to allocate memory for import command");
                    }
                    result = crate::safe_lappend!(result, pg_str);
                }

                if !result.is_null() {
                    // info!("Import foreign schema complete, {} tables imported", (*result).length);
                }
            }
            Err(e) => {
                pgrx::error!("Failed to import foreign schema: {}", e);
            }
        }

        result
    }
}

/// Parse option list into connection parameters
unsafe fn parse_options(
    options: *mut pg_sys::List,
    dbserver: &mut Option<String>,
    user: &mut Option<String>,
    password: &mut Option<String>,
    jwt_token: &mut Option<String>,
    nls_lang: &mut Option<String>,
) {
    if options.is_null() {
        return;
    }

    let list_len = (*options).length;
    for i in 0..list_len {
        let cell = pg_sys::list_nth_cell(options, i);
        if cell.is_null() {
            continue;
        }

        let def = (*cell).ptr_value as *mut pg_sys::DefElem;
        if def.is_null() || (*def).defname.is_null() {
            continue;
        }

        let defname = std::ffi::CStr::from_ptr((*def).defname)
            .to_string_lossy()
            .to_lowercase();

        let defval = if (*def).arg.is_null() {
            String::new()
        } else {
            let val = (*def).arg as *mut pg_sys::String;
            if !val.is_null() && !(*val).sval.is_null() {
                std::ffi::CStr::from_ptr((*val).sval)
                    .to_string_lossy()
                    .to_string()
            } else {
                String::new()
            }
        };

        match defname.as_str() {
            "dbserver" => *dbserver = Some(defval),
            "user" => *user = Some(defval),
            "password" => *password = Some(defval),
            "jwt_token" => *jwt_token = Some(defval),
            "nls_lang" => *nls_lang = Some(defval),
            _ => {}
        }
    }
}

/// Try to import schema from DB2
fn try_import_schema(
    _connection_string: &str,
    _user: Option<&str>,
    _password: Option<&str>,
    remote_schema: &str,
    local_schema: &str,
    server_name: &str,
    table_filter: &str,
    import_type: i32,  // ImportForeignSchemaType is now i32 in PG18
    limit_tables: &[String],
    case_folding: CaseFolding,
    readonly: bool,
) -> Result<Vec<String>, String> {
    // Query to get tables
    let _tables_sql = format!(
        "SELECT TABNAME FROM SYSCAT.TABLES WHERE TABSCHEMA = '{}' AND TYPE = 'T'{}",
        remote_schema.replace('\'', "''"),
        table_filter
    );

    // TODO: Debug logging disabled until DB2 connection is stable
    // // debug!(sql = %tables_sql, "Querying DB2 catalog for tables");

    // For now, we'll return a placeholder indicating what would happen
    // In production, this would:
    // 1. Execute the tables query
    // 2. For each table, query SYSCAT.COLUMNS
    // 3. Build CREATE FOREIGN TABLE statements

    let mut commands = Vec::new();

    // Import foreign schema type constants (from PostgreSQL enum)
    const _FDW_IMPORT_SCHEMA_ALL: i32 = 0;
    const FDW_IMPORT_SCHEMA_LIMIT_TO: i32 = 1;
    const _FDW_IMPORT_SCHEMA_EXCEPT: i32 = 2;

    // If we have limit_tables and it's LIMIT_TO, use those
    let tables_to_import: Vec<&str> = if import_type == FDW_IMPORT_SCHEMA_LIMIT_TO {
        limit_tables.iter().map(|s| s.as_str()).collect()
    } else {
        // Would come from the query
        Vec::new()
    };

    // Build CREATE FOREIGN TABLE for each table
    for table_name in tables_to_import {
        // Query columns
        let _columns_sql = format!(
            "SELECT COLNAME, TYPENAME, NULLS, LENGTH, SCALE FROM SYSCAT.COLUMNS \
             WHERE TABSCHEMA = '{}' AND TABNAME = '{}' ORDER BY COLNO",
            remote_schema.replace('\'', "''"),
            table_name.replace('\'', "''")
        );

        // debug!(sql = %columns_sql, "Would query columns for table {}", table_name);

        // Fold the table name
        let folded_name = fold_case(table_name, case_folding);

        // Build a skeleton CREATE FOREIGN TABLE (columns would come from SYSCAT.COLUMNS)
        let mut create_sql = format!(
            "CREATE FOREIGN TABLE \"{}\".\"{}\" (\n",
            local_schema.replace('"', "\"\""),
            folded_name.replace('"', "\"\"")
        );

        // Add placeholder column (in production, this comes from SYSCAT.COLUMNS)
        create_sql.push_str("    -- Columns would be generated from SYSCAT.COLUMNS query\n");
        create_sql.push_str("    id integer NOT NULL\n");

        create_sql.push_str(&format!(
            ") SERVER \"{}\" OPTIONS (\n    schema '{}',\n    table '{}'",
            server_name.replace('"', "\"\""),
            remote_schema.replace('\'', "''"),
            table_name.replace('\'', "''")
        ));

        if readonly {
            create_sql.push_str(",\n    readonly 'true'");
        }

        create_sql.push_str("\n)");

        commands.push(create_sql);
    }

    // info!("Generated {} CREATE FOREIGN TABLE statements", commands.len());
    Ok(commands)
}

/// Fold identifier case according to the specified option
fn fold_case(name: &str, folding: CaseFolding) -> String {
    match folding {
        CaseFolding::Keep => name.to_string(),
        CaseFolding::Lower => name.to_lowercase(),
        CaseFolding::Smart => {
            // If all uppercase, convert to lowercase; otherwise keep original
            if name.chars().all(|c| !c.is_alphabetic() || c.is_uppercase()) {
                name.to_lowercase()
            } else {
                name.to_string()
            }
        }
    }
}

/// Column definition from DB2 catalog
#[derive(Debug, Clone)]
pub struct Db2Column {
    pub name: String,
    pub db2_type: String,
    pub nullable: bool,
    pub precision: Option<u32>,
    pub scale: Option<i16>,
    pub char_length: Option<u32>,
}

impl Db2Column {
    /// Convert DB2 type to PostgreSQL type
    pub fn to_pg_type(&self) -> String {
        match self.db2_type.to_uppercase().as_str() {
            "SMALLINT" => "smallint".into(),
            "INTEGER" | "INT" => "integer".into(),
            "BIGINT" => "bigint".into(),
            "REAL" => "real".into(),
            "DOUBLE" | "FLOAT" => "double precision".into(),
            "DECIMAL" | "NUMERIC" => {
                match (self.precision, self.scale) {
                    (Some(p), Some(s)) => format!("numeric({}, {})", p, s),
                    (Some(p), None) => format!("numeric({})", p),
                    _ => "numeric".into(),
                }
            }
            "CHAR" | "CHARACTER" => {
                match self.char_length {
                    Some(len) => format!("character({})", len),
                    None => "character(1)".into(),
                }
            }
            "VARCHAR" | "CHARACTER VARYING" => {
                match self.char_length {
                    Some(len) => format!("character varying({})", len),
                    None => "text".into(),
                }
            }
            "CLOB" | "LONG VARCHAR" => "text".into(),
            "BLOB" | "LONG VARBINARY" | "BINARY" | "VARBINARY" => "bytea".into(),
            "DATE" => "date".into(),
            "TIME" => "time".into(),
            "TIMESTAMP" => "timestamp".into(),
            "BOOLEAN" => "boolean".into(),
            "XML" => "xml".into(),
            _ => "text".into(), // Default to text for unknown types
        }
    }

    /// Generate column definition SQL
    pub fn to_sql(&self) -> String {
        let pg_type = self.to_pg_type();
        let null_constraint = if self.nullable { "" } else { " NOT NULL" };
        format!("    {} {}{}", quote_identifier(&self.name), pg_type, null_constraint)
    }
}

/// Table definition from DB2 catalog
#[derive(Debug, Clone)]
pub struct Db2Table {
    pub schema: String,
    pub name: String,
    pub columns: Vec<Db2Column>,
    pub primary_key: Vec<String>,
}

impl Db2Table {
    /// Generate CREATE FOREIGN TABLE statement
    pub fn to_create_statement(&self, server_name: &str, local_schema: &str) -> String {
        let mut sql = format!(
            "CREATE FOREIGN TABLE {}.{} (\n",
            quote_identifier(local_schema),
            quote_identifier(&self.name)
        );

        let column_defs: Vec<String> = self.columns.iter().map(|c| c.to_sql()).collect();
        sql.push_str(&column_defs.join(",\n"));

        sql.push_str("\n) SERVER ");
        sql.push_str(&quote_identifier(server_name));
        sql.push_str(" OPTIONS (\n");
        sql.push_str(&format!("    schema '{}',\n", escape_string(&self.schema)));
        sql.push_str(&format!("    table '{}'\n", escape_string(&self.name)));
        sql.push(')');

        sql
    }
}

/// Quote an identifier for SQL
fn quote_identifier(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Escape a string for SQL options
fn escape_string(s: &str) -> String {
    s.replace('\'', "''")
}

/// Query to get table columns from DB2 catalog
pub const GET_COLUMNS_QUERY: &str = r#"
SELECT
    COLNAME,
    TYPENAME,
    NULLS,
    LENGTH,
    SCALE
FROM SYSCAT.COLUMNS
WHERE TABSCHEMA = ?
  AND TABNAME = ?
ORDER BY COLNO
"#;

/// Query to get primary key columns from DB2 catalog
pub const GET_PRIMARY_KEY_QUERY: &str = r#"
SELECT
    COLNAME
FROM SYSCAT.KEYCOLUSE
WHERE TABSCHEMA = ?
  AND TABNAME = ?
  AND CONSTNAME = (
    SELECT CONSTNAME
    FROM SYSCAT.TABCONST
    WHERE TABSCHEMA = ?
      AND TABNAME = ?
      AND TYPE = 'P'
  )
ORDER BY COLSEQ
"#;

/// Query to list tables in a schema
pub const LIST_TABLES_QUERY: &str = r#"
SELECT
    TABNAME
FROM SYSCAT.TABLES
WHERE TABSCHEMA = ?
  AND TYPE = 'T'
ORDER BY TABNAME
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_column_to_pg_type() {
        let col = Db2Column {
            name: "ID".into(),
            db2_type: "INTEGER".into(),
            nullable: false,
            precision: None,
            scale: None,
            char_length: None,
        };
        assert_eq!(col.to_pg_type(), "integer");

        let col = Db2Column {
            name: "AMOUNT".into(),
            db2_type: "DECIMAL".into(),
            nullable: true,
            precision: Some(10),
            scale: Some(2),
            char_length: None,
        };
        assert_eq!(col.to_pg_type(), "numeric(10, 2)");

        let col = Db2Column {
            name: "NAME".into(),
            db2_type: "VARCHAR".into(),
            nullable: true,
            precision: None,
            scale: None,
            char_length: Some(100),
        };
        assert_eq!(col.to_pg_type(), "character varying(100)");
    }

    #[test]
    fn test_quote_identifier() {
        assert_eq!(quote_identifier("table"), "\"table\"");
        assert_eq!(quote_identifier("has\"quote"), "\"has\"\"quote\"");
    }

    #[test]
    fn test_table_to_create_statement() {
        let table = Db2Table {
            schema: "MYSCHEMA".into(),
            name: "MYTABLE".into(),
            columns: vec![
                Db2Column {
                    name: "ID".into(),
                    db2_type: "INTEGER".into(),
                    nullable: false,
                    precision: None,
                    scale: None,
                    char_length: None,
                },
                Db2Column {
                    name: "NAME".into(),
                    db2_type: "VARCHAR".into(),
                    nullable: true,
                    precision: None,
                    scale: None,
                    char_length: Some(50),
                },
            ],
            primary_key: vec!["ID".into()],
        };

        let sql = table.to_create_statement("db2_server", "public");
        assert!(sql.contains("CREATE FOREIGN TABLE"));
        assert!(sql.contains("\"ID\" integer NOT NULL"));
        assert!(sql.contains("\"NAME\" character varying(50)"));
        assert!(sql.contains("schema 'MYSCHEMA'"));
    }
}
