//! DB2 Foreign Data Wrapper for PostgreSQL
//!
//! This is the main entry point for the FDW, providing the PostgreSQL extension
//! interface implemented in safe Rust.
//!
//! # Safety
//!
//! This crate replaces the C implementation to eliminate memory safety issues:
//! - No manual memory management (uses Rust's ownership)
//! - No dangling pointers (RAII-based resource management)
//! - No buffer overflows (safe string handling)
//! - No use-after-free (compile-time lifetime checking)
//!
//! # Threading Model
//!
//! PostgreSQL uses a multi-process architecture. Each backend is single-threaded,
//! so we use thread_local! + RefCell instead of thread-safe structures.

use pgrx::prelude::*;
use pgrx::pg_sys;

pub mod options;
pub mod scan;
pub mod modify;
pub mod explain;
pub mod import;
pub mod transaction;
pub mod state;
pub mod query;
pub mod deparsing;
pub mod safe_ffi;

use db2_connection::{close_all_connections, get_cache_stats};

// PostgreSQL extension magic
pg_module_magic!();

/// Extension version
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Extension name
pub const EXTENSION_NAME: &str = "db2_fdw";

// Re-export commonly used types
pub use options::{FdwOptions, OptionContext, validate_options};
pub use state::{FdwPlanState, FdwScanState, FdwModifyState};

/// Initialize the extension
#[no_mangle]
pub extern "C" fn _PG_init() {
    // Register transaction callbacks
    transaction::register_callbacks();

    // Log startup
    pgrx::log!("db2_fdw {} loaded", VERSION);
}

/// FDW Handler function
///
/// This is called by PostgreSQL to get the FDW callback routines.
/// It returns a fully initialized FdwRoutine with all callbacks.
///
/// Note: Not using #[pg_extern] because FDW handlers have special calling convention.
/// Using #[no_mangle] to export the symbol.
#[no_mangle]
pub unsafe extern "C" fn db2_fdw_handler(_fcinfo: pg_sys::FunctionCallInfo) -> pg_sys::Datum {
    use crate::scan::{
        get_foreign_rel_size, get_foreign_paths, get_foreign_join_paths,
        get_foreign_plan, analyze_foreign_table,
        begin_foreign_scan, iterate_foreign_scan, re_scan_foreign_scan, end_foreign_scan,
        explain_foreign_scan,
    };
    use crate::explain::explain_foreign_modify;
    use crate::modify::{
        add_foreign_update_targets, plan_foreign_modify,
        begin_foreign_modify, exec_foreign_insert, exec_foreign_update, exec_foreign_delete,
        end_foreign_modify,
        begin_foreign_insert, end_foreign_insert, exec_foreign_truncate,
        is_foreign_rel_updatable,
    };
    use crate::import::import_foreign_schema;

    unsafe {
        // Allocate memory for FdwRoutine using PostgreSQL's palloc
        let fdwroutine = pg_sys::palloc(std::mem::size_of::<pg_sys::FdwRoutine>()) as *mut pg_sys::FdwRoutine;

        if fdwroutine.is_null() {
            pgrx::error!("Failed to allocate memory for FdwRoutine");
        }

        // Initialize all fields to NULL (PostgreSQL convention)
        std::ptr::write_bytes(fdwroutine, 0, 1);

        // Planning callbacks
        (*fdwroutine).GetForeignRelSize = Some(get_foreign_rel_size);
        (*fdwroutine).GetForeignPaths = Some(get_foreign_paths);
        (*fdwroutine).GetForeignJoinPaths = Some(get_foreign_join_paths);
        (*fdwroutine).GetForeignPlan = Some(get_foreign_plan);
        (*fdwroutine).AnalyzeForeignTable = Some(analyze_foreign_table);

        // Execution callbacks - Scan
        (*fdwroutine).ExplainForeignScan = Some(explain_foreign_scan);
        (*fdwroutine).BeginForeignScan = Some(begin_foreign_scan);
        (*fdwroutine).IterateForeignScan = Some(iterate_foreign_scan);
        (*fdwroutine).ReScanForeignScan = Some(re_scan_foreign_scan);
        (*fdwroutine).EndForeignScan = Some(end_foreign_scan);

        // Execution callbacks - Modify
        (*fdwroutine).AddForeignUpdateTargets = Some(add_foreign_update_targets);
        (*fdwroutine).PlanForeignModify = Some(plan_foreign_modify);
        (*fdwroutine).BeginForeignModify = Some(begin_foreign_modify);
        (*fdwroutine).ExecForeignInsert = Some(exec_foreign_insert);
        (*fdwroutine).ExecForeignUpdate = Some(exec_foreign_update);
        (*fdwroutine).ExecForeignDelete = Some(exec_foreign_delete);
        (*fdwroutine).EndForeignModify = Some(end_foreign_modify);
        (*fdwroutine).ExplainForeignModify = Some(explain_foreign_modify);

        // Insert/Modify callbacks
        (*fdwroutine).BeginForeignInsert = Some(begin_foreign_insert);
        (*fdwroutine).EndForeignInsert = Some(end_foreign_insert);

        // Query control callbacks
        (*fdwroutine).ImportForeignSchema = Some(import_foreign_schema);
        (*fdwroutine).IsForeignRelUpdatable = Some(is_foreign_rel_updatable);

        // Truncate (PG14+)
        #[cfg(any(feature = "pg14", feature = "pg15", feature = "pg16", feature = "pg17", feature = "pg18"))]
        {
            (*fdwroutine).ExecForeignTruncate = Some(exec_foreign_truncate);
            // TODO: Implement batch insert support
            // (*fdwroutine).ExecForeignBatchInsert = Some(exec_foreign_batch_insert);
            // (*fdwroutine).GetForeignModifyBatchSize = Some(get_foreign_modify_batch_size);
        }

        // TODO: Add missing callbacks
        // - All callbacks now implemented except optional ones

        pg_sys::Datum::from(fdwroutine)
    }
}

/// Function info record for db2_fdw_handler
///
/// This is equivalent to the C macro PG_FUNCTION_INFO_V1(db2_fdw_handler).
/// PostgreSQL requires this metadata for all SQL-callable functions.
#[no_mangle]
pub extern "C" fn pg_finfo_db2_fdw_handler() -> pg_sys::Pg_finfo_record {
    pg_sys::Pg_finfo_record { api_version: 1 }
}

/// FDW Validator function
///
/// Validates options for foreign server, table, etc.
/// Not using #[pg_extern] due to special FDW calling convention.
///
/// Note: Simplified implementation - full validation requires parsing text[] array.
#[no_mangle]
pub unsafe extern "C" fn db2_fdw_validator(_fcinfo: pg_sys::FunctionCallInfo) -> pg_sys::Datum {
    // For now, just log and accept.
    // Full implementation would need to parse the text[] from PostgreSQL array API
    pgrx::info!("db2_fdw_validator called (validation TODO)");

    pg_sys::Datum::from(0i32)
}

/// Function info record for db2_fdw_validator
///
/// This is equivalent to the C macro PG_FUNCTION_INFO_V1(db2_fdw_validator).
/// PostgreSQL requires this metadata for all SQL-callable functions.
#[no_mangle]
pub extern "C" fn pg_finfo_db2_fdw_validator() -> pg_sys::Pg_finfo_record {
    pg_sys::Pg_finfo_record { api_version: 1 }
}

/// Close all DB2 connections
///
/// Utility function to close all cached connections in this backend.
/// Replaces the C db2_close_connections function.
///
/// Safety: Will error if there's an active DML transaction, to prevent
/// closing connections while modifications are in progress.
#[pg_extern(sql = "
CREATE OR REPLACE FUNCTION db2_close_connections()
RETURNS void
AS 'MODULE_PATHNAME', 'db2_close_connections'
LANGUAGE C STRICT;
")]
fn db2_close_connections() {
    use crate::transaction::is_dml_in_transaction;

    if is_dml_in_transaction() {
        pgrx::error!(
            "connections with an active transaction cannot be closed",
        );
    }

    pgrx::log!("Closing all DB2 connections");
    close_all_connections();
}

/// Diagnostic function
///
/// Returns diagnostic information about the FDW.
/// Replaces the C db2_diag function.
#[pg_extern(sql = "
CREATE OR REPLACE FUNCTION db2_diag()
RETURNS TABLE(name text, value text)
AS 'MODULE_PATHNAME', 'db2_diag'
LANGUAGE C STRICT;
")]
fn db2_diag() -> TableIterator<'static, (name!(name, String), name!(value, String))> {
    let stats = get_cache_stats();

    let diagnostics = vec![
        ("version".to_string(), VERSION.to_string()),
        ("extension_name".to_string(), EXTENSION_NAME.to_string()),
        ("connection_count".to_string(), stats.connection_count.to_string()),
        ("environment_count".to_string(), stats.environment_count.to_string()),
        ("total_use_count".to_string(), stats.total_use_count.to_string()),
    ];

    TableIterator::new(diagnostics)
}

#[cfg(any(test, feature = "pg_test"))]
#[pg_schema]
mod tests {
    use super::*;

    #[pg_test]
    fn test_extension_loads() {
        assert_eq!(EXTENSION_NAME, "db2_fdw");
    }

    #[pg_test]
    fn test_version() {
        assert!(!VERSION.is_empty());
    }
}

#[cfg(test)]
pub mod pg_test {
    pub fn setup(_options: Vec<&str>) {
        // Setup code for tests
    }

    pub fn postgresql_conf_options() -> Vec<&'static str> {
        vec![]
    }
}
