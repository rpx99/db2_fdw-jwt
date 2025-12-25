#include <string.h>
#include <sqlcli1.h>
#include <postgres_ext.h>
#include "db2_fdw.h"

/* Rust FFI core library for safe LOB operations */
#ifdef USE_RUST_CORE
#include "db2_fdw_core.h"
#endif

/** global variables */

/** external variables */
extern char         db2Message[ERRBUFSIZE];/* contains DB2 error messages, set by db2CheckErr()             */

/** external prototypes */
extern void*        db2alloc             (const char* type, size_t size);
extern void*        db2realloc           (void* p, size_t size);
extern void         db2Debug1            (const char* message, ...);
extern void         db2Debug2            (const char* message, ...);
extern void         db2Debug3            (const char* message, ...);
extern SQLRETURN    db2CheckErr          (SQLRETURN status, SQLHANDLE handle, SQLSMALLINT handleType, int line, char* file);
extern void         db2Error_d           (db2error sqlstate, const char* message, const char* detail, ...);

/** internal prototypes */
void                db2GetLob            (DB2Session* session, DB2Column* column, int cidx, char** value, long* value_len, unsigned long trunc);

/** db2GetLob
 *   Get the LOB contents and store them in *value and *value_len.
 *   If "trunc" is nonzero, it contains the number of bytes or characters to get.
 *
 *   FIXED: Original code had potential buffer overflow issues with realloc/memcpy.
 *   The Rust version uses bounds-checked Vec operations.
 */

#ifdef USE_RUST_CORE
/*
 * Safe version using Rust FFI - bounds-checked, no buffer overflow possible
 */
void db2GetLob (DB2Session* session, DB2Column* column, int cidx, char** value, long* value_len, unsigned long trunc) {
  SQLRETURN      rc  = SQL_SUCCESS;
  SQLLEN         ind = 0;
  SQLCHAR        buf[LOB_CHUNK_SIZE+1];
  Db2BlobHandle  blob = NULL;
  size_t         blob_len = 0;
  const unsigned char* blob_data = NULL;

  db2Debug1("> db2GetLob (Rust safe version)");
  db2Debug2("  column->colName: '%s'",column->colName);
  db2Debug2("  cidx           :  %d ",cidx);

  /* Create Rust BLOB with bounds checking */
  blob = db2_blob_new(LOB_CHUNK_SIZE * 4);
  if (blob == NULL) {
    db2Error_d(FDW_OUT_OF_MEMORY, "error fetching result: failed to allocate LOB buffer", "");
    return;
  }

  /* read the LOB in chunks using Rust's safe append */
  do {
    db2Debug2("  reading %d byte chunk of data", (int)sizeof(buf));
    rc = SQLGetData(session->stmtp->hsql, cidx, SQL_C_CHAR, buf, sizeof(buf), &ind);
    rc = db2CheckErr(rc, session->stmtp->hsql, session->stmtp->type, __LINE__, __FILE__);

    if (rc == SQL_ERROR) {
      db2_blob_free(blob);
      db2Error_d(FDW_UNABLE_TO_CREATE_EXECUTION, "error fetching result: SQLGetData failed to read LOB chunk", db2Message);
      return;
    }

    if (rc != 100) {
      int extend = 0;
      switch(ind) {
        case SQL_NULL_DATA:
          db2Debug3("  data length is null (SQL_NULL_DATA)");
          extend = 0;
          break;
        case SQL_NO_TOTAL:
          db2Debug3("  undefined data length (SQL_NO_TOTAL)");
          extend = LOB_CHUNK_SIZE;
          break;
        default:
          db2Debug3("  bytes still remaining: %ld", (long)ind);
          extend = (ind < LOB_CHUNK_SIZE) ? (int)ind : LOB_CHUNK_SIZE;
          break;
      }

      /* Append using Rust's bounds-checked function */
      if (extend > 0) {
        int append_rc = db2_blob_append(blob, buf, extend);
        if (append_rc != 0) {
          db2_blob_free(blob);
          db2Error_d(FDW_UNABLE_TO_CREATE_EXECUTION,
                     "error fetching result: LOB append failed: %s",
                     db2_core_last_error());
          return;
        }
      }
    }
  } while (rc == SQL_SUCCESS_WITH_INFO);

  /* Get the final data from Rust BLOB */
  blob_data = db2_blob_data(blob, &blob_len);

  if (blob_data != NULL && blob_len > 0) {
    /* Allocate PostgreSQL memory and copy */
    *value = db2alloc("lob_value", blob_len + 1);
    if (*value == NULL) {
      db2_blob_free(blob);
      db2Error_d(FDW_OUT_OF_MEMORY, "error fetching result: failed to allocate LOB result", "");
      return;
    }
    memcpy(*value, blob_data, blob_len);
    (*value)[blob_len] = '\0';
    *value_len = (long)blob_len;

    if (db2_blob_is_truncated(blob)) {
      db2Debug2("  WARNING: LOB was truncated at %ld bytes", (long)blob_len);
    }
  } else {
    *value = NULL;
    *value_len = 0;
  }

  db2Debug2("  value_len: %ld", *value_len);
  db2_blob_free(blob);
  db2Debug1("< db2GetLob");
}

#else
/*
 * Original version with improved bounds checking
 */
void db2GetLob (DB2Session* session, DB2Column* column, int cidx, char** value, long* value_len, unsigned long trunc) {
  SQLRETURN      rc  = SQL_SUCCESS;
  SQLLEN         ind = 0;
  SQLCHAR        buf[LOB_CHUNK_SIZE+1];
  int            extend = 0;
  size_t         max_lob_size = 1024 * 1024 * 1024; /* 1GB limit */

  db2Debug1("> db2GetLob");
  db2Debug2("  column->colName: '%s'",column->colName);
  db2Debug2("  cidx           :  %d ",cidx);

  /* initialize result buffer length */
  *value_len = 0;
  *value = NULL;

  /* read the LOB in chunks */
  do {
    db2Debug2("  value_len: %ld",*value_len);
    db2Debug2("  reading %d byte chunk of data",(int)sizeof(buf));
    rc = SQLGetData(session->stmtp->hsql, cidx, SQL_C_CHAR, buf, sizeof(buf), &ind);
    rc = db2CheckErr(rc,session->stmtp->hsql, session->stmtp->type, __LINE__, __FILE__);
    if (rc == SQL_ERROR) {
      db2Error_d ( FDW_UNABLE_TO_CREATE_EXECUTION, "error fetching result: SQLGetData failed to read LOB chunk", db2Message);
    }
    if (rc != 100) {
      switch(ind) {
        case SQL_NULL_DATA:
          db2Debug3("  data length is null (SQL_NULL_DATA)");
          extend = 0;
        break;
        case SQL_NO_TOTAL:
          db2Debug3("  undefined data length (SQL_NO_TOTAL)");
          extend = LOB_CHUNK_SIZE;
        break;
        default:
          db2Debug3("  bytes still remaining: %ld", (long)ind);
          extend = (ind < LOB_CHUNK_SIZE) ? (int)ind : LOB_CHUNK_SIZE;
        break;
      }

      /* BOUNDS CHECK: Prevent buffer overflow */
      if (extend > 0) {
        size_t new_size = (size_t)*value_len + (size_t)extend;
        if (new_size > max_lob_size) {
          db2Debug2("  WARNING: LOB exceeds max size, truncating at %ld", *value_len);
          break;
        }

        db2Debug2("  value_len: %ld", *value_len);
        db2Debug2("  extend   : %d", extend);

        if (*value_len == 0) {
          *value = db2alloc ("lob_value", new_size + 1);
        } else {
          *value = db2realloc (*value, new_size + 1);
        }

        if (*value == NULL) {
          db2Error_d(FDW_OUT_OF_MEMORY, "error fetching result: failed to allocate LOB buffer", "");
          return;
        }

        db2Debug3("  memcpy(%p,%p,%d)", (void*)(*value + *value_len), (void*)buf, extend);
        memcpy(*value + *value_len, buf, extend);
        *value_len += extend;
      }
    }
  } while (rc == SQL_SUCCESS_WITH_INFO);

  /* string end for CLOBs */
  db2Debug2("  *value   : %p" , (void*)*value);
  db2Debug2("  value_len: %ld", *value_len);
  if (*value != NULL) {
    (*value)[*value_len] = '\0';
    db2Debug2("  strlen of lob: %ld", (long)strlen(*value));
  } else {
    db2Debug2("  strlen of lob: 0 since *value is NULL");
  }
  db2Debug1("< db2GetLob");
}
#endif /* USE_RUST_CORE */
