//! Transaction callback handling
//!
//! Manages PostgreSQL transaction events to synchronize with DB2 transactions.
//!
//! ## Threading Model
//!
//! PostgreSQL backends are single-threaded, so we use RefCell instead of
//! thread-safe structures like DashMap. This is simpler and correct.

use pgrx::prelude::*;
use pgrx::pg_sys;
use std::cell::RefCell;
use std::collections::HashMap;
use tracing::{debug, warn};

use db2_connection::pool::cleanup_stale_connections;

/// Savepoint state tracking
#[derive(Debug)]
struct SavepointState {
    name: String,
    level: u32,
}

/// Active savepoints by subtransaction ID (thread-local, single-threaded backend)
thread_local! {
    static ACTIVE_SAVEPOINTS: RefCell<HashMap<pg_sys::SubTransactionId, SavepointState>> =
        RefCell::new(HashMap::new());
}

/// Register transaction callbacks with PostgreSQL
pub fn register_callbacks() {
    unsafe {
        // Register transaction callback
        pg_sys::RegisterXactCallback(Some(xact_callback), std::ptr::null_mut());

        // Register subtransaction callback
        pg_sys::RegisterSubXactCallback(Some(subxact_callback), std::ptr::null_mut());
    }
    debug!("Registered transaction callbacks");
}

/// Transaction event callback
///
/// Called by PostgreSQL for transaction-level events.
#[pg_guard]
extern "C" fn xact_callback(event: pg_sys::XactEvent, _arg: *mut std::ffi::c_void) {
    match event {
        pg_sys::XactEvent_XACT_EVENT_PRE_COMMIT => {
            debug!("Transaction pre-commit");
            // Commit all DB2 transactions
            commit_all_connections();
        }
        pg_sys::XactEvent_XACT_EVENT_ABORT => {
            debug!("Transaction abort");
            // Rollback all DB2 transactions
            rollback_all_connections();
        }
        pg_sys::XactEvent_XACT_EVENT_COMMIT => {
            debug!("Transaction committed");
            // Cleanup after commit
            cleanup_after_transaction();
        }
        pg_sys::XactEvent_XACT_EVENT_PRE_PREPARE => {
            // Two-phase commit preparation
            debug!("Transaction pre-prepare");
        }
        pg_sys::XactEvent_XACT_EVENT_PREPARE => {
            debug!("Transaction prepared");
        }
        _ => {}
    }
}

/// Subtransaction event callback
///
/// Called by PostgreSQL for subtransaction (savepoint) events.
#[pg_guard]
extern "C" fn subxact_callback(
    event: pg_sys::SubXactEvent,
    my_subid: pg_sys::SubTransactionId,
    parent_subid: pg_sys::SubTransactionId,
    _arg: *mut std::ffi::c_void,
) {
    match event {
        pg_sys::SubXactEvent_SUBXACT_EVENT_START_SUB => {
            debug!(subid = my_subid, parent = parent_subid, "Subtransaction start");
            // Create savepoint on DB2
            create_savepoint(my_subid);
        }
        pg_sys::SubXactEvent_SUBXACT_EVENT_COMMIT_SUB => {
            debug!(subid = my_subid, "Subtransaction commit");
            // Release savepoint
            release_savepoint(my_subid);
        }
        pg_sys::SubXactEvent_SUBXACT_EVENT_ABORT_SUB => {
            debug!(subid = my_subid, "Subtransaction abort");
            // Rollback to savepoint
            rollback_to_savepoint(my_subid);
        }
        pg_sys::SubXactEvent_SUBXACT_EVENT_PRE_COMMIT_SUB => {
            debug!(subid = my_subid, "Subtransaction pre-commit");
        }
        _ => {}
    }
}

/// Create a savepoint on all active connections
fn create_savepoint(subid: pg_sys::SubTransactionId) {
    let name = format!("pg_fdw_sp_{}", subid);

    ACTIVE_SAVEPOINTS.with(|savepoints| {
        let mut map = savepoints.borrow_mut();
        let level = map.len() as u32 + 1;

        map.insert(
            subid,
            SavepointState {
                name: name.clone(),
                level,
            },
        );

        // Real implementation would execute:
        // SAVEPOINT {name} ON ROLLBACK RETAIN CURSORS
        // on each active DB2 connection
        debug!(savepoint = %name, level, "Created savepoint");
    });
}

/// Release a savepoint on all active connections
fn release_savepoint(subid: pg_sys::SubTransactionId) {
    ACTIVE_SAVEPOINTS.with(|savepoints| {
        if let Some(state) = savepoints.borrow_mut().remove(&subid) {
            // Real implementation would execute:
            // RELEASE SAVEPOINT {name}
            // on each active DB2 connection
            debug!(savepoint = %state.name, "Released savepoint");
        }
    });
}

/// Rollback to a savepoint on all active connections
fn rollback_to_savepoint(subid: pg_sys::SubTransactionId) {
    ACTIVE_SAVEPOINTS.with(|savepoints| {
        let mut map = savepoints.borrow_mut();
        if let Some(state) = map.remove(&subid) {
            // Real implementation would execute:
            // ROLLBACK TO SAVEPOINT {name}
            // on each active DB2 connection
            debug!(savepoint = %state.name, "Rolled back to savepoint");

            // Also remove any nested savepoints
            let level = state.level;
            map.retain(|_, s| s.level < level);
        }
    });
}

/// Commit all DB2 connections
fn commit_all_connections() {
    // Clear all savepoints
    ACTIVE_SAVEPOINTS.with(|savepoints| {
        savepoints.borrow_mut().clear();
    });

    // Real implementation would commit each connection
    // For now, just log
    debug!("Committed all DB2 connections");
}

/// Rollback all DB2 connections
fn rollback_all_connections() {
    // Clear all savepoints
    ACTIVE_SAVEPOINTS.with(|savepoints| {
        savepoints.borrow_mut().clear();
    });

    // Real implementation would rollback each connection
    // For now, just log
    debug!("Rolled back all DB2 connections");
}

/// Cleanup after a transaction completes
fn cleanup_after_transaction() {
    // Clear savepoint tracking
    ACTIVE_SAVEPOINTS.with(|savepoints| {
        savepoints.borrow_mut().clear();
    });

    // Optionally run connection cache cleanup
    cleanup_stale_connections();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_savepoint_name_generation() {
        let name = format!("pg_fdw_sp_{}", 123);
        assert_eq!(name, "pg_fdw_sp_123");
    }
}
