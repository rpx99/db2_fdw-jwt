-- PostgreSQL FDW Setup for DB2
-- This script creates the foreign data wrapper and foreign tables

-- Create the extension
CREATE EXTENSION IF NOT EXISTS db2_fdw;

-- Create the foreign server
-- Using direct connection parameters (not DSN)
CREATE SERVER db2_server
    FOREIGN DATA WRAPPER db2_fdw
    OPTIONS (
        dbserver '//db2:50000/testdb'
    );

-- Create user mapping
-- Maps PostgreSQL user to DB2 credentials
CREATE USER MAPPING FOR postgres
    SERVER db2_server
    OPTIONS (
        user 'db2inst1',
        password 'db2password'
    );

-- Import the foreign schema (auto-creates foreign tables)
-- This imports all tables from fdw_test schema
IMPORT FOREIGN SCHEMA "FDW_TEST"
    FROM SERVER db2_server
    INTO public;

-- Alternative: Create foreign tables manually
-- Uncomment if IMPORT FOREIGN SCHEMA doesn't work

-- CREATE FOREIGN TABLE employees (
--     id INTEGER OPTIONS (key 'true'),
--     first_name VARCHAR(50),
--     last_name VARCHAR(50),
--     email VARCHAR(100),
--     department VARCHAR(50),
--     salary NUMERIC(10,2),
--     hire_date DATE,
--     active SMALLINT
-- )
-- SERVER db2_server
-- OPTIONS (
--     schema 'FDW_TEST',
--     table 'EMPLOYEES'
-- );

-- CREATE FOREIGN TABLE data_types_test (
--     id INTEGER OPTIONS (key 'true'),
--     col_smallint SMALLINT,
--     col_integer INTEGER,
--     col_bigint BIGINT,
--     col_decimal NUMERIC(15,4),
--     col_real REAL,
--     col_double DOUBLE PRECISION,
--     col_char CHAR(10),
--     col_varchar VARCHAR(100),
--     col_date DATE,
--     col_time TIME,
--     col_timestamp TIMESTAMP,
--     col_clob TEXT,
--     col_blob BYTEA
-- )
-- SERVER db2_server
-- OPTIONS (
--     schema 'FDW_TEST',
--     table 'DATA_TYPES_TEST'
-- );

-- Test query (will be executed after DB2 is ready)
-- SELECT * FROM employees LIMIT 5;
