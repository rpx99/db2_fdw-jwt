//! Foreign Scan implementation
//!
//! Handles SELECT queries against DB2 tables with real ODBC execution.

use pgrx::prelude::*;
use pgrx::pg_sys;
use tracing::{debug, info, warn, error};

use crate::options::FdwOptions;
use crate::state::{FdwPlanState, FdwScanState};
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
    _root: *mut pg_sys::PlannerInfo,
    baserel: *mut pg_sys::RelOptInfo,
    _foreigntableid: pg_sys::Oid,
    _best_path: *mut pg_sys::ForeignPath,
    tlist: *mut pg_sys::List,
    scan_clauses: *mut pg_sys::List,
    outer_plan: *mut pg_sys::Plan,
) -> *mut pg_sys::ForeignScan {
    debug!("get_foreign_plan called");
    unsafe {
        // Build the SQL query
        // TODO: Implement proper query deparsing from db2_query crate
        let plan_state = FdwPlanState::new(
            "SELECT * FROM remote_table".into(),
            vec!["*".into()],
        );

        let scan_clauses = pg_sys::extract_actual_clauses(scan_clauses, false);

        pg_sys::make_foreignscan(
            tlist,
            scan_clauses,
            (*baserel).relid,
            std::ptr::null_mut(), // fdw_exprs
            std::ptr::null_mut(), // fdw_private (would contain serialized plan_state)
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
        // Allocate scan state in PostgreSQL memory context
        let state = Box::new(FdwScanState::new(
            FdwOptions::new(),
            FdwPlanState::default(),
        ));

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
#[pg_guard]
pub extern "C" fn analyze_foreign_table(
    _relation: pg_sys::Relation,
    _func: *mut pg_sys::AcquireSampleRowsFunc,
    _totalpages: *mut pg_sys::BlockNumber,
) -> bool {
    // TODO: Implement ANALYZE support by sampling rows from DB2
    false
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
