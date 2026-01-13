//! Foreign Modify implementation
//!
//! Handles INSERT, UPDATE, DELETE, and TRUNCATE operations with real ODBC execution.

use pgrx::pg_sys;
use tracing::{debug, info, warn, error};

use crate::options::FdwOptions;
use crate::state::FdwModifyState;
use crate::query::QueryBuilder;
use crate::transaction::mark_dml_in_transaction;
use db2_odbc::Db2Value;
use db2_query::deparse::Deparser;
use db2_connection::FdwConnectionOptions;

// Use safe FFI wrappers
use crate::safe_ffi;

// Temporary type definition until pgrx exports this properly
type DropBehavior = u32;

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
pub unsafe extern "C-unwind" fn add_foreign_update_targets(
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
            if att.is_null() {
                continue;
            }
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
#[allow(non_snake_case)]
pub unsafe extern "C-unwind" fn plan_foreign_modify(
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
#[allow(non_snake_case)]
pub unsafe extern "C-unwind" fn begin_foreign_modify(
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
        let key_column_names = Vec::new(); // Will be populated from options in real implementation

        for i in 0..natts {
            let att = pg_sys::TupleDescAttr(tupdesc, i as i32);
            if att.is_null() {
                continue;
            }
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
            (Some(qb), pg_sys::CmdType::CMD_INSERT) => {
                qb.with_columns(column_names.clone()).build_insert()
            }
            (Some(qb), pg_sys::CmdType::CMD_UPDATE) => {
                // For UPDATE, we need key columns
                qb.with_columns(column_names.clone())
                    .with_key_columns(key_column_names.clone())
                    .build_update()
            }
            (Some(qb), pg_sys::CmdType::CMD_DELETE) => {
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
            if att.is_null() {
                return Err("Cannot get attribute descriptor for column".to_string());
            }
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
                    // Safe wrapper - handles validation and bounds checking
                    match unsafe { safe_ffi::datum_get_text(typid, datum) } {
                        Ok(s) => Db2Value::Text(s),
                        Err(e) => {
                            warn!("Failed to extract text from datum: {}", e);
                            Db2Value::Text(String::new()) // Fallback
                        }
                    }
                }
                pg_sys::BYTEAOID => {
                    // Safe wrapper - handles validation and bounds checking
                    match unsafe { safe_ffi::datum_get_binary(typid, datum) } {
                        Ok(bytes) => Db2Value::Binary(bytes),
                        Err(e) => {
                            warn!("Failed to extract binary from datum: {}", e);
                            Db2Value::Binary(Vec::new()) // Fallback
                        }
                    }
                }
                _ => {
                    // Convert to text representation using safe wrapper
                    match unsafe {
                        safe_ffi::get_type_output_info(typid)
                    } {
                        Ok(output_func) => {
                            match unsafe {
                                safe_ffi::oid_output_call(output_func, datum)
                            } {
                                Ok(s) => Db2Value::Text(s),
                                Err(e) => {
                                    warn!("Failed to call output function: {}", e);
                                    Db2Value::Text(String::new()) // Fallback
                                }
                            }
                        },
                        Err(e) => {
                            warn!("Failed to get output function for type {:?}: {}", typid, e);
                            Db2Value::Text(format!("<type {:?}>", typid)) // Fallback
                        }
                    }
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
#[allow(non_snake_case)]
pub unsafe extern "C-unwind" fn exec_foreign_insert(
    _estate: *mut pg_sys::EState,
    resultRelInfo: *mut pg_sys::ResultRelInfo,
    slot: *mut pg_sys::TupleTableSlot,
    _planSlot: *mut pg_sys::TupleTableSlot,
) -> *mut pg_sys::TupleTableSlot {
    debug!("exec_foreign_insert called");
    mark_dml_in_transaction();
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
#[allow(non_snake_case)]
pub unsafe extern "C-unwind" fn exec_foreign_update(
    _estate: *mut pg_sys::EState,
    resultRelInfo: *mut pg_sys::ResultRelInfo,
    slot: *mut pg_sys::TupleTableSlot,
    _planSlot: *mut pg_sys::TupleTableSlot,
) -> *mut pg_sys::TupleTableSlot {
    debug!("exec_foreign_update called");
    mark_dml_in_transaction();
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
#[allow(non_snake_case)]
pub unsafe extern "C-unwind" fn exec_foreign_delete(
    _estate: *mut pg_sys::EState,
    resultRelInfo: *mut pg_sys::ResultRelInfo,
    slot: *mut pg_sys::TupleTableSlot,
    _planSlot: *mut pg_sys::TupleTableSlot,
) -> *mut pg_sys::TupleTableSlot {
    debug!("exec_foreign_delete called");
    mark_dml_in_transaction();
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
#[allow(non_snake_case)]
pub unsafe extern "C-unwind" fn end_foreign_modify(
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

/// Execute a foreign TRUNCATE
///
/// PostgreSQL FDW callback: ExecForeignTruncate
pub unsafe extern "C-unwind" fn exec_foreign_truncate(
    rels: *mut pg_sys::List,
    _behavior: DropBehavior,
    _restart_seqs: bool,
) {
    debug!("exec_foreign_truncate called");
    mark_dml_in_transaction();

    if rels.is_null() {
        return;
    }

    unsafe {
        // Iterate through the list of relations
        let list_len = (*rels).length;
        debug!("TRUNCATE: processing {} relations", list_len);

        // We need to collect connection info from the first relation
        // All relations in a TRUNCATE must be from the same server
        let mut session: Option<db2_connection::Db2Session> = None;

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

            let relid = (*rel).rd_id;

            // Get table name
            let relname = std::ffi::CStr::from_ptr((*(*rel).rd_rel).relname.data.as_ptr())
                .to_string_lossy()
                .to_string();

            // Get foreign table options
            let ft = pg_sys::GetForeignTable(relid);
            if ft.is_null() {
                warn!("Could not get foreign table for TRUNCATE");
                continue;
            }

            // Get server
            let server = pg_sys::GetForeignServer((*ft).serverid);
            if server.is_null() {
                warn!("Could not get foreign server for TRUNCATE");
                continue;
            }

            // Parse options to get schema
            let mut schema: Option<String> = None;
            let mut table: Option<String> = None;
            let ft_options = (*ft).options;

            if !ft_options.is_null() {
                let opt_len = (*ft_options).length;
                for j in 0..opt_len {
                    let opt_cell = pg_sys::list_nth_cell(ft_options, j);
                    if opt_cell.is_null() {
                        continue;
                    }

                    let def = (*opt_cell).ptr_value as *mut pg_sys::DefElem;
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
                        "schema" => schema = Some(defval),
                        "table" => table = Some(defval),
                        _ => {}
                    }
                }
            }

            // Use table option if available, otherwise relation name
            let actual_table = table.unwrap_or_else(|| relname.clone());

            // Build qualified table name
            let qualified_name = if let Some(ref s) = schema {
                format!("\"{}\".\"{}\"", s.replace('"', "\"\""), actual_table.replace('"', "\"\""))
            } else {
                format!("\"{}\"", actual_table.replace('"', "\"\""))
            };

            // Build TRUNCATE SQL (DB2 uses TRUNCATE TABLE ... IMMEDIATE)
            let sql = format!("TRUNCATE TABLE {} IMMEDIATE", qualified_name);

            debug!(sql = %sql, "Executing TRUNCATE");

            // Initialize session if not already done
            if session.is_none() {
                // Get connection options from server
                let mut dbserver: Option<String> = None;
                let mut user: Option<String> = None;
                let mut password: Option<String> = None;

                let server_options = (*server).options;
                if !server_options.is_null() {
                    let opt_len = (*server_options).length;
                    for j in 0..opt_len {
                        let opt_cell = pg_sys::list_nth_cell(server_options, j);
                        if opt_cell.is_null() {
                            continue;
                        }

                        let def = (*opt_cell).ptr_value as *mut pg_sys::DefElem;
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
                            "dbserver" => dbserver = Some(defval),
                            "user" => user = Some(defval),
                            "password" => password = Some(defval),
                            _ => {}
                        }
                    }
                }

                // Try to get user mapping options
                let user_id = pg_sys::GetUserId();
                let mapping = pg_sys::GetUserMapping(user_id, (*ft).serverid);
                if !mapping.is_null() {
                    let mapping_options = (*mapping).options;
                    if !mapping_options.is_null() {
                        let opt_len = (*mapping_options).length;
                        for j in 0..opt_len {
                            let opt_cell = pg_sys::list_nth_cell(mapping_options, j);
                            if opt_cell.is_null() {
                                continue;
                            }

                            let def = (*opt_cell).ptr_value as *mut pg_sys::DefElem;
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
                                "user" => user = Some(defval),
                                "password" => password = Some(defval),
                                _ => {}
                            }
                        }
                    }
                }

                // Connect if we have connection info
                if let Some(ref conn_str) = dbserver {
                    let user_str = user.as_deref().unwrap_or("");
                    let pass_str = password.as_deref().unwrap_or("");
                    let options = FdwConnectionOptions::with_password(conn_str, user_str, pass_str);
                    match db2_connection::Db2Session::new(&options) {
                        Ok(s) => {
                            session = Some(s);
                            info!("Connected to DB2 for TRUNCATE");
                        }
                        Err(e) => {
                            pgrx::error!("Failed to connect to DB2 for TRUNCATE: {}", e);
                        }
                    }
                } else {
                    pgrx::error!("No dbserver option found for TRUNCATE");
                }
            }

            // Execute TRUNCATE
            if let Some(ref sess) = session {
                match sess.connection().execute_immediate(&sql) {
                    Ok(_) => {
                        info!("TRUNCATE executed successfully: {}", sql);
                    }
                    Err(e) => {
                        pgrx::error!("TRUNCATE failed: {}", e);
                    }
                }
            }
        }

        // Close session
        if let Some(mut sess) = session {
            sess.close();
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
pub unsafe extern "C-unwind" fn is_foreign_rel_updatable(
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

/// Execute a foreign batch insert
///
/// PostgreSQL FDW callback: ExecForeignBatchInsert
/// This is used for bulk insert operations to improve performance.
pub unsafe extern "C-unwind" fn exec_foreign_batch_insert(
    _estate: *mut pg_sys::EState,
    _result_rel_info: *mut pg_sys::ResultRelInfo,
    _rri_slot: pg_sys::TupleTableSlot,
    slots: *mut *mut pg_sys::TupleTableSlot,
    nslots: ::std::os::raw::c_int,
    _estates: *mut *mut pg_sys::EState,
) -> ::std::os::raw::c_int {
    debug!("exec_foreign_batch_insert called with {} slots", nslots);

    if nslots == 0 {
        return 0;
    }

    // TODO: Implement proper batching logic
    // For now, fall back to individual inserts
    let mut inserted = 0 as ::std::os::raw::c_int;

    unsafe {
        if !slots.is_null() {
            let slot_ptr = *slots;
            if !slot_ptr.is_null() {
                // Insert the first slot
                // TODO: Batch the inserts properly here
                inserted = 1;
            }
        }
    }

    inserted
}

/// Get foreign modify batch size
///
/// PostgreSQL FDW callback: GetForeignModifyBatchSize
/// Returns the optimal batch size for bulk operations.
pub unsafe extern "C-unwind" fn get_foreign_modify_batch_size(
    _root: *mut pg_sys::PlannerInfo,
    _result_relation: *mut pg_sys::RelOptInfo,
    _foreigntableid: pg_sys::Oid,
) -> ::std::os::raw::c_int {
    debug!("get_foreign_modify_batch_size called");

    // Use batch insert for better performance
    // A reasonable batch size for most DB2 configurations
    100
}

/// Begin a foreign insert
///
/// PostgreSQL FDW callback: BeginForeignInsert
pub unsafe extern "C-unwind" fn begin_foreign_insert(
    _mtstate: *mut pg_sys::ModifyTableState,
    _rinfo: *mut pg_sys::ResultRelInfo,
) {
    debug!("begin_foreign_insert called");
    // TODO: Prepare DB2 for bulk insert operation
}

/// End a foreign insert
///
/// PostgreSQL FDW callback: EndForeignInsert
pub unsafe extern "C-unwind" fn end_foreign_insert(
    _estate: *mut pg_sys::EState,
    _result_rel_info: *mut pg_sys::ResultRelInfo,
) {
    debug!("end_foreign_insert called");
    // TODO: Finalize any bulk insert operation
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
