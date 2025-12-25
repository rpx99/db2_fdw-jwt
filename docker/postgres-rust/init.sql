-- PostgreSQL Rust FDW Setup for DB2
-- This script creates the foreign data wrapper and foreign tables
-- Uses the complete Rust implementation (db2_fdw)

-- Create the Rust extension
CREATE EXTENSION IF NOT EXISTS db2_fdw;

-- Show extension info
SELECT * FROM db2_diag();

-- Create the foreign server
CREATE SERVER db2_server
    FOREIGN DATA WRAPPER db2_fdw
    OPTIONS (
        dbserver '//db2:50000/testdb',
        nls_lang 'AMERICAN_AMERICA.UTF8'
    );

-- Create user mapping
CREATE USER MAPPING FOR postgres
    SERVER db2_server
    OPTIONS (
        user 'db2inst1',
        password 'db2password'
    );

-- Import the foreign schema
IMPORT FOREIGN SCHEMA "FDW_TEST"
    FROM SERVER db2_server
    INTO public;

-- Show what was imported
SELECT
    c.relname as table_name,
    ft.ftoptions as options
FROM pg_foreign_table ft
JOIN pg_class c ON ft.ftrelid = c.oid
ORDER BY c.relname;

-- Test query
DO $$
BEGIN
    -- Give DB2 time to initialize fully
    PERFORM pg_sleep(5);

    -- Try a simple query
    RAISE NOTICE 'Testing FDW connection...';
END $$;

-- This will be executed after init
-- SELECT * FROM employees LIMIT 3;
