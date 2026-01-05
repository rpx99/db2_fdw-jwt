#include <stdio.h>
#include <string.h>
#include <sqlcli1.h>
#include <postgres_ext.h>
#include "db2_fdw.h"

/* Rust FFI core library for safe operations */
#ifdef USE_RUST_CORE
#include "db2_fdw_core.h"
#endif

/** global variables */
DB2EnvEntry*        rootenvEntry    = NULL;/* Linked list of handles for cached DB2 connections.            */
int                 sql_initialized = 0;   /* set to "1" as soon as SQLAllocHandle(SQL_HANDLE_ENV is called */

/** external variables */
extern char         db2Message[ERRBUFSIZE];/* contains DB2 error messages, set by db2CheckErr()             */

/** external prototypes */
extern void      db2SetHandlers       (void);
extern void      db2Debug1            (const char* message, ...);
extern void      db2Debug2            (const char* message, ...);
extern void      db2Debug3            (const char* message, ...);
extern void      db2Error_d           (db2error sqlstate, const char* message, const char* detail, ...);
extern SQLRETURN db2CheckErr          (SQLRETURN status, SQLHANDLE handle, SQLSMALLINT handleType, int line, char* file);
extern char*     db2strdup            (const char* p);
extern void      db2free              (void* p);

/** local prototypes */
DB2EnvEntry*     db2AllocEnvHdl       (const char* nls_lang);
void             setDB2Environment    (char* nls_lang);
DB2EnvEntry*     insertenvEntry       (DB2EnvEntry* start, const char* nlslang, SQLHENV henv);

/** db2AllocEnvHdl
 * 
 */
DB2EnvEntry* db2AllocEnvHdl(const char* nls_lang){
  char*         nlscopy = NULL;
  DB2EnvEntry*  envp    = NULL;
  SQLHENV       henv    = SQL_NULL_HENV;
  SQLRETURN     rc      = 0;

  db2Debug1("> db2AllocEnvHdl");
  /* create persistent copy of "nls_lang" */
  if ((nlscopy = db2strdup (nls_lang)) == NULL)
    db2Error_d (FDW_OUT_OF_MEMORY, "error connecting to DB2:"," failed to allocate %d bytes of memory", strlen (nls_lang) + 1);

  /* set DB2 environment */
  setDB2Environment (nlscopy);

  /* create environment handle */
  rc = SQLAllocHandle(SQL_HANDLE_ENV, SQL_NULL_HANDLE, &henv);
  db2Debug3("  allocate env handle - rc: %d, henv: %d",rc, henv);
  rc = db2CheckErr(rc, henv, SQL_HANDLE_ENV, __LINE__, __FILE__);
  if (rc != SQL_SUCCESS) {
    db2free (nlscopy);
    db2Error_d (FDW_UNABLE_TO_ESTABLISH_CONNECTION, "error connecting to DB2: SQLAllocHandle failed to create environment handle", db2Message);
  }

  /* we can call db2Shutdown now */
  sql_initialized = 1;
  db2Debug3("  sql_initialized: %d",sql_initialized);

  rc = SQLSetEnvAttr(henv, SQL_ATTR_ODBC_VERSION, (SQLPOINTER)SQL_OV_ODBC3, 0);
  db2Debug3("  set env attributes odbcv3 - rc: %d, henv: %d",rc, henv);
  rc = db2CheckErr(rc, henv, SQL_HANDLE_ENV, __LINE__, __FILE__);
  if (rc != SQL_SUCCESS) {
    db2free (nlscopy);
    db2Error_d (FDW_UNABLE_TO_ESTABLISH_CONNECTION, "error connecting to DB2: SQLSetEnvAttr failed to set ODBC v3.0", db2Message);
  }

  /*
   * DB2 overwrites PostgreSQL's signal handlers, so we have to restore them.
   * DB2's SIGINT handler is ok (it cancels the query), but we must do something
   * reasonable for SIGTERM.
   */
  db2SetHandlers ();
  /* add handles to cache */
  envp = insertenvEntry(rootenvEntry, nlscopy, henv);
  if ( rootenvEntry == NULL) {
    rootenvEntry = envp;
  }

  db2Debug1("< db2AllocEnvHdl - returns: %x",envp);
  return envp;
}

/** setDB2Environment
 *   Set environment variables do that DB2 works as we want.
 *
 *   NLS_LANG sets the language and client encoding
 *   NLS_NCHAR is unset so that N* data types are converted to the
 *   character set specified in NLS_LANG.
 *
 *   The following variables are set to values that make DB2 convert
 *   numeric and date/time values to strings PostgreSQL can parse:
 *   NLS_DATE_FORMAT
 *   NLS_TIMESTAMP_FORMAT
 *   NLS_TIMESTAMP_TZ_FORMAT
 *   NLS_NUMERIC_CHARACTERS
 *   NLS_CALENDAR
 *
 *   FIXED: The original code had a use-after-free bug where putenv() was
 *   called with a malloc'd string that was later freed. putenv() stores
 *   the pointer, not a copy of the string!
 */

#ifdef USE_RUST_CORE
/*
 * Safe version using Rust FFI - no use-after-free possible
 */
void setDB2Environment (char* nls_lang) {
  db2Debug2("  > setDB2Environment (Rust safe version)");

  /* Use Rust's safe NLS handling - stores strings permanently */
  if (db2_env_set_nls_lang(nls_lang) != 0) {
    db2Error_d (FDW_UNABLE_TO_ESTABLISH_CONNECTION, "error connecting to DB2",
                "Environment variable NLS_LANG cannot be set: %s", db2_core_last_error());
  }

  /* Static strings are safe for putenv - they live forever */
  static char nls_date_language[]   = "NLS_DATE_LANGUAGE=AMERICAN";
  static char nls_date_format[]     = "NLS_DATE_FORMAT=YYYY-MM-DD HH24:MI:SS BC";
  static char nls_timestamp_format[] = "NLS_TIMESTAMP_FORMAT=YYYY-MM-DD HH24:MI:SS.FF9 BC";
  static char nls_timestamp_tz[]    = "NLS_TIMESTAMP_TZ_FORMAT=YYYY-MM-DD HH24:MI:SS.FF9TZH:TZM BC";
  static char nls_time_format[]     = "NLS_TIME_FORMAT=HH24:MI:SS.FF9 BC";
  static char nls_time_tz_format[]  = "NLS_TIME_TZ_FORMAT= HH24:MI:SS.FF9TZH:TZM BC";
  static char nls_numeric_chars[]   = "NLS_NUMERIC_CHARACTERS=.,";
  static char nls_calendar[]        = "NLS_CALENDAR=";
  static char nls_nchar[]           = "NLS_NCHAR=";

  if (putenv(nls_date_language) != 0)
    db2Error_d (FDW_UNABLE_TO_ESTABLISH_CONNECTION, "error connecting to DB2", "Environment variable NLS_DATE_LANGUAGE cannot be set.");
  if (putenv(nls_date_format) != 0)
    db2Error_d (FDW_UNABLE_TO_ESTABLISH_CONNECTION, "error connecting to DB2", "Environment variable NLS_DATE_FORMAT cannot be set.");
  if (putenv(nls_timestamp_format) != 0)
    db2Error_d (FDW_UNABLE_TO_ESTABLISH_CONNECTION, "error connecting to DB2", "Environment variable NLS_TIMESTAMP_FORMAT cannot be set.");
  if (putenv(nls_timestamp_tz) != 0)
    db2Error_d (FDW_UNABLE_TO_ESTABLISH_CONNECTION, "error connecting to DB2", "Environment variable NLS_TIMESTAMP_TZ_FORMAT cannot be set.");
  if (putenv(nls_time_format) != 0)
    db2Error_d (FDW_UNABLE_TO_ESTABLISH_CONNECTION, "error connecting to DB2", "Environment variable NLS_TIME_FORMAT cannot be set.");
  if (putenv(nls_time_tz_format) != 0)
    db2Error_d (FDW_UNABLE_TO_ESTABLISH_CONNECTION, "error connecting to DB2", "Environment variable NLS_TIME_TZ_FORMAT cannot be set.");
  if (putenv(nls_numeric_chars) != 0)
    db2Error_d (FDW_UNABLE_TO_ESTABLISH_CONNECTION, "error connecting to DB2", "Environment variable NLS_NUMERIC_CHARACTERS cannot be set.");
  if (putenv(nls_calendar) != 0)
    db2Error_d (FDW_UNABLE_TO_ESTABLISH_CONNECTION, "error connecting to DB2", "Environment variable NLS_CALENDAR cannot be set.");
  if (putenv(nls_nchar) != 0)
    db2Error_d (FDW_UNABLE_TO_ESTABLISH_CONNECTION, "error connecting to DB2", "Environment variable NLS_NCHAR cannot be set.");

  db2Debug2("  < setDB2Environment");
}

#else
/*
 * Original version - FIXED to use static buffers instead of malloc'd strings
 * that were incorrectly freed after putenv().
 *
 * NOTE: putenv() stores the POINTER, not a copy! The string must live forever.
 */
void setDB2Environment (char* nls_lang) {
  static char nls_lang_buf[256];

  db2Debug2("  > setDB2Environment (fixed version)");

  /* Copy to static buffer - lives forever, safe for putenv */
  snprintf(nls_lang_buf, sizeof(nls_lang_buf), "NLS_LANG=%s", nls_lang);

  if (putenv(nls_lang_buf) != 0) {
    db2Error_d (FDW_UNABLE_TO_ESTABLISH_CONNECTION, "error connecting to DB2", "Environment variable NLS_LANG cannot be set.");
  }

  /* Static strings are safe for putenv */
  static char nls_date_language[]   = "NLS_DATE_LANGUAGE=AMERICAN";
  static char nls_date_format[]     = "NLS_DATE_FORMAT=YYYY-MM-DD HH24:MI:SS BC";
  static char nls_timestamp_format[] = "NLS_TIMESTAMP_FORMAT=YYYY-MM-DD HH24:MI:SS.FF9 BC";
  static char nls_timestamp_tz[]    = "NLS_TIMESTAMP_TZ_FORMAT=YYYY-MM-DD HH24:MI:SS.FF9TZH:TZM BC";
  static char nls_time_format[]     = "NLS_TIME_FORMAT=HH24:MI:SS.FF9 BC";
  static char nls_time_tz_format[]  = "NLS_TIME_TZ_FORMAT= HH24:MI:SS.FF9TZH:TZM BC";
  static char nls_numeric_chars[]   = "NLS_NUMERIC_CHARACTERS=.,";
  static char nls_calendar[]        = "NLS_CALENDAR=";
  static char nls_nchar[]           = "NLS_NCHAR=";

  if (putenv(nls_date_language) != 0)
    db2Error_d (FDW_UNABLE_TO_ESTABLISH_CONNECTION, "error connecting to DB2", "Environment variable NLS_DATE_LANGUAGE cannot be set.");
  if (putenv(nls_date_format) != 0)
    db2Error_d (FDW_UNABLE_TO_ESTABLISH_CONNECTION, "error connecting to DB2", "Environment variable NLS_DATE_FORMAT cannot be set.");
  if (putenv(nls_timestamp_format) != 0)
    db2Error_d (FDW_UNABLE_TO_ESTABLISH_CONNECTION, "error connecting to DB2", "Environment variable NLS_TIMESTAMP_FORMAT cannot be set.");
  if (putenv(nls_timestamp_tz) != 0)
    db2Error_d (FDW_UNABLE_TO_ESTABLISH_CONNECTION, "error connecting to DB2", "Environment variable NLS_TIMESTAMP_TZ_FORMAT cannot be set.");
  if (putenv(nls_time_format) != 0)
    db2Error_d (FDW_UNABLE_TO_ESTABLISH_CONNECTION, "error connecting to DB2", "Environment variable NLS_TIME_FORMAT cannot be set.");
  if (putenv(nls_time_tz_format) != 0)
    db2Error_d (FDW_UNABLE_TO_ESTABLISH_CONNECTION, "error connecting to DB2", "Environment variable NLS_TIME_TZ_FORMAT cannot be set.");
  if (putenv(nls_numeric_chars) != 0)
    db2Error_d (FDW_UNABLE_TO_ESTABLISH_CONNECTION, "error connecting to DB2", "Environment variable NLS_NUMERIC_CHARACTERS cannot be set.");
  if (putenv(nls_calendar) != 0)
    db2Error_d (FDW_UNABLE_TO_ESTABLISH_CONNECTION, "error connecting to DB2", "Environment variable NLS_CALENDAR cannot be set.");
  if (putenv(nls_nchar) != 0)
    db2Error_d (FDW_UNABLE_TO_ESTABLISH_CONNECTION, "error connecting to DB2", "Environment variable NLS_NCHAR cannot be set.");

  db2Debug2("  < setDB2Environment");
}
#endif /* USE_RUST_CORE */

/** insertenvEntry
 * 
 */
DB2EnvEntry* insertenvEntry(DB2EnvEntry* start, const char* nlslang, SQLHENV henv) { 
  DB2EnvEntry* step = NULL;
  DB2EnvEntry* new  = NULL;
  db2Debug2("  > insertenvEntry(start: %x, nlslang: '%s', henv: %d)",start, nlslang, henv);

  /* allocate a  new DB2EnvEntry and initialize it*/
  new = malloc(sizeof(DB2EnvEntry));
  if (new  == NULL) {
    db2Error_d (FDW_OUT_OF_MEMORY, "error connecting to DB2:"," failed to allocate %d bytes of memory", sizeof (DB2EnvEntry));
  }
  new->nls_lang = strdup(nlslang);  // important to use strdup since env will survive multiple PG scopes, and so needs nls_lang
  new->henv     = henv;
  new->connlist = NULL;
  new->left     = NULL;
  new->right    = NULL;

  // adding the first element to the list is done outside of this fuunction
  if (start != NULL) {
    // scroll forward to the last element of the list
    for (step = start; step->right != NULL; step = step->right){ }
    step->right = new;
    new->left  = step;
    new->right = NULL;
  }
  db2Debug3("    new: %x ->henv: %d, ->connlist: %x, ->left: %x, ->right: %x, ->nls_lang: '%s'",new,new->henv,new->connlist,new->left,new->right,new->nls_lang);
  db2Debug2("  < insertenvEntry - returns: %x", new);
  return new; 
} 
