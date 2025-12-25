/**
 * DB2 FDW Core - Safe Rust implementation for critical FDW components
 *
 * This header provides the C interface to the Rust core library.
 * Link with: -ldb2_fdw_core
 *
 * All functions are thread-safe and panic-safe.
 */

#ifndef DB2_FDW_CORE_H
#define DB2_FDW_CORE_H

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ============================================================================
 * Error Codes
 * ============================================================================ */

typedef enum {
    DB2_CORE_SUCCESS = 0,
    DB2_CORE_CONNECTION_FAILED = -1,
    DB2_CORE_INVALID_HANDLE = -2,
    DB2_CORE_QUERY_FAILED = -3,
    DB2_CORE_OUT_OF_MEMORY = -4,
    DB2_CORE_INVALID_PARAMETER = -5,
    DB2_CORE_TIMEOUT = -6,
    DB2_CORE_NOT_CONNECTED = -7,
    DB2_CORE_LOB_ERROR = -8,
    DB2_CORE_ENCODING_ERROR = -9,
    DB2_CORE_INTERNAL_ERROR = -99
} Db2CoreErrorCode;

/* ============================================================================
 * Opaque Handle Types
 * ============================================================================ */

/** Opaque connection handle */
typedef void* Db2ConnHandle;

/** Opaque BLOB handle */
typedef void* Db2BlobHandle;

/** Opaque CLOB handle */
typedef void* Db2ClobHandle;

/* ============================================================================
 * Library Initialization
 * ============================================================================ */

/**
 * Initialize the library.
 * Call once at PostgreSQL startup.
 *
 * @return 0 on success, negative error code on failure
 */
int db2_core_init(void);

/**
 * Shutdown the library.
 * Call at PostgreSQL shutdown. Closes all connections.
 *
 * @return 0 on success, negative error code on failure
 */
int db2_core_shutdown(void);

/**
 * Get library version string.
 *
 * @return Null-terminated version string (do not free)
 */
const char* db2_core_version(void);

/**
 * Get last error message for current thread.
 *
 * @return Null-terminated error message or NULL if no error
 *         Valid until next error occurs on this thread.
 */
const char* db2_core_last_error(void);

/* ============================================================================
 * Environment Functions
 * ============================================================================ */

/**
 * Initialize ODBC environment.
 * Called automatically, but can be called explicitly.
 *
 * @return 0 on success, negative error code on failure
 */
int db2_env_init(void);

/**
 * Set NLS_LANG environment variable safely.
 * This fixes the use-after-free bug in the original C code.
 *
 * @param nls_lang  NLS_LANG value (e.g., "AMERICAN_AMERICA.UTF8")
 * @return 0 on success, negative error code on failure
 */
int db2_env_set_nls_lang(const char* nls_lang);

/* ============================================================================
 * Direct Connection Functions (not pooled)
 * ============================================================================ */

/**
 * Connect with password authentication.
 *
 * @param dsn              Data source name
 * @param user             Username
 * @param password         Password
 * @param timeout_seconds  Connection timeout (0 = no timeout)
 * @return Connection handle or NULL on failure
 */
Db2ConnHandle db2_conn_connect_password(
    const char* dsn,
    const char* user,
    const char* password,
    uint32_t timeout_seconds
);

/**
 * Connect with JWT token authentication (DB2 11.5.4+).
 *
 * @param dsn              Data source name
 * @param token            JWT token
 * @param timeout_seconds  Connection timeout (0 = no timeout)
 * @return Connection handle or NULL on failure
 */
Db2ConnHandle db2_conn_connect_jwt(
    const char* dsn,
    const char* token,
    uint32_t timeout_seconds
);

/**
 * Close a connection.
 *
 * @param handle  Connection handle (may be NULL)
 * @return 0 on success
 */
int db2_conn_close(Db2ConnHandle handle);

/**
 * Check if connection is valid.
 *
 * @param handle  Connection handle
 * @return 1 if valid, 0 if invalid or NULL
 */
int db2_conn_is_valid(Db2ConnHandle handle);

/**
 * Get connection ID.
 *
 * @param handle  Connection handle
 * @return Connection ID or 0 if invalid
 */
uint64_t db2_conn_get_id(Db2ConnHandle handle);

/* ============================================================================
 * Connection Pool Functions
 * ============================================================================ */

/** Pool statistics */
typedef struct {
    size_t connection_count;
    uint64_t total_use_count;
} Db2PoolStats;

/**
 * Get a pooled connection with password authentication.
 * Reuses existing connection if available.
 *
 * @param dsn              Data source name
 * @param user             Username
 * @param password         Password
 * @param timeout_seconds  Connection timeout (0 = no timeout)
 * @return Connection handle or NULL on failure
 */
Db2ConnHandle db2_pool_get_password(
    const char* dsn,
    const char* user,
    const char* password,
    uint32_t timeout_seconds
);

/**
 * Get a pooled connection with JWT authentication.
 *
 * @param dsn              Data source name
 * @param token            JWT token
 * @param timeout_seconds  Connection timeout (0 = no timeout)
 * @return Connection handle or NULL on failure
 */
Db2ConnHandle db2_pool_get_jwt(
    const char* dsn,
    const char* token,
    uint32_t timeout_seconds
);

/**
 * Release a pooled connection.
 * The connection is returned to the pool for reuse.
 *
 * @param handle  Connection handle
 * @return 0 on success
 */
int db2_pool_release(Db2ConnHandle handle);

/**
 * Close all pooled connections.
 *
 * @return 0 on success
 */
int db2_pool_close_all(void);

/**
 * Clean up stale pooled connections.
 *
 * @return 0 on success
 */
int db2_pool_cleanup(void);

/**
 * Get pool statistics.
 *
 * @return Pool statistics structure
 */
Db2PoolStats db2_pool_stats(void);

/* ============================================================================
 * BLOB Functions
 * ============================================================================ */

/**
 * Create a new BLOB with optional capacity hint.
 *
 * @param capacity  Initial capacity (0 for default)
 * @return BLOB handle or NULL on failure
 */
Db2BlobHandle db2_blob_new(size_t capacity);

/**
 * Append data to BLOB.
 *
 * @param handle  BLOB handle
 * @param data    Data to append
 * @param len     Length of data
 * @return 0 on success, negative error code on failure
 */
int db2_blob_append(Db2BlobHandle handle, const unsigned char* data, size_t len);

/**
 * Get BLOB data pointer and length.
 *
 * @param handle   BLOB handle
 * @param out_len  Output: length of data
 * @return Pointer to data (valid until BLOB modified/freed) or NULL
 */
const unsigned char* db2_blob_data(Db2BlobHandle handle, size_t* out_len);

/**
 * Check if BLOB was truncated due to size limit.
 *
 * @param handle  BLOB handle
 * @return 1 if truncated, 0 otherwise
 */
int db2_blob_is_truncated(Db2BlobHandle handle);

/**
 * Free BLOB.
 *
 * @param handle  BLOB handle (may be NULL)
 */
void db2_blob_free(Db2BlobHandle handle);

/* ============================================================================
 * CLOB Functions
 * ============================================================================ */

/**
 * Create a new CLOB.
 *
 * @return CLOB handle or NULL on failure
 */
Db2ClobHandle db2_clob_new(void);

/**
 * Append text to CLOB.
 *
 * @param handle  CLOB handle
 * @param text    Null-terminated UTF-8 text
 * @return 0 on success, negative error code on failure
 */
int db2_clob_append(Db2ClobHandle handle, const char* text);

/**
 * Get CLOB data as C string.
 *
 * @param handle  CLOB handle
 * @return Pointer to null-terminated string (valid until CLOB modified/freed)
 *         or NULL on error
 */
const char* db2_clob_data(Db2ClobHandle handle);

/**
 * Get CLOB length in bytes.
 *
 * @param handle  CLOB handle
 * @return Length in bytes or 0 if invalid
 */
size_t db2_clob_len(Db2ClobHandle handle);

/**
 * Free CLOB.
 *
 * @param handle  CLOB handle (may be NULL)
 */
void db2_clob_free(Db2ClobHandle handle);

/* ============================================================================
 * Utility Functions
 * ============================================================================ */

/**
 * Free a string allocated by the Rust library.
 *
 * @param ptr  String pointer (may be NULL)
 */
void db2_free_string(char* ptr);

#ifdef __cplusplus
}
#endif

#endif /* DB2_FDW_CORE_H */
