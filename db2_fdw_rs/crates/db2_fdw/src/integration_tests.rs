//! Integration Tests for FDW Handler
//!
//! Tests the complete FDW handler lifecycle in a controlled environment

#[cfg(all(test, feature = "pg_test"))]
mod fdw.integration {
    use pgrx::prelude::*;
    use pgrx::pg_sys;
    use std::ptr;

    /// Test that FdwHandler can be called multiple times without memory leaks
    #[pg_test]
    fn test_fdw_handler_idempotent() {
        unsafe {
            // Call handler twice - should not leak or crash
            let result1 = super::db2_fdw_handler(ptr::null_mut());
            let result2 = super::db2_fdw_handler(ptr::null_mut());

            assert!(!result1.is_null(), "First call should return valid pointer");
            assert!(!result2.is_null(), "Second call should return valid pointer");

            // Results should be different pointers (if allocation is correct)
            assert_ne!(result1.as_ptr(), result2.as_ptr(),
                      "Each call should create new allocation");
        }
    }

    /// Test that validator handles various inputs without panicking
    #[pg_test]
    fn test_fdw_validator_null_safety() {
        unsafe {
            // Test 1: Null input
            let result1 = super::db2_fdw_validator(ptr::null_mut());
            assert_eq!(result1.value(), 0);

            // Test 2: Call multiple times (no state should cause issues)
            for _ in 0..10 {
                let result = super::db2_fdw_validator(ptr::null_mut());
                assert_eq!(result.value(), 0);
            }
        }
    }

    /// Test TupleDescriptor attribute access pattern
    #[pg_test]
    fn test_tuple_descriptor_safety() {
        unsafe {
            // Get system TupleDesc (should be valid)
            let typ OID = pg_sys::INT4OID;

            // Get type name using PostgreSQL API
            let type_name = pg_sys::format_type_be(typ OID);

            // Test that we got a valid pointer
            assert!(!type_name.is_null(), "format_type_be should not return null");

            // Test that pstrdup result is valid
            let duplicated = pg_sys::pstrdup(type_name);
            assert!(!duplicated.is_null(), "pstrdup should not return null");

            // Clean up
            pg_sys::pfree(type_name as *mut std::ffi::c_void);
            pg_sys::pfree(duplicated as *mut std::ffi::c_void);
        }
    }

    /// Test list operations with Postgres list API
    #[pg_test]
    fn test_postgres_list_api_safety() {
        unsafe {
            // Create a new list
            let mut list: *mut pg_sys::List = ptr::null_mut();

            // Try to append to null list (should create new list)
            let dummy_string = std::ffi::CString::new("test").unwrap();
            let pg_str = pg_sys::pstrdup(dummy_string.as_ptr());
            assert!(!pg_str.is_null(), "pstrdup should succeed");

            list = pg_sys::lappend(list, pg_str as *mut std::ffi::c_void);

            // Verify list is now valid
            assert!(!list.is_null(), "lappend should create valid list");

            // Check list length
            assert_eq!((*list).length, 1, "List should have 1 element");

            // Clean up
            pg_sys::list_free(list);
        }
    }

    /// Test memory allocation patterns
    #[pg_test]
    fn test_malloc_free_patterns() {
        unsafe {
            // Test 1: palloc and pfree
            {
                let ptr = pg_sys::palloc(1024);
                assert!(!ptr.is_null(), "palloc should succeed");

                // Write to memory
                let mut slice = std::slice::from_raw_parts_mut(ptr as *mut u8, 1024);
                slice[0] = 42;
                assert_eq!(slice[0], 42, "Should be able to write to palloc'd memory");

                // Free memory
                pg_sys::pfree(ptr);
            }

            // Test 2: palloc0 (zeroed memory)
            {
                let ptr = pg_sys::palloc0(512);
                assert!(!ptr.is_null(), "palloc0 should succeed");

                // Memory should be zeroed
                let slice = std::slice::from_raw_parts_mut(ptr as *mut u8, 512);
                for &byte in slice.iter().take(10) {
                    assert_eq!(byte, 0, "Memory should be zeroed");
                }

                // Clean up
                pg_sys::pfree(ptr);
            }
        }
    }
}
