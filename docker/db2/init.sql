-- DB2 Test Database Initialization
-- This script creates a test table for FDW testing

-- Connect to testdb (created automatically by DBNAME env var)
CONNECT TO testdb;

-- Create a test schema
CREATE SCHEMA fdw_test;

-- Create a simple test table
CREATE TABLE fdw_test.employees (
    id INTEGER NOT NULL GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    first_name VARCHAR(50) NOT NULL,
    last_name VARCHAR(50) NOT NULL,
    email VARCHAR(100),
    department VARCHAR(50),
    salary DECIMAL(10,2),
    hire_date DATE,
    active SMALLINT DEFAULT 1
);

-- Insert sample data
INSERT INTO fdw_test.employees (first_name, last_name, email, department, salary, hire_date, active) VALUES
    ('Max', 'Mustermann', 'max@example.com', 'Engineering', 75000.00, '2020-01-15', 1),
    ('Erika', 'Musterfrau', 'erika@example.com', 'Marketing', 65000.00, '2019-06-01', 1),
    ('Hans', 'Schmidt', 'hans@example.com', 'Engineering', 80000.00, '2018-03-20', 1),
    ('Anna', 'Mueller', 'anna@example.com', 'Sales', 70000.00, '2021-09-10', 1),
    ('Peter', 'Weber', 'peter@example.com', 'Engineering', 85000.00, '2017-11-30', 0);

-- Create a table with various data types for comprehensive testing
CREATE TABLE fdw_test.data_types_test (
    id INTEGER NOT NULL GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    col_smallint SMALLINT,
    col_integer INTEGER,
    col_bigint BIGINT,
    col_decimal DECIMAL(15,4),
    col_real REAL,
    col_double DOUBLE,
    col_char CHAR(10),
    col_varchar VARCHAR(100),
    col_date DATE,
    col_time TIME,
    col_timestamp TIMESTAMP,
    col_clob CLOB(1M),
    col_blob BLOB(1M)
);

-- Insert test data for data types
INSERT INTO fdw_test.data_types_test
    (col_smallint, col_integer, col_bigint, col_decimal, col_real, col_double,
     col_char, col_varchar, col_date, col_time, col_timestamp, col_clob)
VALUES
    (32767, 2147483647, 9223372036854775807, 12345.6789, 3.14, 3.14159265359,
     'CHAR      ', 'Variable length string', '2024-01-15', '14:30:00', '2024-01-15-14.30.00.000000',
     'This is a CLOB text field for testing large text data.');

COMMIT;

-- Grant permissions
GRANT SELECT, INSERT, UPDATE, DELETE ON fdw_test.employees TO PUBLIC;
GRANT SELECT, INSERT, UPDATE, DELETE ON fdw_test.data_types_test TO PUBLIC;

DISCONNECT testdb;
