# Agent Guide for db2_fdw

This repository contains a PostgreSQL Foreign Data Wrapper (FDW) extension for DB2 database access. This guide provides essential information for agents working with this codebase.

## Project Overview

**Project Type**: PostgreSQL C Extension (Foreign Data Wrapper)  
**Primary Language**: C  
**Purpose**: Enables PostgreSQL to query and modify DB2 database tables as foreign tables  
**Version**: 18.0.1  

## Directory Structure

```
/home/rpx/vibe-coding/db2_fdw-fork/
├── source/              # C source files (58 *.c files)
├── include/             # Header files (9 *.h files)
├── sql/                 # SQL installation scripts
├── test/                # Regression tests (sql/, expected/)
├── doc/                 # Documentation files
├── Makefile             # Build configuration
├── db2_fdw.control      # Extension control file
├── META.json            # Extension metadata
└── AGENTS.md           # This file
```

## Essential Commands

### Building the Extension
```bash
# Standard build (requires DB2_HOME environment variable)
make

# Build within PostgreSQL source tree
make NO_PGXS=1

# Install extension
make install

# Run regression tests (requires DB2 and PostgreSQL setup)
make installcheck
```

### Environment Requirements
- **DB2_HOME**: Must point to DB2 installation directory
- **pg_config**: Must be in PATH
- **PostgreSQL**: Development headers and PGXS infrastructure required
- **DB2 Client**: Version 11.1 or better with SDK headers

### Test Database Setup
For `make installcheck` to work:
1. DB2 SAMPLE database must exist
2. OS user with password authentication required
3. User needs DBADM rights on SAMPLE database

## Code Organization and Patterns

### Source File Naming Convention
- **Main file**: `db2_fdw.c` (entry point, FDW handler)
- **Utility files**: `db2_*_utils.c` (shared functionality)
- **DB2 interface**: `db2*` prefix for all files
- **Specific functions**: Descriptive names like `db2GetForeignPlan.c`

### Header Files Structure
```
include/
├── db2_fdw.h          # Main extension header
├── db2_pg.h           # PostgreSQL interface header
├── DB2*.h             # DB2-specific structures
└── *.h                # Supporting headers
```

### Key Source Files
- **db2_fdw.c**: Entry point, FDW API implementation
- **source/db2_fdw_utils.c**: Core utility functions
- **source/db2GetForeignPlan.c**: Query planning
- **source/db2ExecForeign*.c**: INSERT/UPDATE/DELETE operations

## Coding Conventions and Patterns

### C Code Style
- **Function naming**: `db2FunctionName` pattern with mixed case
- **Variables**: Mixed case, descriptive names
- **Comments**: C-style `/* */` blocks with function descriptions
- **Debugging**: `db2Debug1/2/3` functions for logging

### PostgreSQL Extension Patterns
```c
PG_MODULE_MAGIC;           // Required for all extensions

// SQL function declarations
extern PGDLLEXPORT Datum db2_fdw_handler(PG_FUNCTION_ARGS);
PG_FUNCTION_INFO_V1(db2_fdw_handler);

// Version-specific includes
#if PG_VERSION_NUM >= 180000
    // PostgreSQL 18+ specific code
#endif
```

### Error Handling Patterns
- PostgreSQL `elog()` functions for errors/logging
- DB2 API return code checking
- Transaction-aware error handling

### Memory Management
- PostgreSQL `palloc/pfree` for extension memory
- DB2-specific memory management for connections/statements
- Session-scoped connection caching

## Testing Approach

### Test Structure
- **Location**: `test/sql/` (SQL test files)
- **Expected results**: `test/expected/` (`.out` files)
- **Test framework**: PostgreSQL standard regression tests

### Running Tests
```bash
# Requires configured DB2 environment
make installcheck
```

### Test Database Requirements
- PostgreSQL 10.1+ cluster
- DB2 server 11.1+ with SAMPLE database
- Configured user permissions

## Important Gotchas and Known Issues

### Version Compatibility
- **PostgreSQL**: Requires 10.1 or better
- **DB2 Client**: Version 11.1 or better required
- **Architecture**: PostgreSQL and DB2 must match (32-bit/64-bit)

### Environment Setup Critical Issues
1. **DB2 Environment Variables**: Must be sourced from `db2profile`
2. **LD_LIBRARY_PATH**: DB2 client libraries must be accessible
3. **System Locale**: Windows requires English (United States) locale
4. **Code Page**: DB2 database and PostgreSQL should use matching code pages

### Development-Specific Gotchas
1. **Build Dependencies**: Both PostgreSQL dev headers AND DB2 SDK required
2. **Conditional Compilation**: Extensive version checking with `PG_VERSION_NUM`
3. **Connection Caching**: Connections cached per session, auto-closed at end
4. **Transaction Context**: Some operations cannot run inside certain transactions

### Platform-Specific Issues
- **Windows**: Limited to English (United States) system locale
- **Linux**: DB2 libraries may need LD_LIBRARY_PATH configuration
- **Symbolic Links**: DB2 Instant Client may need manual libclntsh.so symlink

## Build Configuration Details

### Makefile Structure
- Uses PostgreSQL PGXS infrastructure
- **EXTENSION**: `db2_fdw`
- **MODULE_big**: Shared library name
- **OBJS**: Compiled object files list
- **SHLIB_LINK**: DB2 client library linking
- **PG_CPPFLAGS**: Compiler flags including DB2 include paths

### Environment Variables
```bash
export DB2_HOME=/path/to/db2/installation
export PATH=$PATH:$DB2_HOME/bin  # For db2expln command
export LD_LIBRARY_PATH=$DB2_HOME/lib64:$LD_LIBRARY_PATH
```

### Compiler Flags
- `-g -fPIC`: Debug info and position-independent code
- `-I$(DB2_HOME)/include`: DB2 header paths
- `-I./include`: Extension header paths
- `-L$(DB2_HOME)/lib64`: DB2 library paths

## Extension Installation and Usage

### Installation Steps
```sql
-- Create extension
CREATE EXTENSION db2_fdw;

-- Create foreign server
CREATE SERVER sample FOREIGN DATA WRAPPER db2_fdw 
  OPTIONS (dbserver 'SAMPLE');

-- Grant usage
GRANT USAGE ON FOREIGN SERVER sample TO pguser;

-- Create user mapping
CREATE USER MAPPING FOR PUBLIC SERVER sample 
  OPTIONS (user '', password '');

-- Import schema
IMPORT FOREIGN SCHEMA "DB2INST1" 
  FROM SERVER sample INTO public;
```

### Key Options
- **dbserver**: DB2 database connection string
- **user/password**: DB2 credentials (empty for external auth)
- **table**: DB2 table name (usually uppercase)
- **schema**: DB2 schema name (optional)
- **readonly**: Prevent INSERT/UPDATE/DELETE
- **max_long**: Maximum LONG column size
- **prefetch**: Row prefetch count (default 200)

## Debugging and Diagnostics

### Debug Functions
- **db2Debug1/2/3**: Logging functions with increasing verbosity
- **db2_diag()**: SQL function for diagnostic information

### Diagnostic Information
```sql
-- Get version and environment info
SELECT db2_diag();

-- Get server-specific info
SELECT db2_diag('server_name');
```

### Common Error Patterns
- **Connection failures**: Check DB2 environment variables
- **Type conversion errors**: Verify column type mappings
- **Permission errors**: Ensure DB2 user has appropriate rights
- **Locale issues**: Check system and database code pages

## Development Workflow

### Adding New Features
1. **C files**: Add to `source/` directory
2. **Headers**: Add corresponding declarations to `include/`
3. **Makefile**: Add new `.o` file to `OBJS` list
4. **Tests**: Add regression tests to `test/sql/`

### Code Review Focus Areas
- PostgreSQL version compatibility (`PG_VERSION_NUM` checks)
- Memory management (palloc vs DB2 allocations)
- Error handling and transaction context
- DB2 API return code checking
- Debug logging appropriateness

### Testing Changes
1. **Build verification**: `make clean && make`
2. **Installation test**: `make install`
3. **Regression tests**: `make installcheck` (with DB2)
4. **Manual testing**: Basic CREATE EXTENSION and foreign table operations

## Repository Maintenance

### Git Ignore Patterns
```
*.o          # Compiled objects
*.so         # Shared libraries
*.bc         # Bitcode files (if generated)
test.bash    # Generated test scripts
.vscode/     # VSCode settings
```

### Release Management
- Version controlled via `db2_fdw.control` and `META.json`
- SQL upgrade scripts in `sql/` directory
- Documentation updates in `doc/` directory

### Known Contributors
- **Primary Author**: Wolfgang Brandl
- **Major Contributions**: Laurenz Alba (Austria)
- **Current Maintainer**: Thomas Muenz (thomas.muenz@living-mainframe.de)

## Resources

### Documentation
- **README.md**: Comprehensive usage and installation guide
- **doc/db2_fdw.md**: Additional documentation
- **META.json**: Extension metadata and dependencies

### External Resources
- [PostgreSQL FDW Documentation](https://www.postgresql.org/docs/current/static/ddl-foreign-data.html)
- [DB2 Express-C Download](https://www.ibm.com/developerworks/downloads/im/db2express/)
- [GitHub Repository](https://github.com/Living-Mainframe/db2_fdw)

### Support Channels
- **GitHub Issues**: [Project Repository](https://github.com/Living-Mainframe/db2_fdw)
- **Email**: thomas.muenz@living-mainframe.de
- **Wiki**: Project-specific tips and configuration help

---

**Last Updated**: Based on repository analysis  
**Repository**: https://github.com/Living-Mainframe/db2_fdw  
**License**: PostgreSQL License