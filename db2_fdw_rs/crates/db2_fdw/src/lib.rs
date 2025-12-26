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

pub mod options;
pub mod scan;
pub mod modify;
pub mod explain;
pub mod import;
pub mod transaction;
pub mod state;
pub mod query;

use db2_connection::{close_all_connections, get_cache_stats};

// PostgreSQL extension magic
pg_module_magic!();

/// Extension version
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Extension name
pub const EXTENSION_NAME: &str = "db2_fdw";

// Re-export commonly used types
pub use options::{FdwOptions, OptionContext, validate_options};
pub use scan::ForeignScan;
pub use modify::ForeignModify;
pub use state::FdwState;

/// Initialize the extension
#[pg_guard]
pub extern "C" fn _PG_init() {
    // Register transaction callbacks
    transaction::register_callbacks();

    // Log startup
    pgrx::log!("db2_fdw {} loaded", VERSION);
}

/// FDW Handler function
///
/// This is called by PostgreSQL to get the FDW callback routines.
/// It replaces the C db2_fdw_handler function.
#[pg_extern(sql = "
CREATE OR REPLACE FUNCTION db2_fdw_handler()
RETURNS fdw_handler
AS 'MODULE_PATHNAME', 'db2_fdw_handler'
LANGUAGE C STRICT;
")]
fn db2_fdw_handler() -> pg_sys::FdwRoutine {
    let mut routine = pg_sys::FdwRoutine::default();

    // Query planning callbacks
    routine.GetForeignRelSize = Some(scan::get_foreign_rel_size);
    routine.GetForeignPaths = Some(scan::get_foreign_paths);
    routine.GetForeignPlan = Some(scan::get_foreign_plan);

    // Scan execution callbacks
    routine.BeginForeignScan = Some(scan::begin_foreign_scan);
    routine.IterateForeignScan = Some(scan::iterate_foreign_scan);
    routine.ReScanForeignScan = Some(scan::rescan_foreign_scan);
    routine.EndForeignScan = Some(scan::end_foreign_scan);

    // Modification callbacks
    routine.AddForeignUpdateTargets = Some(modify::add_foreign_update_targets);
    routine.PlanForeignModify = Some(modify::plan_foreign_modify);
    routine.BeginForeignModify = Some(modify::begin_foreign_modify);
    routine.ExecForeignInsert = Some(modify::exec_foreign_insert);
    routine.ExecForeignUpdate = Some(modify::exec_foreign_update);
    routine.ExecForeignDelete = Some(modify::exec_foreign_delete);
    routine.EndForeignModify = Some(modify::end_foreign_modify);

    // Batch insert support (PostgreSQL 14+)
    #[cfg(feature = "pg14")]
    {
        routine.GetForeignModifyBatchSize = Some(modify::get_foreign_modify_batch_size);
        routine.ExecForeignBatchInsert = Some(modify::exec_foreign_batch_insert);
    }

    // Truncate support
    routine.ExecForeignTruncate = Some(modify::exec_foreign_truncate);

    // EXPLAIN support
    routine.ExplainForeignScan = Some(explain::explain_foreign_scan);
    routine.ExplainForeignModify = Some(explain::explain_foreign_modify);

    // ANALYZE support
    routine.AnalyzeForeignTable = Some(scan::analyze_foreign_table);

    // IMPORT FOREIGN SCHEMA support
    routine.ImportForeignSchema = Some(import::import_foreign_schema);

    // Updateability check
    routine.IsForeignRelUpdatable = Some(modify::is_foreign_rel_updatable);

    // Join pushdown (PostgreSQL 9.6+)
    routine.GetForeignJoinPaths = Some(scan::get_foreign_join_paths);

    routine
}

/// FDW Validator function
///
/// Validates options for foreign servers, tables, and user mappings.
/// This replaces the C db2_fdw_validator function.
#[pg_extern(sql = "
CREATE OR REPLACE FUNCTION db2_fdw_validator(text[], oid)
RETURNS void
AS 'MODULE_PATHNAME', 'db2_fdw_validator'
LANGUAGE C STRICT;
")]
fn db2_fdw_validator(options: Vec<String>, catalog: pg_sys::Oid) {
    let context = OptionContext::from_catalog_oid(catalog);

    if let Err(e) = validate_options(&options, context) {
        pgrx::error!("{}", e);
    }
}

/// Close all DB2 connections
///
/// Utility function to close all cached connections in this backend.
/// Replaces the C db2_close_connections function.
#[pg_extern(sql = "
CREATE OR REPLACE FUNCTION db2_close_connections()
RETURNS void
AS 'MODULE_PATHNAME', 'db2_close_connections'
LANGUAGE C STRICT;
")]
fn db2_close_connections() {
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
