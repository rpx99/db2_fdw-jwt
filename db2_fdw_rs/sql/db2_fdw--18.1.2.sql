-- DB2 Foreign Data Wrapper for PostgreSQL
-- Rust Implementation
-- Version 18.1.2

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

-- Grant usage to public
GRANT USAGE ON FOREIGN DATA WRAPPER db2_fdw TO PUBLIC;
