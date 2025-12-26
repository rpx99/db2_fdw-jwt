# DB2 FDW Rust Migration Plan

## Status: ✅ COMPLETED (December 2025)

The migration from C to Rust has been successfully completed. All FDW callbacks are implemented and the Rust version has 100% feature parity with the C implementation.

## Executive Summary

Dieses Dokument beschreibt den Plan zur Umschreibung des DB2 Foreign Data Wrapper (db2_fdw) von C nach Rust. Die Hauptmotivation ist die Eliminierung von Memory-Safety-Problemen (Segfaults), die durch die manuelle Speicherverwaltung in C entstehen.

## Analyse der aktuellen Codebase

### Projektstatistik
- **Sprache**: C
- **Dateien**: 75+ C-Dateien, 10 Header
- **Codezeilen**: ~15.000 LOC
- **Version**: 18.1.1
- **PostgreSQL-Kompatibilität**: 9.1 - 18.0+

### Kritische Segfault-Quellen identifiziert

1. **putenv() Memory Management** (db2AllocEnvHdl.c:99-139)
   - `putenv()` speichert Pointer, nicht String
   - Bei Fehler wird Speicher freigegeben → Dangling Pointer

2. **Doppelt verkettete Listen ohne Validierung** (db2FreeEnvHdl.c)
   - NULL-Pointer-Dereferenzierung möglich
   - Use-after-free bei Liste-Manipulation

3. **Buffer Overflow in LOB-Handling** (db2GetLob.c)
   - Keine Bounds-Checking bei memcpy
   - Unbegrenzte Speicher-Reallokation

4. **Mixed Memory Management**
   - PostgreSQL palloc/pfree gemischt mit C malloc/free
   - Memory Leaks bei PostgreSQL Exceptions (longjmp)

5. **Transaktions-Callbacks** (db2Callbacks.c)
   - Use-after-free wenn Memory Context resettet wird

---

## Rust-Architektur Design

### Projektstruktur

```
db2_fdw_rs/
├── Cargo.toml                    # Workspace-Definition
├── crates/
│   ├── db2_fdw/                  # Haupt-FDW Crate
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs            # FDW Handler & Validator
│   │       ├── options.rs        # Option-Parsing
│   │       ├── scan.rs           # Foreign Scan Implementierung
│   │       ├── modify.rs         # DML Operationen
│   │       ├── explain.rs        # EXPLAIN Support
│   │       └── import.rs         # IMPORT FOREIGN SCHEMA
│   │
│   ├── db2_connection/           # Connection Management
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── pool.rs           # Connection Pooling
│   │       ├── session.rs        # Session Management
│   │       └── auth.rs           # JWT & Password Auth
│   │
│   ├── db2_odbc/                 # Safe ODBC Wrapper
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── environment.rs    # ODBC Environment
│   │       ├── connection.rs     # ODBC Connection
│   │       ├── statement.rs      # ODBC Statement
│   │       ├── types.rs          # SQL Type Mapping
│   │       └── error.rs          # Error Handling
│   │
│   ├── db2_query/                # Query Processing
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── deparse.rs        # SQL Generation
│   │       ├── pushdown.rs       # WHERE Pushdown
│   │       └── convert.rs        # Type Conversion
│   │
│   └── pg_fdw_bindings/          # PostgreSQL FFI
│       ├── Cargo.toml
│       ├── build.rs              # bindgen
│       └── src/
│           ├── lib.rs
│           ├── fdw_api.rs        # FDW Callbacks
│           ├── datum.rs          # Datum Handling
│           └── memory.rs         # Memory Context
│
├── build.rs                      # Global Build Script
├── Makefile                      # PostgreSQL PGXS Integration
└── sql/
    └── db2_fdw--18.1.1.sql       # SQL Extension Definition
```

### Technologie-Stack

| Komponente | Rust Crate | Begründung |
|------------|------------|------------|
| PostgreSQL Bindings | `pgrx` (v0.12+) | De-facto Standard für PostgreSQL Extensions in Rust |
| ODBC | `odbc-api` | Safe, moderne ODBC-API |
| Connection Pool | `dashmap` + `parking_lot` | Lock-free concurrent HashMap |
| Error Handling | `thiserror` | Derive-basierte Error Types |
| Logging | `tracing` | Structured Logging |
| FFI Safety | `cxx` | Safe C++ Interop (optional) |

---

## Migrations-Phasen

### Phase 1: Foundation (Woche 1-2)

#### 1.1 Projektsetup
```toml
# Cargo.toml
[workspace]
members = ["crates/*"]
resolver = "2"

[workspace.dependencies]
pgrx = "0.12"
odbc-api = "8.0"
thiserror = "2.0"
tracing = "0.1"
```

#### 1.2 PostgreSQL FDW Bindings
```rust
// crates/pg_fdw_bindings/src/lib.rs
use pgrx::prelude::*;

pg_module_magic!();

#[pg_extern]
fn db2_fdw_handler() -> FdwHandler {
    FdwHandler {
        get_foreign_rel_size: Some(get_foreign_rel_size),
        get_foreign_paths: Some(get_foreign_paths),
        get_foreign_plan: Some(get_foreign_plan),
        begin_foreign_scan: Some(begin_foreign_scan),
        iterate_foreign_scan: Some(iterate_foreign_scan),
        // ... weitere Callbacks
    }
}
```

#### 1.3 Safe ODBC Wrapper
```rust
// crates/db2_odbc/src/lib.rs
use odbc_api::{Environment, Connection, Cursor};

pub struct Db2Environment {
    inner: Environment,
}

pub struct Db2Connection {
    inner: Connection<'static>,
    server_name: String,
}

impl Db2Connection {
    pub fn new_with_password(
        env: &Db2Environment,
        server: &str,
        user: &str,
        password: &str,
    ) -> Result<Self, Db2Error> {
        // Safe ODBC connection
    }

    pub fn new_with_jwt(
        env: &Db2Environment,
        server: &str,
        jwt_token: &str,
    ) -> Result<Self, Db2Error> {
        // JWT authentication
    }
}
```

### Phase 2: Connection Management (Woche 3)

#### 2.1 Thread-Safe Connection Pool
```rust
// crates/db2_connection/src/pool.rs
use dashmap::DashMap;
use std::sync::Arc;

pub struct ConnectionPool {
    connections: DashMap<ConnectionKey, Arc<Db2Connection>>,
    environments: DashMap<String, Arc<Db2Environment>>,
}

#[derive(Hash, Eq, PartialEq)]
struct ConnectionKey {
    server: String,
    user: String,
    // Kein Passwort im Key aus Sicherheitsgründen
}

impl ConnectionPool {
    pub fn get_or_create(
        &self,
        options: &FdwOptions,
    ) -> Result<Arc<Db2Connection>, Db2Error> {
        // Thread-safe connection caching
        // Eliminiert C's doppelt verkettete Listen
    }
}
```

#### 2.2 Session Management
```rust
// crates/db2_connection/src/session.rs
pub struct Db2Session {
    connection: Arc<Db2Connection>,
    statement: Option<PreparedStatement>,
    xact_level: u32,
}

impl Db2Session {
    pub fn prepare_query(&mut self, sql: &str) -> Result<(), Db2Error> {
        // RAII-basiertes Statement Management
        // Automatisches Cleanup bei Drop
    }
}

impl Drop for Db2Session {
    fn drop(&mut self) {
        // Garantiertes Cleanup - keine Memory Leaks!
    }
}
```

### Phase 3: Query Processing (Woche 4-5)

#### 3.1 SQL Deparsing
```rust
// crates/db2_query/src/deparse.rs
use pgrx::pg_sys::*;

pub struct QueryDeparser {
    params: Vec<ParamInfo>,
    column_map: HashMap<Oid, ColumnInfo>,
}

impl QueryDeparser {
    pub fn deparse_expr(&mut self, node: *mut Node) -> Result<String, DeparseError> {
        // Safe pattern matching statt C switch
        match unsafe { (*node).type_ } {
            NodeTag::T_Const => self.deparse_const(node as *mut Const),
            NodeTag::T_Var => self.deparse_var(node as *mut Var),
            NodeTag::T_OpExpr => self.deparse_op_expr(node as *mut OpExpr),
            _ => Err(DeparseError::UnsupportedNode),
        }
    }
}
```

#### 3.2 Type Conversion
```rust
// crates/db2_query/src/convert.rs
pub enum Db2Value {
    Null,
    Char(String),
    VarChar(String),
    Integer(i32),
    BigInt(i64),
    Decimal(rust_decimal::Decimal),
    Float(f32),
    Double(f64),
    Date(chrono::NaiveDate),
    Timestamp(chrono::NaiveDateTime),
    Blob(Vec<u8>),
    Clob(String),
}

impl Db2Value {
    pub fn to_datum(&self, typoid: Oid) -> Result<Datum, ConvertError> {
        // Type-safe Konvertierung mit Result
        // Keine Buffer Overflows möglich!
    }
}
```

### Phase 4: Scan Operations (Woche 6)

#### 4.1 Foreign Scan
```rust
// crates/db2_fdw/src/scan.rs
pub struct ForeignScan {
    session: Db2Session,
    query: String,
    columns: Vec<ColumnDef>,
    current_row: Option<Vec<Db2Value>>,
}

impl ForeignScan {
    pub fn begin(state: &FdwState) -> Result<Self, ScanError> {
        let session = CONNECTION_POOL.get_or_create(&state.options)?;
        let query = state.deparser.build_select_query()?;
        // ...
    }

    pub fn iterate(&mut self) -> Option<HeapTuple> {
        // Iterator-Pattern statt manueller Cursor-Management
        self.session.fetch_next().map(|row| {
            self.row_to_tuple(&row)
        })
    }
}

impl Drop for ForeignScan {
    fn drop(&mut self) {
        // RAII: Statement automatisch geschlossen
    }
}
```

### Phase 5: DML Operations (Woche 7-8)

#### 5.1 Insert/Update/Delete
```rust
// crates/db2_fdw/src/modify.rs
pub struct ForeignModify {
    session: Db2Session,
    operation: ModifyOperation,
    target_columns: Vec<ColumnDef>,
    key_columns: Vec<ColumnDef>,
}

pub enum ModifyOperation {
    Insert { batch_size: usize },
    Update,
    Delete,
}

impl ForeignModify {
    pub fn exec_insert(&mut self, slot: &TupleTableSlot) -> Result<(), ModifyError> {
        let values = self.extract_values(slot)?;
        self.session.execute_insert(&values)?;
        Ok(())
    }

    pub fn exec_batch_insert(
        &mut self,
        slots: &[TupleTableSlot]
    ) -> Result<usize, ModifyError> {
        // Batch-Insert mit prepared statements
        // Automatisches Batching
    }
}
```

### Phase 6: Transaction Management (Woche 9)

#### 6.1 Safe Callbacks
```rust
// crates/db2_fdw/src/transaction.rs
use pgrx::hooks::*;

static ACTIVE_TRANSACTIONS: Lazy<DashMap<SubTransactionId, SavepointState>> =
    Lazy::new(DashMap::new);

pub fn register_transaction_callbacks() {
    // PostgreSQL Transaction Hooks
    register_xact_callback(|event| {
        match event {
            XactEvent::PreCommit => commit_all_connections(),
            XactEvent::Abort => rollback_all_connections(),
            _ => {}
        }
    });

    register_subxact_callback(|event, mySubid, parentSubid| {
        match event {
            SubXactEvent::Start => create_savepoint(mySubid),
            SubXactEvent::Abort => rollback_to_savepoint(mySubid),
            SubXactEvent::Commit => release_savepoint(mySubid),
            _ => {}
        }
    });
}
```

---

## Error Handling Strategie

### Custom Error Types
```rust
// crates/db2_odbc/src/error.rs
use thiserror::Error;

#[derive(Error, Debug)]
pub enum Db2Error {
    #[error("ODBC error: {sqlstate} - {message}")]
    Odbc {
        sqlstate: String,
        native_error: i32,
        message: String,
    },

    #[error("Connection failed: {0}")]
    ConnectionFailed(String),

    #[error("Authentication failed: invalid {method}")]
    AuthenticationFailed { method: &'static str },

    #[error("Type conversion error: cannot convert {from} to {to}")]
    TypeConversion { from: String, to: String },

    #[error("Query timeout after {seconds}s")]
    Timeout { seconds: u64 },
}

impl From<Db2Error> for pgrx::PgError {
    fn from(e: Db2Error) -> Self {
        // Konvertierung zu PostgreSQL ERROR
        pgrx::error!("{}", e)
    }
}
```

### Panic Safety
```rust
// Alle FDW-Callbacks werden mit catch_unwind geschützt
pub fn safe_fdw_callback<F, R>(f: F) -> R
where
    F: FnOnce() -> Result<R, Db2Error> + std::panic::UnwindSafe,
{
    match std::panic::catch_unwind(f) {
        Ok(Ok(result)) => result,
        Ok(Err(e)) => {
            pgrx::error!("DB2 FDW error: {}", e);
        }
        Err(_) => {
            pgrx::error!("DB2 FDW internal panic");
        }
    }
}
```

---

## Build & Deployment

### Makefile Integration
```makefile
# Makefile
EXTENSION = db2_fdw
EXTVERSION = 19.0.0

# Rust Build
.PHONY: rust-build
rust-build:
	cargo build --release
	cp target/release/libdb2_fdw.so $(DESTDIR)$(pkglibdir)/db2_fdw.so

# PostgreSQL PGXS
PGXS := $(shell $(PG_CONFIG) --pgxs)
include $(PGXS)

install: rust-build
	$(INSTALL_DATA) sql/$(EXTENSION)--$(EXTVERSION).sql $(DESTDIR)$(datadir)/extension/
	$(INSTALL_DATA) $(EXTENSION).control $(DESTDIR)$(datadir)/extension/
```

### Cargo.toml für Shared Library
```toml
[lib]
crate-type = ["cdylib"]

[profile.release]
lto = true
opt-level = 3
panic = "abort"  # Wichtig für FFI
```

---

## Testing Strategie

### Unit Tests
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_type_conversion() {
        let db2_int = Db2Value::Integer(42);
        let datum = db2_int.to_datum(INT4OID).unwrap();
        assert_eq!(unsafe { datum.value() }, 42);
    }

    #[test]
    fn test_deparse_simple_where() {
        let deparser = QueryDeparser::new();
        let sql = deparser.deparse_where("id = 1").unwrap();
        assert_eq!(sql, "\"ID\" = 1");
    }
}
```

### Integration Tests
```rust
#[cfg(test)]
mod integration {
    use pgrx_tests::*;

    #[pg_test]
    fn test_foreign_scan() {
        Spi::run("CREATE SERVER db2_test FOREIGN DATA WRAPPER db2_fdw ...");
        Spi::run("CREATE FOREIGN TABLE test_table ...");

        let result = Spi::get_one::<i32>("SELECT count(*) FROM test_table");
        assert!(result.is_some());
    }
}
```

---

## Migration Checkliste

### Phase 1: Foundation ✅
- [x] Cargo Workspace Setup
- [x] pgrx Integration
- [x] ODBC-API Bindings
- [x] Basic Error Types

### Phase 2: Connection ✅
- [x] Environment Management
- [x] Connection mit Password
- [x] Connection mit JWT
- [x] Connection Pool (thread_local! + HashMap)
- [x] Session Management

### Phase 3: Query ✅
- [x] SQL Deparser
- [x] WHERE Pushdown
- [x] Type Mapping DB2 → Rust
- [x] Type Mapping Rust → PostgreSQL

### Phase 4: Scan ✅
- [x] GetForeignRelSize
- [x] GetForeignPaths
- [x] GetForeignPlan
- [x] BeginForeignScan
- [x] IterateForeignScan
- [x] ReScanForeignScan
- [x] EndForeignScan

### Phase 5: DML ✅
- [x] BeginForeignModify
- [x] ExecForeignInsert
- [x] ExecForeignUpdate
- [x] ExecForeignDelete
- [x] ExecForeignBatchInsert
- [x] ExecForeignTruncate

### Phase 6: Transactions ✅
- [x] Transaction Callbacks
- [x] Savepoint Management
- [x] Subtransaction Support

### Phase 7: Extras ✅
- [x] EXPLAIN Support
- [x] ANALYZE Support
- [x] IMPORT FOREIGN SCHEMA
- [x] JOIN Pushdown (INNER only, matching C)

### Phase 8: Testing & Docs ✅
- [x] Unit Tests
- [x] Integration Tests (via pgrx)
- [x] README.md (User Guide)
- [x] ARCHITECTURE.md (Technical Docs)
- [x] Migration Plan (this document)

---

## Risiken & Mitigationen

| Risiko | Wahrscheinlichkeit | Impact | Mitigation |
|--------|-------------------|--------|------------|
| pgrx Inkompatibilität mit älteren PG | Mittel | Hoch | PG 12+ unterstützen, Legacy-Support dokumentieren |
| ODBC-API Performance | Niedrig | Mittel | Benchmarks, optional unsafe FFI |
| DB2-spezifische ODBC-Features | Mittel | Mittel | Fallback auf raw FFI |
| Komplexe Query Pushdown | Hoch | Niedrig | Inkrementelle Implementierung |

---

## Vorteile der Rust-Migration

1. **Memory Safety**: Keine Segfaults durch Ownership-System
2. **Thread Safety**: Safe Concurrency durch Send/Sync
3. **Error Handling**: Explizite Result-Types statt NULL-Checks
4. **RAII**: Automatisches Resource Cleanup
5. **Modern Tooling**: Cargo, Clippy, Rustfmt
6. **Performance**: Zero-Cost Abstractions
7. **Maintainability**: Bessere Code-Struktur durch Module

---

## Nächste Schritte

1. Projekt-Struktur erstellen
2. pgrx + odbc-api Abhängigkeiten einrichten
3. Minimalen FDW Handler implementieren
4. Connection Management portieren
5. Inkrementell weitere Features hinzufügen

---

*Erstellt: 2025-12-25*
*Autor: Claude Code Assistant*
