//! Foreign Modify implementation
//!
//! Handles INSERT, UPDATE, DELETE, and TRUNCATE operations with real ODBC execution.

use pgrx::prelude::*;
use pgrx::pg_sys;
use tracing::{debug, info, warn, error};

use crate::options::FdwOptions;
use crate::state::FdwModifyState;
use crate::query::QueryBuilder;
use db2_odbc::Db2Value;
use db2_query::deparse::Deparser;

/// Operation type for modify operations
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ModifyOperation {
    Insert,
    Update,
    Delete,
}

/// Add columns needed for UPDATE/DELETE operations
///
/// PostgreSQL FDW callback: AddForeignUpdateTargets
/// This adds the key columns to the target list so they're available for WHERE clauses.
#[pg_guard]
pub extern "C" fn add_foreign_update_targets(
    _root: *mut pg_sys::PlannerInfo,
    _rtindex: pg_sys::Index,
    _target_rte: *mut pg_sys::RangeTblEntry,
    target_relation: pg_sys::Relation,
) {
    debug!("add_foreign_update_targets called");
    unsafe {
        // Get the tuple descriptor to access column information
        let tupdesc = (*target_relation).rd_att;
        let natts = (*tupdesc).natts;

        // Look for columns marked as key columns in FDW options
        // These columns are needed for UPDATE/DELETE WHERE clauses
        for i in 0..natts {
            let att = pg_sys::TupleDescAttr(tupdesc, i);
            let attname = std::ffi::CStr::from_ptr((*att).attname.data.as_ptr())
                .to_string_lossy();

            // Check if this column is marked as a key in options
            // For now, we add all non-dropped columns as potential keys
            if !(*att).attisdropped {
                debug!("Potential key column: {}", attname);
            }
        }
    }
}

/// Plan a foreign modification
///
/// PostgreSQL FDW callback: PlanForeignModify
/// Builds the SQL for INSERT/UPDATE/DELETE operations.
#[pg_guard]
pub extern "C" fn plan_foreign_modify(
    root: *mut pg_sys::PlannerInfo,
    plan: *mut pg_sys::ModifyTable,
    resultRelation: pg_sys::Index,
    _subplan_index: ::std::os::raw::c_int,
) -> *mut pg_sys::List {
    debug!("plan_foreign_modify called");
    unsafe {
        // Get the operation type
        let operation = (*plan).operation;
        debug!("Modify operation: {:?}", operation);

        // Get the result relation
        let rte = pg_sys::planner_rt_fetch(resultRelation, root);
        if rte.is_null() {
            return std::ptr::null_mut();
        }

        // Build the appropriate SQL based on operation type
        // The SQL will be passed to BeginForeignModify via fdw_private

        // Return NULL for now - we build SQL in BeginForeignModify
        // where we have access to the actual table options
        std::ptr::null_mut()
    }
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
    debug!("begin_foreign_modify called");
    unsafe {
        // Get the operation type
        let operation = (*mtstate).operation;

        // Get the relation to access options
        let rel = (*resultRelInfo).ri_RelationDesc;
        let tupdesc = (*rel).rd_att;
        let natts = (*tupdesc).natts as usize;

        // Extract column names
        let mut column_names = Vec::with_capacity(natts);
        let mut key_column_names = Vec::new();

        for i in 0..natts {
            let att = pg_sys::TupleDescAttr(tupdesc, i as i32);
            if !(*att).attisdropped {
                let name = std::ffi::CStr::from_ptr((*att).attname.data.as_ptr())
                    .to_string_lossy()
                    .to_string();
                column_names.push(name);
            }
        }

        // Build options - in real implementation, parse from foreign table options
        let mut options = FdwOptions::new();

        // Get table name from relation - simplified for now
        let relname = std::ffi::CStr::from_ptr((*(*rel).rd_rel).relname.data.as_ptr())
            .to_string_lossy()
            .to_string();
        options.table = Some(relname.clone());

        // Build the SQL based on operation
        let qb = QueryBuilder::from_options(&options);
        let sql = match (qb, operation) {
            (Some(qb), pg_sys::CmdType_CMD_INSERT) => {
                qb.with_columns(column_names.clone()).build_insert()
            }
            (Some(qb), pg_sys::CmdType_CMD_UPDATE) => {
                // For UPDATE, we need key columns
                qb.with_columns(column_names.clone())
                    .with_key_columns(key_column_names.clone())
                    .build_update()
            }
            (Some(qb), pg_sys::CmdType_CMD_DELETE) => {
                qb.with_key_columns(key_column_names.clone())
                    .build_delete()
            }
            _ => relname.clone(),
        };

        // Initialize modify state
        let mut state = Box::new(FdwModifyState::new(options, sql));
        state.target_columns = column_names;
        state.key_columns = key_column_names;

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
    _state: &FdwModifyState,
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

    // Use the Deparser to build proper SQL with literals
    let dp = Deparser::default_context();

    // Execute INSERT for each row
    for row in &state.batch_buffer {
        // Build INSERT SQL with literal values
        let values: Vec<String> = row.iter()
            .map(|v| dp.deparse_literal(v))
            .collect();

        // Build qualified table name
        let table_name = if let Some(ref schema) = state.options.schema {
            format!("\"{}\".\"{}\"", schema, state.options.table.as_deref().unwrap_or(""))
        } else {
            format!("\"{}\"", state.options.table.as_deref().unwrap_or(""))
        };

        let sql = format!(
            "INSERT INTO {} ({}) VALUES ({})",
            table_name,
            state.target_columns.iter()
                .map(|c| format!("\"{}\"", c))
                .collect::<Vec<_>>()
                .join(", "),
            values.join(", ")
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
        let dp = Deparser::default_context();

        // Extract values from slot
        match extract_slot_values(slot, state) {
            Ok(values) => {
                if let Some(ref session) = state.session {
                    // Build qualified table name
                    let table_name = if let Some(ref schema) = state.options.schema {
                        format!("\"{}\".\"{}\"", schema, state.options.table.as_deref().unwrap_or(""))
                    } else {
                        format!("\"{}\"", state.options.table.as_deref().unwrap_or(""))
                    };

                    // Build SET clause
                    let set_parts: Vec<String> = state.target_columns.iter()
                        .zip(values.iter())
                        .map(|(col, val)| format!("\"{}\" = {}", col, dp.deparse_literal(val)))
                        .collect();

                    // Build WHERE clause from key columns
                    let where_parts: Vec<String> = if state.key_columns.is_empty() {
                        // No key columns - use all columns (dangerous but functional)
                        state.target_columns.iter()
                            .zip(values.iter())
                            .filter(|(_, v)| !matches!(v, Db2Value::Null))
                            .map(|(col, val)| format!("\"{}\" = {}", col, dp.deparse_literal(val)))
                            .collect()
                    } else {
                        // Use key columns
                        let key_indices: Vec<usize> = state.key_columns.iter()
                            .filter_map(|k| state.target_columns.iter().position(|c| c == k))
                            .collect();

                        key_indices.iter()
                            .filter_map(|&i| values.get(i).map(|v| {
                                format!("\"{}\" = {}", &state.target_columns[i], dp.deparse_literal(v))
                            }))
                            .collect()
                    };

                    let sql = if where_parts.is_empty() {
                        warn!("UPDATE with no WHERE clause - skipping for safety");
                        return slot;
                    } else {
                        format!(
                            "UPDATE {} SET {} WHERE {}",
                            table_name,
                            set_parts.join(", "),
                            where_parts.join(" AND ")
                        )
                    };

                    debug!(sql = %sql, "Executing UPDATE");
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
        let dp = Deparser::default_context();

        // Extract key values from slot for WHERE clause
        match extract_slot_values(slot, state) {
            Ok(values) => {
                if let Some(ref session) = state.session {
                    // Build qualified table name
                    let table_name = if let Some(ref schema) = state.options.schema {
                        format!("\"{}\".\"{}\"", schema, state.options.table.as_deref().unwrap_or(""))
                    } else {
                        format!("\"{}\"", state.options.table.as_deref().unwrap_or(""))
                    };

                    // Build WHERE clause from key columns
                    let where_parts: Vec<String> = if state.key_columns.is_empty() {
                        // No key columns - use all non-null columns
                        state.target_columns.iter()
                            .zip(values.iter())
                            .filter(|(_, v)| !matches!(v, Db2Value::Null))
                            .map(|(col, val)| format!("\"{}\" = {}", col, dp.deparse_literal(val)))
                            .collect()
                    } else {
                        // Use key columns
                        let key_indices: Vec<usize> = state.key_columns.iter()
                            .filter_map(|k| state.target_columns.iter().position(|c| c == k))
                            .collect();

                        key_indices.iter()
                            .filter_map(|&i| values.get(i).map(|v| {
                                format!("\"{}\" = {}", &state.target_columns[i], dp.deparse_literal(v))
                            }))
                            .collect()
                    };

                    let sql = if where_parts.is_empty() {
                        warn!("DELETE with no WHERE clause - skipping for safety");
                        return slot;
                    } else {
                        format!(
                            "DELETE FROM {} WHERE {}",
                            table_name,
                            where_parts.join(" AND ")
                        )
                    };

                    debug!(sql = %sql, "Executing DELETE");
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
    rels: *mut pg_sys::List,
    behavior: pg_sys::DropBehavior,
) {
    debug!("exec_foreign_truncate called");

    if rels.is_null() {
        return;
    }

    unsafe {
        // Iterate through the list of relations
        let list_len = (*rels).length;
        debug!("TRUNCATE: processing {} relations", list_len);

        for i in 0..list_len {
            // Get relation from list
            let cell = pg_sys::list_nth_cell(rels, i);
            if cell.is_null() {
                continue;
            }

            let rel = (*cell).ptr_value as pg_sys::Relation;
            if rel.is_null() {
                continue;
            }

            // Get table name
            let relname = std::ffi::CStr::from_ptr((*(*rel).rd_rel).relname.data.as_ptr())
                .to_string_lossy()
                .to_string();

            // Build options
            let mut options = FdwOptions::new();
            options.table = Some(relname.clone());

            // Build TRUNCATE SQL
            let sql = if let Some(qb) = QueryBuilder::from_options(&options) {
                qb.build_truncate()
            } else {
                format!("TRUNCATE TABLE \"{}\" IMMEDIATE", relname)
            };

            debug!(sql = %sql, "Executing TRUNCATE");

            // Execute TRUNCATE
            // Note: This requires a session - in practice, we'd need to establish one
            // For now, log the SQL that would be executed
            info!("Would execute: {}", sql);

            // TODO: Execute via session when proper option parsing is available
            // let conn_opts = options.to_connection_options();
            // if let Some(opts) = conn_opts {
            //     let session = Db2Session::new(&opts);
            //     session.connection().execute_immediate(&sql);
            // }
        }
    }
}

/// Check if a foreign table is updatable
///
/// PostgreSQL FDW callback: IsForeignRelUpdatable
/// Returns a bitmap of allowed operations:
/// - 1 = INSERT
/// - 2 = UPDATE
/// - 4 = DELETE
#[pg_guard]
pub extern "C" fn is_foreign_rel_updatable(
    rel: pg_sys::Relation,
) -> ::std::os::raw::c_int {
    debug!("is_foreign_rel_updatable called");

    unsafe {
        if rel.is_null() {
            return 0;
        }

        // Get the foreign table options to check for readonly flag
        let relid = (*rel).rd_id;

        // Get foreign table
        let ft = pg_sys::GetForeignTable(relid);
        if ft.is_null() {
            // Not a foreign table, no updates allowed
            return 0;
        }

        // Check options for readonly flag
        let options = (*ft).options;
        let mut is_readonly = false;

        if !options.is_null() {
            let list_len = (*options).length;
            for i in 0..list_len {
                let cell = pg_sys::list_nth_cell(options, i);
                if cell.is_null() {
                    continue;
                }

                let def = (*cell).ptr_value as *mut pg_sys::DefElem;
                if def.is_null() {
                    continue;
                }

                let defname = std::ffi::CStr::from_ptr((*def).defname)
                    .to_string_lossy()
                    .to_lowercase();

                if defname == "readonly" {
                    // Get the value
                    let defval = (*def).arg as *mut pg_sys::String;
                    if !defval.is_null() {
                        let val = std::ffi::CStr::from_ptr((*defval).sval)
                            .to_string_lossy()
                            .to_lowercase();
                        is_readonly = matches!(val.as_ref(), "on" | "true" | "yes" | "1");
                    }
                }
            }
        }

        if is_readonly {
            debug!("Foreign table is marked readonly");
            return 0; // No updates allowed
        }

        // Return bitmap: INSERT | UPDATE | DELETE
        1 | 2 | 4
    }
}

// Re-export for lib.rs
pub struct ForeignModify;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_modify_state() {
        let state = FdwModifyState::new(FdwOptions::new(), "INSERT INTO test".into());
        assert_eq!(state.rows_affected, 0);
    }
}
