-- Initialize DB2 FDW Extension
CREATE EXTENSION IF NOT EXISTS db2_fdw;

-- The extension is now loaded and ready
-- You can create foreign servers and user mappings when you have a DB2 database available:
--
-- CREATE SERVER db2_server FOREIGN DATA WRAPPER db2_fdw
--   OPTIONS (dbserver 'YOUR_DSN');
-- CREATE USER MAPPING FOR postgres SERVER db2_server
--   OPTIONS (user 'YOUR_USER', password 'YOUR_PASSWORD');
