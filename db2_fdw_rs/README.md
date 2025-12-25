# DB2 Foreign Data Wrapper for PostgreSQL (Rust Implementation)

A memory-safe reimplementation of the DB2 FDW in Rust, eliminating the segfault issues present in the C implementation.

## Features

- **Memory Safety**: No more segfaults from dangling pointers, buffer overflows, or use-after-free bugs
- **Thread Safety**: Safe concurrent access through Rust's ownership system
- **Connection Pooling**: Efficient connection reuse with automatic cleanup
- **Full DML Support**: SELECT, INSERT, UPDATE, DELETE, and TRUNCATE
- **JWT Authentication**: Support for JWT token-based authentication (DB2 11.5.4+)
- **Query Pushdown**: WHERE clauses, JOINs, and aggregates pushed to DB2
- **Batch Inserts**: Efficient bulk insert operations (PostgreSQL 14+)

## Requirements

- PostgreSQL 12 or higher
- Rust 1.75 or higher
- IBM DB2 ODBC driver
- cargo-pgrx

## Installation

### From Source

```bash
# Install cargo-pgrx if not already installed
cargo install cargo-pgrx --locked

# Initialize pgrx
cargo pgrx init

# Build the extension
make build

# Install (requires superuser)
sudo make install

# Enable in PostgreSQL
psql -c "CREATE EXTENSION db2_fdw;"
```

### Using cargo-pgrx

```bash
# Build and run tests
cargo pgrx test pg16

# Install directly
cargo pgrx install
```

## Usage

### Create a Foreign Server

```sql
CREATE SERVER db2_server
    FOREIGN DATA WRAPPER db2_fdw
    OPTIONS (
        dbserver 'MY_DB2_DSN',
        prefetch '200'
    );
```

### Create User Mapping (Password Auth)

```sql
CREATE USER MAPPING FOR CURRENT_USER
    SERVER db2_server
    OPTIONS (
        user 'db2user',
        password 'db2password'
    );
```

### Create User Mapping (JWT Auth)

```sql
CREATE USER MAPPING FOR CURRENT_USER
    SERVER db2_server
    OPTIONS (
        jwt_token 'eyJhbGciOiJSUzI1NiIs...'
    );
```

### Create Foreign Table

```sql
CREATE FOREIGN TABLE employees (
    emp_id INTEGER OPTIONS (key 'true'),
    first_name VARCHAR(50),
    last_name VARCHAR(50),
    hire_date DATE,
    salary NUMERIC(10,2)
)
SERVER db2_server
OPTIONS (
    schema 'HR',
    table 'EMPLOYEES'
);
```

### Import Foreign Schema

```sql
IMPORT FOREIGN SCHEMA "HR"
    FROM SERVER db2_server
    INTO public;
```

## Options

### Server Options

| Option | Description | Default |
|--------|-------------|---------|
| `dbserver` | DB2 DSN or connection string | (required) |
| `nls_lang` | NLS language setting | (system default) |
| `prefetch` | Row prefetch count | 200 |
| `batch_size` | Batch insert size | 1 |

### Table Options

| Option | Description | Default |
|--------|-------------|---------|
| `schema` | Remote schema name | (none) |
| `table` | Remote table name | (required) |
| `readonly` | Treat as read-only | off |
| `max_long` | Max LONG column size | 32767 |
| `sample_percent` | ANALYZE sample rate | (none) |

### User Mapping Options

| Option | Description |
|--------|-------------|
| `user` | DB2 username |
| `password` | DB2 password |
| `jwt_token` | JWT token (alternative to user/password) |

### Column Options

| Option | Description | Default |
|--------|-------------|---------|
| `key` | Part of primary key | off |

## Diagnostic Functions

```sql
-- Show FDW diagnostics
SELECT * FROM db2_diag();

-- Close all connections
SELECT db2_close_connections();
```

## Differences from C Implementation

### Improvements

1. **No Segfaults**: Rust's ownership system prevents memory safety bugs
2. **Better Error Messages**: Structured error types with context
3. **Connection Pool**: Lock-free concurrent HashMap instead of linked lists
4. **Resource Cleanup**: RAII guarantees cleanup even on errors
5. **Type Safety**: Compile-time type checking for SQL operations

### API Compatibility

The SQL interface is fully compatible with the C implementation. Existing scripts and configurations will work without modification.

## Building

```bash
# Debug build
cargo build

# Release build
cargo build --release

# Run tests
cargo test --all

# Run PostgreSQL tests
cargo pgrx test pg16

# Generate documentation
cargo doc --open
```

## Project Structure

```
db2_fdw_rs/
├── crates/
│   ├── db2_fdw/          # Main FDW extension
│   ├── db2_connection/   # Connection pool
│   ├── db2_odbc/         # ODBC wrapper
│   └── db2_query/        # Query processing
├── sql/                  # SQL scripts
└── Makefile              # Build system
```

## License

PostgreSQL License (same as the original C implementation)

## Authors

- Thomas Muenz
- Wolfgang Brandl
- Contributors

## See Also

- [Original C Implementation](https://github.com/Living-Mainframe/db2_fdw)
- [pgrx Documentation](https://github.com/pgcentralfoundation/pgrx)
- [PostgreSQL FDW Documentation](https://www.postgresql.org/docs/current/fdw.html)
