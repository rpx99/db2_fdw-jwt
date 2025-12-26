//! Foreign Scan implementation
//!
//! Handles SELECT queries against DB2 tables with real ODBC execution.

use pgrx::prelude::*;
use pgrx::pg_sys;
use tracing::{debug, info, warn, error};

use crate::options::FdwOptions;
use crate::state::{FdwPlanState, FdwScanState};
use crate::query::QueryBuilder;
use db2_odbc::{Db2Value, SqlType};

/// Get the estimated size of a foreign relation
///
/// PostgreSQL FDW callback: GetForeignRelSize
#[pg_guard]
pub extern "C" fn get_foreign_rel_size(
    _root: *mut pg_sys::PlannerInfo,
    baserel: *mut pg_sys::RelOptInfo,
    _foreigntableid: pg_sys::Oid,
) {
    debug!("get_foreign_rel_size called");
    unsafe {
        // Default estimates - could query DB2 for statistics
        (*baserel).rows = 1000.0;
    }
}

/// Create access paths for a foreign scan
///
/// PostgreSQL FDW callback: GetForeignPaths
#[pg_guard]
pub extern "C" fn get_foreign_paths(
    root: *mut pg_sys::PlannerInfo,
    baserel: *mut pg_sys::RelOptInfo,
    _foreigntableid: pg_sys::Oid,
) {
    debug!("get_foreign_paths called");
    unsafe {
        let startup_cost = 10.0;
        let total_cost = (*baserel).rows * 0.01 + startup_cost;

        let path = pg_sys::create_foreignscan_path(
            root,
            baserel,
            std::ptr::null_mut(), // pathtarget
            (*baserel).rows,
            startup_cost,
            total_cost,
            std::ptr::null_mut(), // pathkeys
            std::ptr::null_mut(), // required_outer
            std::ptr::null_mut(), // fdw_outerpath
            std::ptr::null_mut(), // fdw_private
        );

        pg_sys::add_path(baserel, path as *mut pg_sys::Path);
    }
}

/// Create a foreign scan plan
///
/// PostgreSQL FDW callback: GetForeignPlan
#[pg_guard]
pub extern "C" fn get_foreign_plan(
    root: *mut pg_sys::PlannerInfo,
    baserel: *mut pg_sys::RelOptInfo,
    foreigntableid: pg_sys::Oid,
    _best_path: *mut pg_sys::ForeignPath,
    tlist: *mut pg_sys::List,
    scan_clauses: *mut pg_sys::List,
    outer_plan: *mut pg_sys::Plan,
) -> *mut pg_sys::ForeignScan {
    debug!("get_foreign_plan called");
    unsafe {
        // Get the foreign table and relation
        let rte = pg_sys::planner_rt_fetch((*baserel).relid, root);

        // Extract column names from target list
        let mut columns = Vec::new();
        if !tlist.is_null() {
            let list_len = (*tlist).length;
            for i in 0..list_len {
                // Each element in target list is a TargetEntry
                // For now, we'll select all columns
                columns.push("*".to_string());
                break; // Just use SELECT * for now
            }
        }
        if columns.is_empty() {
            columns.push("*".to_string());
        }

        // Build options - get table name from relation
        let mut options = FdwOptions::new();

        // Try to get the relation to extract table name
        let rel = pg_sys::RelationIdGetRelation(foreigntableid);
        if !rel.is_null() {
            let relname = std::ffi::CStr::from_ptr((*(*rel).rd_rel).relname.data.as_ptr())
                .to_string_lossy()
                .to_string();
            options.table = Some(relname);
            pg_sys::RelationClose(rel);
        }

        // Build the SQL query using QueryBuilder
        let sql = if let Some(qb) = QueryBuilder::from_options(&options) {
            qb.with_columns(columns.clone()).build_select(None, None, None)
        } else {
            format!("SELECT * FROM \"{}\"", options.table.as_deref().unwrap_or("unknown"))
        };

        debug!(sql = %sql, "Built SELECT query for plan");

        // Create plan state
        let plan_state = FdwPlanState::new(sql, columns);

        // Serialize plan state to pass to executor
        let plan_bytes = plan_state.serialize();

        // Create a PostgreSQL List to hold the plan state
        // For now, we'll pass it via the scan's fdw_private field
        let scan_clauses = pg_sys::extract_actual_clauses(scan_clauses, false);

        pg_sys::make_foreignscan(
            tlist,
            scan_clauses,
            (*baserel).relid,
            std::ptr::null_mut(), // fdw_exprs
            std::ptr::null_mut(), // fdw_private - TODO: pass plan_state through this
            std::ptr::null_mut(), // fdw_scan_tlist
            std::ptr::null_mut(), // fdw_recheck_quals
            outer_plan,
        )
    }
}

/// Begin a foreign scan
///
/// PostgreSQL FDW callback: BeginForeignScan
#[pg_guard]
pub extern "C" fn begin_foreign_scan(
    node: *mut pg_sys::ForeignScanState,
    _eflags: ::std::os::raw::c_int,
) {
    debug!("begin_foreign_scan called");

    unsafe {
        // Get the foreign scan node to access relation info
        let scan = (*node).ss.ps.plan as *mut pg_sys::ForeignScan;
        let rel = (*node).ss.ss_currentRelation;

        // Build options from relation
        let mut options = FdwOptions::new();

        if !rel.is_null() {
            let relname = std::ffi::CStr::from_ptr((*(*rel).rd_rel).relname.data.as_ptr())
                .to_string_lossy()
                .to_string();
            options.table = Some(relname);
        }

        // Build the SQL query
        let sql = if let Some(qb) = QueryBuilder::from_options(&options) {
            qb.build_select(None, None, None)
        } else {
            format!("SELECT * FROM \"{}\"", options.table.as_deref().unwrap_or("unknown"))
        };

        let plan_state = FdwPlanState::new(sql.clone(), vec!["*".into()]);

        // Allocate scan state in PostgreSQL memory context
        let state = Box::new(FdwScanState::new(options, plan_state));

        // Store as fdw_state
        (*node).fdw_state = Box::into_raw(state) as *mut std::ffi::c_void;

        // Initialize the session and execute the query
        let state = &mut *((*node).fdw_state as *mut FdwScanState);

        // Initialize session
        if let Err(e) = state.init_session() {
            error!("Failed to initialize session: {}", e);
            state.finished = true;
            return;
        }

        // Execute the query
        debug!(sql = %state.plan.sql, "Executing foreign scan query");
        if let Some(ref mut session) = state.session {
            if let Err(e) = session.prepare_and_execute(&state.plan.sql, &[]) {
                error!("Failed to execute query: {}", e);
                state.finished = true;
            } else {
                info!("Query executed successfully");
            }
        }
    }
}

/// Fetch the next row from the foreign table
///
/// PostgreSQL FDW callback: IterateForeignScan
#[pg_guard]
pub extern "C" fn iterate_foreign_scan(
    node: *mut pg_sys::ForeignScanState,
) -> *mut pg_sys::TupleTableSlot {
    unsafe {
        let slot = (*node).ss.ss_ScanTupleSlot;
        pg_sys::ExecClearTuple(slot);

        let state = (*node).fdw_state as *mut FdwScanState;
        if state.is_null() {
            return slot;
        }

        let state = &mut *state;

        if state.finished {
            return slot;
        }

        // Fetch the next row from the session
        if let Some(ref mut session) = state.session {
            match session.fetch_next() {
                Ok(Some(row)) => {
                    state.rows_fetched += 1;
                    state.current_row = Some(row.clone());

                    // Convert row to tuple slot
                    if let Err(e) = fill_tuple_slot(slot, &row, &(*node).ss.ss_ScanTupleSlot) {
                        error!("Failed to fill tuple slot: {}", e);
                        state.finished = true;
                        return slot;
                    }

                    // Mark slot as containing a valid tuple
                    pg_sys::ExecStoreVirtualTuple(slot);
                }
                Ok(None) => {
                    debug!("End of result set, {} rows fetched", state.rows_fetched);
                    state.finished = true;
                }
                Err(e) => {
                    error!("Error fetching row: {}", e);
                    state.finished = true;
                }
            }
        } else {
            state.finished = true;
        }

        slot
    }
}

/// Fill a tuple slot with values from a DB2 row
unsafe fn fill_tuple_slot(
    slot: *mut pg_sys::TupleTableSlot,
    row: &[Db2Value],
    _scan_slot: &*mut pg_sys::TupleTableSlot,
) -> Result<(), String> {
    let tupdesc = (*slot).tts_tupleDescriptor;
    let natts = (*tupdesc).natts as usize;

    if row.len() != natts {
        return Err(format!(
            "Row has {} columns but tuple descriptor expects {}",
            row.len(),
            natts
        ));
    }

    // Get the values and nulls arrays
    let values = (*slot).tts_values;
    let nulls = (*slot).tts_isnull;

    for (i, value) in row.iter().enumerate() {
        match value {
            Db2Value::Null => {
                *nulls.add(i) = true;
                *values.add(i) = pg_sys::Datum::from(0);
            }
            Db2Value::Text(s) => {
                *nulls.add(i) = false;
                // Convert to PostgreSQL text datum
                let cstr = std::ffi::CString::new(s.as_str()).map_err(|e| e.to_string())?;
                let text = pg_sys::cstring_to_text(cstr.as_ptr());
                *values.add(i) = pg_sys::Datum::from(text);
            }
            Db2Value::SmallInt(v) => {
                *nulls.add(i) = false;
                *values.add(i) = pg_sys::Datum::from(*v as i16);
            }
            Db2Value::Integer(v) => {
                *nulls.add(i) = false;
                *values.add(i) = pg_sys::Datum::from(*v);
            }
            Db2Value::BigInt(v) => {
                *nulls.add(i) = false;
                *values.add(i) = pg_sys::Datum::from(*v);
            }
            Db2Value::Real(v) => {
                *nulls.add(i) = false;
                *values.add(i) = pg_sys::Datum::from(*v);
            }
            Db2Value::Double(v) => {
                *nulls.add(i) = false;
                *values.add(i) = pg_sys::Datum::from(*v);
            }
            Db2Value::Decimal(d) => {
                *nulls.add(i) = false;
                // Convert decimal to numeric string then to PostgreSQL numeric
                let s = d.to_string();
                let cstr = std::ffi::CString::new(s.as_str()).map_err(|e| e.to_string())?;
                let text = pg_sys::cstring_to_text(cstr.as_ptr());
                *values.add(i) = pg_sys::Datum::from(text);
            }
            Db2Value::Date(d) => {
                *nulls.add(i) = false;
                // Convert to PostgreSQL date
                let s = d.format("%Y-%m-%d").to_string();
                let cstr = std::ffi::CString::new(s.as_str()).map_err(|e| e.to_string())?;
                let text = pg_sys::cstring_to_text(cstr.as_ptr());
                *values.add(i) = pg_sys::Datum::from(text);
            }
            Db2Value::Time(t) => {
                *nulls.add(i) = false;
                let s = t.format("%H:%M:%S").to_string();
                let cstr = std::ffi::CString::new(s.as_str()).map_err(|e| e.to_string())?;
                let text = pg_sys::cstring_to_text(cstr.as_ptr());
                *values.add(i) = pg_sys::Datum::from(text);
            }
            Db2Value::Timestamp(ts) => {
                *nulls.add(i) = false;
                let s = ts.format("%Y-%m-%d %H:%M:%S%.f").to_string();
                let cstr = std::ffi::CString::new(s.as_str()).map_err(|e| e.to_string())?;
                let text = pg_sys::cstring_to_text(cstr.as_ptr());
                *values.add(i) = pg_sys::Datum::from(text);
            }
            Db2Value::Boolean(b) => {
                *nulls.add(i) = false;
                *values.add(i) = pg_sys::Datum::from(*b);
            }
            Db2Value::Binary(b) => {
                *nulls.add(i) = false;
                // Convert to PostgreSQL bytea
                let bytea = pg_sys::palloc(b.len() + pg_sys::VARHDRSZ as usize) as *mut pg_sys::varlena;
                pg_sys::SET_VARSIZE(bytea, (b.len() + pg_sys::VARHDRSZ as usize) as i32);
                std::ptr::copy_nonoverlapping(
                    b.as_ptr(),
                    (bytea as *mut u8).add(pg_sys::VARHDRSZ as usize),
                    b.len(),
                );
                *values.add(i) = pg_sys::Datum::from(bytea);
            }
            Db2Value::Xml(x) => {
                *nulls.add(i) = false;
                let cstr = std::ffi::CString::new(x.as_str()).map_err(|e| e.to_string())?;
                let text = pg_sys::cstring_to_text(cstr.as_ptr());
                *values.add(i) = pg_sys::Datum::from(text);
            }
        }
    }

    Ok(())
}

/// Restart a foreign scan
///
/// PostgreSQL FDW callback: ReScanForeignScan
#[pg_guard]
pub extern "C" fn rescan_foreign_scan(node: *mut pg_sys::ForeignScanState) {
    debug!("rescan_foreign_scan called");
    unsafe {
        let state = (*node).fdw_state as *mut FdwScanState;
        if !state.is_null() {
            let state = &mut *state;
            state.rows_fetched = 0;
            state.finished = false;
            state.current_row = None;

            // Re-execute the query
            if let Some(ref mut session) = state.session {
                if let Err(e) = session.close_cursor() {
                    warn!("Error closing cursor: {}", e);
                }
                if let Err(e) = session.prepare_and_execute(&state.plan.sql, &[]) {
                    error!("Error re-executing query: {}", e);
                    state.finished = true;
                }
            }
        }
    }
}

/// End a foreign scan
///
/// PostgreSQL FDW callback: EndForeignScan
#[pg_guard]
pub extern "C" fn end_foreign_scan(node: *mut pg_sys::ForeignScanState) {
    debug!("end_foreign_scan called");
    unsafe {
        let state = (*node).fdw_state as *mut FdwScanState;
        if !state.is_null() {
            // Close session and take ownership to drop
            let mut state = Box::from_raw(state);
            if let Some(ref mut session) = state.session {
                session.close();
            }
            (*node).fdw_state = std::ptr::null_mut();
            // state is dropped here
        }
    }
}

/// Analyze a foreign table
///
/// PostgreSQL FDW callback: AnalyzeForeignTable
/// Returns true if we can provide statistics, false otherwise.
#[pg_guard]
pub extern "C" fn analyze_foreign_table(
    relation: pg_sys::Relation,
    func: *mut pg_sys::AcquireSampleRowsFunc,
    totalpages: *mut pg_sys::BlockNumber,
) -> bool {
    debug!("analyze_foreign_table called");

    unsafe {
        if relation.is_null() || func.is_null() || totalpages.is_null() {
            return false;
        }

        // Set the sampling function
        *func = Some(acquire_sample_rows);

        // Estimate total pages (1 page = 8KB typically)
        // Use a rough estimate based on expected row count
        *totalpages = 100; // Default estimate

        true
    }
}

/// Acquire sample rows for ANALYZE
///
/// This function is called by PostgreSQL to get a sample of rows from the foreign table.
#[pg_guard]
pub extern "C" fn acquire_sample_rows(
    relation: pg_sys::Relation,
    _elevel: ::std::os::raw::c_int,
    rows: *mut pg_sys::HeapTuple,
    targrows: ::std::os::raw::c_int,
    totalrows: *mut f64,
    _totaldeadrows: *mut f64,
) -> ::std::os::raw::c_int {
    debug!("acquire_sample_rows called, target = {}", targrows);

    unsafe {
        if relation.is_null() || rows.is_null() || totalrows.is_null() {
            return 0;
        }

        // Build options from relation
        let mut options = FdwOptions::new();
        let relname = std::ffi::CStr::from_ptr((*(*relation).rd_rel).relname.data.as_ptr())
            .to_string_lossy()
            .to_string();
        options.table = Some(relname);

        // Build a COUNT query to get total rows
        let count_sql = if let Some(qb) = QueryBuilder::from_options(&options) {
            format!("SELECT COUNT(*) FROM \"{}\"", qb.table())
        } else {
            return 0;
        };

        // For now, return 0 rows but estimate total
        // A full implementation would:
        // 1. Execute COUNT(*) to get total rows
        // 2. Execute SELECT with TABLESAMPLE or FETCH FIRST N ROWS
        // 3. Convert rows to HeapTuples

        *totalrows = 1000.0; // Default estimate

        debug!("ANALYZE: estimated {} total rows", *totalrows);
        0 // Return 0 sample rows for now
    }
}

/// Get foreign join paths
///
/// PostgreSQL FDW callback: GetForeignJoinPaths
#[pg_guard]
pub extern "C" fn get_foreign_join_paths(
    _root: *mut pg_sys::PlannerInfo,
    _joinrel: *mut pg_sys::RelOptInfo,
    _outerrel: *mut pg_sys::RelOptInfo,
    _innerrel: *mut pg_sys::RelOptInfo,
    _jointype: pg_sys::JoinType,
    _extra: *mut pg_sys::JoinPathExtraData,
) {
    // TODO: Implement join pushdown
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plan_state() {
        let state = FdwPlanState::new("SELECT 1".into(), vec![]);
        assert_eq!(state.sql, "SELECT 1");
    }
}
