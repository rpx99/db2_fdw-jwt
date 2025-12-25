//! EXPLAIN support for foreign scans and modifications

use pgrx::prelude::*;
use pgrx::pg_sys;

use crate::state::{FdwScanState, FdwModifyState};

/// Explain a foreign scan
///
/// PostgreSQL FDW callback: ExplainForeignScan
#[pg_guard]
pub extern "C" fn explain_foreign_scan(
    node: *mut pg_sys::ForeignScanState,
    es: *mut pg_sys::ExplainState,
) {
    unsafe {
        let state = (*node).fdw_state as *const FdwScanState;

        if state.is_null() {
            return;
        }

        let state = &*state;

        // Add DB2 query to EXPLAIN output
        if (*es).verbose {
            let sql = std::ffi::CString::new(state.plan.sql.as_str()).unwrap_or_default();
            pg_sys::ExplainPropertyText(
                b"DB2 Query\0".as_ptr() as *const i8,
                sql.as_ptr(),
                es,
            );
        }

        // Show server name
        if let Some(ref server) = state.options.dbserver {
            let server_cstr = std::ffi::CString::new(server.as_str()).unwrap_or_default();
            pg_sys::ExplainPropertyText(
                b"DB2 Server\0".as_ptr() as *const i8,
                server_cstr.as_ptr(),
                es,
            );
        }

        // Show table name
        if let Some(ref table) = state.options.table {
            let mut table_full = String::new();
            if let Some(ref schema) = state.options.schema {
                table_full.push_str(schema);
                table_full.push('.');
            }
            table_full.push_str(table);

            let table_cstr = std::ffi::CString::new(table_full.as_str()).unwrap_or_default();
            pg_sys::ExplainPropertyText(
                b"Remote Table\0".as_ptr() as *const i8,
                table_cstr.as_ptr(),
                es,
            );
        }

        // Show rows fetched (if ANALYZE was run)
        if (*es).analyze && state.rows_fetched > 0 {
            pg_sys::ExplainPropertyInteger(
                b"Rows Fetched\0".as_ptr() as *const i8,
                std::ptr::null(),
                state.rows_fetched as i64,
                es,
            );
        }
    }
}

/// Explain a foreign modify
///
/// PostgreSQL FDW callback: ExplainForeignModify
#[pg_guard]
pub extern "C" fn explain_foreign_modify(
    mtstate: *mut pg_sys::ModifyTableState,
    resultRelInfo: *mut pg_sys::ResultRelInfo,
    _fdw_private: *mut pg_sys::List,
    _subplan_index: ::std::os::raw::c_int,
    es: *mut pg_sys::ExplainState,
) {
    unsafe {
        let state = (*resultRelInfo).ri_FdwState as *const FdwModifyState;

        if state.is_null() {
            return;
        }

        let state = &*state;

        // Add the modification SQL
        if (*es).verbose && !state.sql.is_empty() {
            let sql = std::ffi::CString::new(state.sql.as_str()).unwrap_or_default();
            pg_sys::ExplainPropertyText(
                b"DB2 Query\0".as_ptr() as *const i8,
                sql.as_ptr(),
                es,
            );
        }

        // Show server name
        if let Some(ref server) = state.options.dbserver {
            let server_cstr = std::ffi::CString::new(server.as_str()).unwrap_or_default();
            pg_sys::ExplainPropertyText(
                b"DB2 Server\0".as_ptr() as *const i8,
                server_cstr.as_ptr(),
                es,
            );
        }

        // Show batch size if > 1
        if state.batch_size > 1 {
            pg_sys::ExplainPropertyInteger(
                b"Batch Size\0".as_ptr() as *const i8,
                std::ptr::null(),
                state.batch_size as i64,
                es,
            );
        }

        // Show rows affected (if ANALYZE was run)
        if (*es).analyze && state.rows_affected > 0 {
            pg_sys::ExplainPropertyInteger(
                b"Rows Affected\0".as_ptr() as *const i8,
                std::ptr::null(),
                state.rows_affected as i64,
                es,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_explain_compiles() {
        // Just ensure the module compiles
        assert!(true);
    }
}
