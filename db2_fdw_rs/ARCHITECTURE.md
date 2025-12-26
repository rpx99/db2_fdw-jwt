# DB2 FDW Rust Architecture

## Overview

This document describes the architecture of the Rust implementation of the DB2 Foreign Data Wrapper for PostgreSQL.

```
┌─────────────────────────────────────────────────────────────────────────┐
│                         PostgreSQL Backend                               │
├─────────────────────────────────────────────────────────────────────────┤
│  FDW API Callbacks                                                       │
│  ┌─────────────┐ ┌─────────────┐ ┌─────────────┐ ┌─────────────────────┐│
│  │ GetForeign  │ │ BeginForeign│ │ ExecForeign │ │ Transaction         ││
│  │ RelSize/    │ │ Scan/Modify │ │ Insert/     │ │ Callbacks           ││
│  │ Paths/Plan  │ │             │ │ Update/Del  │ │ (Commit/Abort)      ││
│  └──────┬──────┘ └──────┬──────┘ └──────┬──────┘ └──────────┬──────────┘│
└─────────┼───────────────┼───────────────┼───────────────────┼───────────┘
          │               │               │                   │
          ▼               ▼               ▼                   ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                        db2_fdw (Main Crate)                              │
│  ┌────────────────────────────────────────────────────────────────────┐ │
│  │ lib.rs - FDW Handler & Validator                                    │ │
│  │  - db2_fdw_handler() → FdwRoutine with all callbacks                │ │
│  │  - db2_fdw_validator() → Option validation                          │ │
│  │  - db2_close_connections() → Cleanup utility                        │ │
│  │  - db2_diag() → Diagnostic info                                     │ │
│  └────────────────────────────────────────────────────────────────────┘ │
│  ┌─────────────┐ ┌─────────────┐ ┌─────────────┐ ┌─────────────────────┐│
│  │ scan.rs     │ │ modify.rs   │ │ explain.rs  │ │ import.rs           ││
│  │ - RelSize   │ │ - Insert    │ │ - EXPLAIN   │ │ - IMPORT FOREIGN    ││
│  │ - Paths     │ │ - Update    │ │   support   │ │   SCHEMA            ││
│  │ - Plan      │ │ - Delete    │ │             │ │                     ││
│  │ - Begin     │ │ - Truncate  │ └─────────────┘ └─────────────────────┘│
│  │ - Iterate   │ │ - Batch     │                                        │
│  │ - ReScan    │ │ - Updatable │ ┌─────────────┐ ┌─────────────────────┐│
│  │ - End       │ └─────────────┘ │ options.rs  │ │ state.rs            ││
│  │ - Analyze   │                 │ - Parse     │ │ - FdwPlanState      ││
│  │ - JoinPaths │                 │ - Validate  │ │ - FdwScanState      ││
│  └─────────────┘                 └─────────────┘ │ - FdwModifyState    ││
│  ┌─────────────┐ ┌──────────────────────────────┐└─────────────────────┘│
│  │transaction. │ │ deparsing.rs                 │ ┌─────────────────────┐│
│  │rs           │ │ - classify_conditions()      │ │ query.rs            ││
│  │ - Xact      │ │ - deparse_expr()             │ │ - QueryBuilder      ││
│  │   callbacks │ │ - Predicate Pushdown         │ │ - build_select()    ││
│  │ - Savepoint │ │                              │ │ - build_insert()    ││
│  └─────────────┘ └──────────────────────────────┘ └─────────────────────┘│
└────────────────────────────────┬────────────────────────────────────────┘
                                 │
                                 ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                     db2_connection (Crate)                               │
│  ┌────────────────────────────────────────────────────────────────────┐ │
│  │ Connection Pool (thread_local! + RefCell<HashMap>)                  │ │
│  │  - Per-backend connection caching                                   │ │
│  │  - PostgreSQL is multi-process, so thread_local is safe            │ │
│  └────────────────────────────────────────────────────────────────────┘ │
│  ┌─────────────────────────────┐ ┌────────────────────────────────────┐ │
│  │ Db2Session                  │ │ Authentication                     │ │
│  │  - prepare_and_execute()    │ │  - Password auth                   │ │
│  │  - fetch_next()             │ │  - JWT token auth (DB2 11.5.4+)    │ │
│  │  - close_cursor()           │ │                                    │ │
│  └─────────────────────────────┘ └────────────────────────────────────┘ │
└────────────────────────────────┬────────────────────────────────────────┘
                                 │
                                 ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                        db2_odbc (Crate)                                  │
│  ┌────────────────────────────────────────────────────────────────────┐ │
│  │ Safe ODBC Wrapper over odbc-api crate                               │ │
│  │  - Db2Environment (static lifetime via Box::leak)                   │ │
│  │  - Db2Connection                                                    │ │
│  │  - PreparedStatement                                                │ │
│  └────────────────────────────────────────────────────────────────────┘ │
│  ┌─────────────────────────────┐ ┌────────────────────────────────────┐ │
│  │ Type Mapping                │ │ Db2Value Enum                      │ │
│  │  - DB2 SQL Types → Rust     │ │  - Null, Text, SmallInt, Integer   │ │
│  │  - Rust → PostgreSQL Datum  │ │  - BigInt, Real, Double, Decimal   │ │
│  │                             │ │  - Date, Time, Timestamp, Binary   │ │
│  └─────────────────────────────┘ └────────────────────────────────────┘ │
└────────────────────────────────┬────────────────────────────────────────┘
                                 │
                                 ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                        db2_query (Crate)                                 │
│  ┌─────────────────────────────┐ ┌────────────────────────────────────┐ │
│  │ Deparser                    │ │ PushdownChecker                    │ │
│  │  - SQL statement generation │ │  - Check if expr can be pushed     │ │
│  │  - Literal escaping         │ │  - Supported operators list        │ │
│  │  - Identifier quoting       │ │  - DB2-specific function support   │ │
│  └─────────────────────────────┘ └────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────────────────┘
```

## Crate Responsibilities

### db2_fdw (Main Extension)

The PostgreSQL extension crate that implements all FDW callbacks:

| Module | Responsibility |
|--------|---------------|
| `lib.rs` | Extension entry point, FDW handler, utility functions |
| `scan.rs` | SELECT operations, ANALYZE, JOIN pushdown |
| `modify.rs` | INSERT, UPDATE, DELETE, TRUNCATE operations |
| `explain.rs` | EXPLAIN output for scans and modifications |
| `import.rs` | IMPORT FOREIGN SCHEMA implementation |
| `transaction.rs` | Transaction and subtransaction callbacks |
| `options.rs` | Option parsing and validation |
| `state.rs` | State structures for plan/scan/modify phases |
| `query.rs` | QueryBuilder helper for SQL generation |
| `deparsing.rs` | PostgreSQL expression → DB2 SQL conversion |

### db2_connection

Connection lifecycle management:

- **Session Management**: Wraps ODBC connection with statement lifecycle
- **Connection Caching**: Per-backend cache using `thread_local!`
- **Authentication**: Password and JWT token support

### db2_odbc

Safe ODBC wrapper:

- **Environment**: Single static environment per backend (`Box::leak`)
- **Connection**: Owned connection with RAII cleanup
- **Statement**: Prepared statements with cursor management
- **Types**: `Db2Value` enum for type-safe data transfer

### db2_query

Query processing utilities:

- **Deparser**: Generate DB2-compatible SQL from internal representations
- **PushdownChecker**: Determine which expressions can be executed on DB2
- **Type Conversion**: Map between PostgreSQL, Rust, and DB2 types

## Data Flow

### SELECT Query Flow

```
1. GetForeignRelSize
   └── Estimate row count (default: 1000)

2. GetForeignPaths
   └── Create ForeignPath with cost estimates

3. GetForeignPlan
   ├── classify_conditions() → Split WHERE into remote/local
   ├── QueryBuilder.build_select() → Generate SQL with WHERE
   └── make_foreignscan() → Create ForeignScan plan node

4. BeginForeignScan
   ├── Parse options from relation
   ├── Build SQL query
   ├── FdwScanState.init_session() → Get/create DB2 connection
   └── session.prepare_and_execute() → Execute query on DB2

5. IterateForeignScan (called repeatedly)
   ├── session.fetch_next() → Get next row
   ├── fill_tuple_slot() → Convert Db2Value → PostgreSQL Datum
   └── Return slot or NULL if done

6. EndForeignScan
   └── Drop FdwScanState (RAII closes cursor/session)
```

### INSERT Flow

```
1. PlanForeignModify
   └── Return NULL (SQL built in BeginForeignModify)

2. BeginForeignModify
   ├── Get operation type (INSERT/UPDATE/DELETE)
   ├── Extract column names from tuple descriptor
   ├── QueryBuilder.build_insert() → Generate INSERT SQL
   └── FdwModifyState.init_session()

3. ExecForeignInsert (per row)
   ├── extract_slot_values() → TupleSlot → Vec<Db2Value>
   ├── add_to_batch() → Collect for batch insert
   └── flush_batch_with_session() → Execute when batch full

4. EndForeignModify
   ├── Flush remaining batch
   └── Drop FdwModifyState (RAII cleanup)
```

## Memory Safety Guarantees

### C Implementation Problems (Eliminated)

| Issue | C Code Location | Rust Solution |
|-------|-----------------|---------------|
| Dangling pointers from putenv() | db2AllocEnvHdl.c | Not needed - odbc-api handles env |
| Use-after-free in linked lists | db2FreeEnvHdl.c | RAII with Drop trait |
| Buffer overflow in LOB | db2GetLob.c | Vec<u8> with bounds checking |
| Mixed palloc/malloc | Various | All heap allocations via Rust |
| Memory leaks on longjmp | Callbacks | pg_guard macro + catch_unwind |

### Rust Safety Mechanisms

1. **Ownership**: Each resource has exactly one owner
2. **RAII**: Resources cleaned up when owner goes out of scope
3. **Drop trait**: Guaranteed cleanup even on errors
4. **Result<T, E>**: Explicit error handling, no NULL checks
5. **pg_guard macro**: Catches Rust panics before they cross FFI boundary

## Threading Model

PostgreSQL uses a **multi-process** architecture:
- Each connection gets its own backend process
- Each backend is **single-threaded**
- No shared state between backends (except shared memory)

Therefore:
- We use `thread_local!` instead of `Arc<Mutex<>>`
- `RefCell` is safe (no concurrent access)
- No synchronization overhead

```rust
thread_local! {
    static ENVIRONMENT: RefCell<Option<&'static Environment>> = RefCell::new(None);
    static CONNECTION_CACHE: RefCell<HashMap<ConnectionKey, Db2Session>> = RefCell::new(HashMap::new());
}
```

## Feature Comparison with C

| Feature | C Implementation | Rust Implementation |
|---------|------------------|---------------------|
| SELECT | ✅ | ✅ |
| INSERT | ✅ | ✅ + Batch support |
| UPDATE | ✅ | ✅ |
| DELETE | ✅ | ✅ |
| TRUNCATE | ✅ | ✅ |
| EXPLAIN | ✅ | ✅ |
| ANALYZE | ✅ (SAMPLE BLOCK) | ✅ (TABLESAMPLE BERNOULLI) |
| IMPORT FOREIGN SCHEMA | ✅ | ✅ + case folding options |
| Predicate Pushdown | ✅ | ✅ |
| Join Pushdown | ✅ (INNER only) | ✅ (INNER only) |
| JWT Authentication | ✅ | ✅ |
| Connection Pooling | ✅ (linked list) | ✅ (HashMap) |
| Transaction Callbacks | ✅ | ✅ |
| Memory Safety | ❌ (segfaults) | ✅ (guaranteed) |

## Error Handling

```rust
// All errors flow through thiserror-derived types
#[derive(Error, Debug)]
pub enum Db2Error {
    #[error("ODBC error: {0}")]
    Odbc(#[from] odbc_api::Error),

    #[error("Connection failed: {0}")]
    Connection(String),

    #[error("Type conversion: {0}")]
    TypeConversion(String),
}

// Conversion to PostgreSQL errors
impl From<Db2Error> for pgrx::spi::SpiError {
    fn from(e: Db2Error) -> Self {
        pgrx::error!("{}", e)
    }
}
```

## Building

```bash
# Development build
cargo build

# Release build
cargo build --release

# Run tests (non-PostgreSQL)
cargo test --all

# Run PostgreSQL tests (requires pgrx setup)
cargo pgrx test pg16

# Install extension
cargo pgrx install

# Generate documentation
cargo doc --no-deps --open
```

## Configuration

### Server Options

```sql
CREATE SERVER db2_server
    FOREIGN DATA WRAPPER db2_fdw
    OPTIONS (
        dbserver 'DSN_NAME',    -- ODBC DSN or connection string
        nls_lang 'GERMAN',      -- NLS language setting
        prefetch '200',         -- Rows to prefetch
        batch_size '100'        -- Batch insert size
    );
```

### Table Options

```sql
CREATE FOREIGN TABLE employees (
    id INTEGER OPTIONS (key 'true'),
    name VARCHAR(100)
)
SERVER db2_server
OPTIONS (
    schema 'HR',
    table 'EMPLOYEES',
    readonly 'false',
    sample_percent '10'
);
```

### User Mapping Options

```sql
-- Password authentication
CREATE USER MAPPING FOR current_user
    SERVER db2_server
    OPTIONS (user 'db2user', password 'secret');

-- JWT authentication (DB2 11.5.4+)
CREATE USER MAPPING FOR current_user
    SERVER db2_server
    OPTIONS (jwt_token 'eyJhbGciOiJSUzI1NiIs...');
```

## License

PostgreSQL License (same as original C implementation)
