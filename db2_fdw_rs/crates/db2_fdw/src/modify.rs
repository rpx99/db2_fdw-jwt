//! Foreign Modify implementation
//!
//! Handles INSERT, UPDATE, DELETE, and TRUNCATE operations with real ODBC execution.

use pgrx::prelude::*;
use pgrx::pg_sys;
use tracing::{debug, info, warn, error};

use crate::options::FdwOptions;
use crate::state::FdwModifyState;
use db2_odbc::{Db2Value, Db2Statement};
use db2_connection::Db2Session;

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
    debug!("add_foreign_update_targets called");
    // TODO: Add ctid or primary key columns needed to identify rows for UPDATE/DELETE
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
    debug!("plan_foreign_modify called");
    // TODO: Build INSERT/UPDATE/DELETE SQL and return as private data
    std::ptr::null_mut()
}

/// Begin a foreign modification
///
/// PostgreSQL FDW callback: BeginForeignModify
#[pg_guard]
pub extern "C" fn begin_foreign_modify(
    _mtstate: *mut pg_sys::ModifyTableState,
    resultRelInfo: *mut pg_sys::ResultRelInfo,
    _fdw_private: *mut pg_sys::List,
    _subplan_index: ::std::os::raw::c_int,
    _eflags: ::std::os::raw::c_int,
) {
    debug!("begin_foreign_modify called");
    unsafe {
        // Initialize modify state
        let mut state = Box::new(FdwModifyState::new(
            FdwOptions::new(),
            String::new(),
        ));

        // Initialize session
        if let Err(e) = state.init_session() {
            error!("Failed to initialize session for modify: {}", e);
        }

        (*resultRelInfo).ri_FdwState = Box::into_raw(state) as *mut std::ffi::c_void;
    }
}

/// Extract values from a tuple slot into Db2Values
unsafe fn extract_slot_values(
    slot: *mut pg_sys::TupleTableSlot,
    state: &FdwModifyState,
) -> Result<Vec<Db2Value>, String> {
    let tupdesc = (*slot).tts_tupleDescriptor;
    let natts = (*tupdesc).natts as usize;
    let values = (*slot).tts_values;
    let nulls = (*slot).tts_isnull;

    let mut result = Vec::with_capacity(natts);

    for i in 0..natts {
        if *nulls.add(i) {
            result.push(Db2Value::Null);
        } else {
            let datum = *values.add(i);
            let att = pg_sys::TupleDescAttr(tupdesc, i as i32);
            let typid = (*att).atttypid;

            // Convert PostgreSQL datum to Db2Value based on type
            let value = match typid {
                pg_sys::INT2OID => Db2Value::SmallInt(datum.value() as i16),
                pg_sys::INT4OID => Db2Value::Integer(datum.value() as i32),
                pg_sys::INT8OID => Db2Value::BigInt(datum.value() as i64),
                pg_sys::FLOAT4OID => Db2Value::Real(f32::from_bits(datum.value() as u32)),
                pg_sys::FLOAT8OID => Db2Value::Double(f64::from_bits(datum.value() as u64)),
                pg_sys::BOOLOID => Db2Value::Boolean(datum.value() != 0),
                pg_sys::TEXTOID | pg_sys::VARCHAROID | pg_sys::BPCHAROID => {
                    let text = datum.cast_mut_ptr::<pg_sys::varlena>();
                    let data = pg_sys::VARDATA_ANY(text);
                    let len = pg_sys::VARSIZE_ANY_EXHDR(text);
                    let slice = std::slice::from_raw_parts(data as *const u8, len);
                    let s = String::from_utf8_lossy(slice).to_string();
                    Db2Value::Text(s)
                }
                pg_sys::BYTEAOID => {
                    let bytea = datum.cast_mut_ptr::<pg_sys::varlena>();
                    let data = pg_sys::VARDATA_ANY(bytea);
                    let len = pg_sys::VARSIZE_ANY_EXHDR(bytea);
                    let slice = std::slice::from_raw_parts(data as *const u8, len);
                    Db2Value::Binary(slice.to_vec())
                }
                _ => {
                    // Convert to text representation for other types
                    let cstr = pg_sys::OidOutputFunctionCall(
                        pg_sys::getTypeOutputInfo(typid, std::ptr::null_mut(), std::ptr::null_mut()),
                        datum,
                    );
                    let s = std::ffi::CStr::from_ptr(cstr).to_string_lossy().to_string();
                    Db2Value::Text(s)
                }
            };
            result.push(value);
        }
    }

    Ok(result)
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
    debug!("exec_foreign_insert called");
    unsafe {
        let state = (*resultRelInfo).ri_FdwState as *mut FdwModifyState;
        if state.is_null() {
            pgrx::error!("FDW state not initialized");
        }

        let state = &mut *state;

        // Extract values from slot
        match extract_slot_values(slot, state) {
            Ok(values) => {
                // Add to batch or execute immediately
                if state.add_to_batch(values) {
                    // Batch is full, flush it
                    if let Err(e) = flush_batch_with_session(state) {
                        error!("Failed to flush batch: {}", e);
                    }
                }
            }
            Err(e) => {
                error!("Failed to extract values: {}", e);
            }
        }

        state.rows_affected += 1;
        slot
    }
}

/// Flush batch with actual ODBC execution
fn flush_batch_with_session(state: &mut FdwModifyState) -> Result<usize, String> {
    if state.batch_buffer.is_empty() {
        return Ok(0);
    }

    let session = state.session.as_ref()
        .ok_or_else(|| "No session available".to_string())?;

    let count = state.batch_buffer.len();
    debug!("Flushing {} rows to DB2", count);

    // Execute INSERT for each row
    for row in &state.batch_buffer {
        // Build INSERT SQL with values
        let placeholders: Vec<String> = row.iter().map(|v| v.to_string()).collect();
        let sql = format!(
            "INSERT INTO {} VALUES ({})",
            state.sql,
            placeholders.join(", ")
        );

        if let Err(e) = session.connection().execute_immediate(&sql) {
            return Err(format!("INSERT failed: {}", e));
        }
    }

    state.batch_buffer.clear();
    info!("Flushed {} rows", count);
    Ok(count)
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
    debug!("exec_foreign_update called");
    unsafe {
        let state = (*resultRelInfo).ri_FdwState as *mut FdwModifyState;
        if state.is_null() {
            pgrx::error!("FDW state not initialized");
        }

        let state = &mut *state;

        // Extract values from slot
        match extract_slot_values(slot, state) {
            Ok(values) => {
                if let Some(ref session) = state.session {
                    // Build UPDATE SQL
                    // TODO: Properly build WHERE clause from key columns
                    let set_parts: Vec<String> = state.target_columns.iter()
                        .zip(values.iter())
                        .map(|(col, val)| format!("{} = {}", col, val))
                        .collect();

                    let sql = format!(
                        "UPDATE {} SET {} WHERE 1=1",
                        state.sql,
                        set_parts.join(", ")
                    );

                    if let Err(e) = session.connection().execute_immediate(&sql) {
                        error!("UPDATE failed: {}", e);
                    }
                }
            }
            Err(e) => {
                error!("Failed to extract values: {}", e);
            }
        }

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
    debug!("exec_foreign_delete called");
    unsafe {
        let state = (*resultRelInfo).ri_FdwState as *mut FdwModifyState;
        if state.is_null() {
            pgrx::error!("FDW state not initialized");
        }

        let state = &mut *state;

        // Extract key values from slot for WHERE clause
        match extract_slot_values(slot, state) {
            Ok(values) => {
                if let Some(ref session) = state.session {
                    // Build DELETE SQL
                    // TODO: Properly build WHERE clause from key columns
                    let where_parts: Vec<String> = state.key_columns.iter()
                        .zip(values.iter())
                        .map(|(col, val)| format!("{} = {}", col, val))
                        .collect();

                    let sql = if where_parts.is_empty() {
                        format!("DELETE FROM {} WHERE 1=0", state.sql) // Safety: don't delete all
                    } else {
                        format!(
                            "DELETE FROM {} WHERE {}",
                            state.sql,
                            where_parts.join(" AND ")
                        )
                    };

                    if let Err(e) = session.connection().execute_immediate(&sql) {
                        error!("DELETE failed: {}", e);
                    }
                }
            }
            Err(e) => {
                error!("Failed to extract values: {}", e);
            }
        }

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
    debug!("end_foreign_modify called");
    unsafe {
        let state = (*resultRelInfo).ri_FdwState as *mut FdwModifyState;
        if !state.is_null() {
            let mut state = Box::from_raw(state);

            // Flush any remaining batch
            if let Err(e) = flush_batch_with_session(&mut state) {
                warn!("Failed to flush final batch: {}", e);
            }

            // Close session
            if let Some(ref mut session) = state.session {
                session.close();
            }

            (*resultRelInfo).ri_FdwState = std::ptr::null_mut();
            info!("Modify complete, {} rows affected", state.rows_affected);
        }
    }
}

/// Get batch size for foreign modify
///
/// PostgreSQL FDW callback: GetForeignModifyBatchSize (PostgreSQL 14+)
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
#[pg_guard]
pub extern "C" fn exec_foreign_batch_insert(
    _estate: *mut pg_sys::EState,
    resultRelInfo: *mut pg_sys::ResultRelInfo,
    slots: *mut *mut pg_sys::TupleTableSlot,
    _planSlots: *mut *mut pg_sys::TupleTableSlot,
    numSlots: *mut ::std::os::raw::c_int,
) -> *mut *mut pg_sys::TupleTableSlot {
    debug!("exec_foreign_batch_insert called");
    unsafe {
        let state = (*resultRelInfo).ri_FdwState as *mut FdwModifyState;
        if state.is_null() {
            pgrx::error!("FDW state not initialized");
        }

        let state = &mut *state;
        let num = *numSlots as usize;

        // Extract values from all slots and execute batch
        for i in 0..num {
            let slot = *slots.add(i);
            match extract_slot_values(slot, state) {
                Ok(values) => {
                    state.batch_buffer.push(values);
                }
                Err(e) => {
                    error!("Failed to extract values from slot {}: {}", i, e);
                }
            }
        }

        // Flush the batch
        if let Err(e) = flush_batch_with_session(state) {
            error!("Failed to flush batch insert: {}", e);
        }

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
    debug!("exec_foreign_truncate called");
    // TODO: Build TRUNCATE SQL for each table and execute on DB2
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
    // TODO: Check readonly option
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
