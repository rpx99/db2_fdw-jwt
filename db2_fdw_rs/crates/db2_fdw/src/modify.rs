//! Foreign Modify implementation
//!
//! Handles INSERT, UPDATE, DELETE, and TRUNCATE operations.

use pgrx::prelude::*;
use pgrx::pg_sys;

use crate::options::FdwOptions;
use crate::state::FdwModifyState;

/// Add columns needed for UPDATE/DELETE operations
///
/// PostgreSQL FDW callback: AddForeignUpdateTargets
#[pg_guard]
pub extern "C" fn add_foreign_update_targets(
    _root: *mut pg_sys::PlannerInfo,
    _rtindex: pg_sys::Index,
    _target_rte: *mut pg_sys::RangeTblEntry,
    _target_relation: pg_sys::Relation,
) {
    // Real implementation would add ctid or primary key columns
    // needed to identify rows for UPDATE/DELETE
}

/// Plan a foreign modification
///
/// PostgreSQL FDW callback: PlanForeignModify
#[pg_guard]
pub extern "C" fn plan_foreign_modify(
    _root: *mut pg_sys::PlannerInfo,
    _plan: *mut pg_sys::ModifyTable,
    _resultRelation: pg_sys::Index,
    _subplan_index: ::std::os::raw::c_int,
) -> *mut pg_sys::List {
    // Real implementation would:
    // 1. Build INSERT/UPDATE/DELETE SQL
    // 2. Return as private data

    std::ptr::null_mut()
}

/// Begin a foreign modification
///
/// PostgreSQL FDW callback: BeginForeignModify
#[pg_guard]
pub extern "C" fn begin_foreign_modify(
    mtstate: *mut pg_sys::ModifyTableState,
    resultRelInfo: *mut pg_sys::ResultRelInfo,
    _fdw_private: *mut pg_sys::List,
    _subplan_index: ::std::os::raw::c_int,
    _eflags: ::std::os::raw::c_int,
) {
    unsafe {
        // Initialize modify state
        let state = Box::new(FdwModifyState::new(
            FdwOptions::new(),
            String::new(),
        ));

        (*resultRelInfo).ri_FdwState = Box::into_raw(state) as *mut std::ffi::c_void;
    }
}

/// Execute a foreign INSERT
///
/// PostgreSQL FDW callback: ExecForeignInsert
#[pg_guard]
pub extern "C" fn exec_foreign_insert(
    _estate: *mut pg_sys::EState,
    resultRelInfo: *mut pg_sys::ResultRelInfo,
    slot: *mut pg_sys::TupleTableSlot,
    _planSlot: *mut pg_sys::TupleTableSlot,
) -> *mut pg_sys::TupleTableSlot {
    unsafe {
        let state = (*resultRelInfo).ri_FdwState as *mut FdwModifyState;
        if state.is_null() {
            pgrx::error!("FDW state not initialized");
        }

        let state = &mut *state;

        // Real implementation would:
        // 1. Extract values from slot
        // 2. Convert to DB2 format
        // 3. Execute INSERT on DB2
        // 4. Return the slot

        state.rows_affected += 1;
        slot
    }
}

/// Execute a foreign UPDATE
///
/// PostgreSQL FDW callback: ExecForeignUpdate
#[pg_guard]
pub extern "C" fn exec_foreign_update(
    _estate: *mut pg_sys::EState,
    resultRelInfo: *mut pg_sys::ResultRelInfo,
    slot: *mut pg_sys::TupleTableSlot,
    _planSlot: *mut pg_sys::TupleTableSlot,
) -> *mut pg_sys::TupleTableSlot {
    unsafe {
        let state = (*resultRelInfo).ri_FdwState as *mut FdwModifyState;
        if state.is_null() {
            pgrx::error!("FDW state not initialized");
        }

        let state = &mut *state;

        // Real implementation would:
        // 1. Extract key values for WHERE clause
        // 2. Extract new values for SET clause
        // 3. Execute UPDATE on DB2

        state.rows_affected += 1;
        slot
    }
}

/// Execute a foreign DELETE
///
/// PostgreSQL FDW callback: ExecForeignDelete
#[pg_guard]
pub extern "C" fn exec_foreign_delete(
    _estate: *mut pg_sys::EState,
    resultRelInfo: *mut pg_sys::ResultRelInfo,
    slot: *mut pg_sys::TupleTableSlot,
    _planSlot: *mut pg_sys::TupleTableSlot,
) -> *mut pg_sys::TupleTableSlot {
    unsafe {
        let state = (*resultRelInfo).ri_FdwState as *mut FdwModifyState;
        if state.is_null() {
            pgrx::error!("FDW state not initialized");
        }

        let state = &mut *state;

        // Real implementation would:
        // 1. Extract key values for WHERE clause
        // 2. Execute DELETE on DB2

        state.rows_affected += 1;
        slot
    }
}

/// End a foreign modification
///
/// PostgreSQL FDW callback: EndForeignModify
#[pg_guard]
pub extern "C" fn end_foreign_modify(
    _estate: *mut pg_sys::EState,
    resultRelInfo: *mut pg_sys::ResultRelInfo,
) {
    unsafe {
        let state = (*resultRelInfo).ri_FdwState as *mut FdwModifyState;
        if !state.is_null() {
            let mut state = Box::from_raw(state);

            // Flush any remaining batch
            if let Err(e) = state.flush_batch() {
                pgrx::warning!("Failed to flush batch: {}", e);
            }

            // State is dropped here
            (*resultRelInfo).ri_FdwState = std::ptr::null_mut();
        }
    }
}

/// Get batch size for foreign modify
///
/// PostgreSQL FDW callback: GetForeignModifyBatchSize (PostgreSQL 14+)
#[cfg(feature = "pg14")]
#[pg_guard]
pub extern "C" fn get_foreign_modify_batch_size(
    resultRelInfo: *mut pg_sys::ResultRelInfo,
) -> ::std::os::raw::c_int {
    unsafe {
        let state = (*resultRelInfo).ri_FdwState as *mut FdwModifyState;
        if state.is_null() {
            return 1;
        }

        (*state).batch_size as i32
    }
}

/// Execute a batch of foreign INSERTs
///
/// PostgreSQL FDW callback: ExecForeignBatchInsert (PostgreSQL 14+)
#[cfg(feature = "pg14")]
#[pg_guard]
pub extern "C" fn exec_foreign_batch_insert(
    _estate: *mut pg_sys::EState,
    resultRelInfo: *mut pg_sys::ResultRelInfo,
    slots: *mut *mut pg_sys::TupleTableSlot,
    _planSlots: *mut *mut pg_sys::TupleTableSlot,
    numSlots: *mut ::std::os::raw::c_int,
) -> *mut *mut pg_sys::TupleTableSlot {
    unsafe {
        let state = (*resultRelInfo).ri_FdwState as *mut FdwModifyState;
        if state.is_null() {
            pgrx::error!("FDW state not initialized");
        }

        let state = &mut *state;
        let num = *numSlots as usize;

        // Real implementation would:
        // 1. Extract values from all slots
        // 2. Execute batch INSERT on DB2

        state.rows_affected += num as u64;
        slots
    }
}

/// Execute a foreign TRUNCATE
///
/// PostgreSQL FDW callback: ExecForeignTruncate
#[pg_guard]
pub extern "C" fn exec_foreign_truncate(
    _rels: *mut pg_sys::List,
    _behavior: pg_sys::DropBehavior,
) {
    // Real implementation would:
    // 1. Build TRUNCATE SQL for each table
    // 2. Execute on DB2
}

/// Check if a foreign table is updatable
///
/// PostgreSQL FDW callback: IsForeignRelUpdatable
#[pg_guard]
pub extern "C" fn is_foreign_rel_updatable(
    _rel: pg_sys::Relation,
) -> ::std::os::raw::c_int {
    // Return bitmap of allowed operations:
    // 1 = INSERT, 2 = UPDATE, 4 = DELETE

    // Check readonly option
    // For now, allow all operations
    1 | 2 | 4 // INSERT | UPDATE | DELETE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_modify_state() {
        let state = FdwModifyState::new(FdwOptions::new(), "INSERT INTO test".into());
        assert_eq!(state.rows_affected, 0);
    }
}
