-- DB2 Foreign Data Wrapper for PostgreSQL
-- Rust Implementation
-- Version 18.2.0

-- Complain if script is sourced in psql, rather than via CREATE EXTENSION
\echo Use "CREATE EXTENSION db2_fdw" to load this file. \quit

-- Create the FDW handler function
CREATE OR REPLACE FUNCTION db2_fdw_handler()
RETURNS fdw_handler
AS 'MODULE_PATHNAME', 'db2_fdw_handler'
LANGUAGE C;

-- Create the FDW validator function
CREATE OR REPLACE FUNCTION db2_fdw_validator(text[], oid)
RETURNS void
AS 'MODULE_PATHNAME', 'db2_fdw_validator'
LANGUAGE C;

-- Create the Foreign Data Wrapper
CREATE FOREIGN DATA WRAPPER db2_fdw
    HANDLER db2_fdw_handler
    VALIDATOR db2_fdw_validator;

-- Create the utility function to close all connections
CREATE OR REPLACE FUNCTION db2_close_connections()
RETURNS void
AS 'MODULE_PATHNAME', 'db2_close_connections'
LANGUAGE C STRICT;

COMMENT ON FUNCTION db2_close_connections() IS
'Close all cached connections to DB2 databases';

-- Create the diagnostic function
CREATE OR REPLACE FUNCTION db2_diag()
RETURNS TABLE(name text, value text)
AS 'MODULE_PATHNAME', 'db2_diag'
LANGUAGE C STRICT;

COMMENT ON FUNCTION db2_diag() IS
'Display diagnostic information about the DB2 FDW';

-- Grant usage to public
GRANT USAGE ON FOREIGN DATA WRAPPER db2_fdw TO PUBLIC;
