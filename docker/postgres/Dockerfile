# PostgreSQL with DB2 FDW extension
FROM postgres:17-bookworm AS builder

# Install build dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential \
    postgresql-server-dev-17 \
    curl \
    ca-certificates \
    unixodbc \
    unixodbc-dev \
    libxml2 \
    libpam0g \
    && rm -rf /var/lib/apt/lists/*

# Download IBM Data Server Driver (CLI driver - free for ODBC)
WORKDIR /tmp
RUN curl -L -o ibm_data_server_driver.tar.gz \
    "https://public.dhe.ibm.com/ibmdl/export/pub/software/data/db2/drivers/odbc_cli/linuxx64_odbc_cli.tar.gz" \
    && mkdir -p /opt/ibm \
    && tar -xzf ibm_data_server_driver.tar.gz -C /opt/ibm \
    && rm ibm_data_server_driver.tar.gz \
    && mv /opt/ibm/clidriver /opt/ibm/db2

# Set DB2 environment for build
ENV DB2_HOME=/opt/ibm/db2
ENV LD_LIBRARY_PATH=/opt/ibm/db2/lib:$LD_LIBRARY_PATH

# Copy and build db2_fdw
WORKDIR /build/db2_fdw
COPY Makefile db2_fdw.control ./
COPY source/ ./source/
COPY include/ ./include/
COPY sql/ ./sql/

# Adjust for clidriver structure (lib instead of lib64)
RUN sed -i 's|lib64|lib|g' Makefile

# Build the extension
RUN make

# --- Runtime stage ---
FROM postgres:17-bookworm

# Install runtime dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    unixodbc \
    libxml2 \
    libpam0g \
    && rm -rf /var/lib/apt/lists/*

# Copy DB2 driver from builder
COPY --from=builder /opt/ibm/db2 /opt/ibm/db2

# Copy built extension from builder
COPY --from=builder /build/db2_fdw/db2_fdw.so /usr/lib/postgresql/17/lib/
COPY --from=builder /build/db2_fdw/db2_fdw.control /usr/share/postgresql/17/extension/
COPY --from=builder /build/db2_fdw/sql/*.sql /usr/share/postgresql/17/extension/

# Set runtime environment
ENV DB2_HOME=/opt/ibm/db2
ENV LD_LIBRARY_PATH=/opt/ibm/db2/lib:$LD_LIBRARY_PATH

# Configure ODBC DSN
RUN echo "[ODBC Data Sources]" > /etc/odbc.ini && \
    echo "db2testdb = IBM DB2 ODBC Driver" >> /etc/odbc.ini && \
    echo "" >> /etc/odbc.ini && \
    echo "[db2testdb]" >> /etc/odbc.ini && \
    echo "Driver = /opt/ibm/db2/lib/libdb2o.so" >> /etc/odbc.ini && \
    echo "Description = DB2 Test Database" >> /etc/odbc.ini && \
    echo "Protocol = TCPIP" >> /etc/odbc.ini && \
    echo "Hostname = db2" >> /etc/odbc.ini && \
    echo "Port = 50000" >> /etc/odbc.ini && \
    echo "Database = testdb" >> /etc/odbc.ini

# Configure ODBC driver
RUN echo "[IBM DB2 ODBC Driver]" > /etc/odbcinst.ini && \
    echo "Description = IBM DB2 ODBC Driver" >> /etc/odbcinst.ini && \
    echo "Driver = /opt/ibm/db2/lib/libdb2o.so" >> /etc/odbcinst.ini && \
    echo "FileUsage = 1" >> /etc/odbcinst.ini && \
    echo "DontDLClose = 1" >> /etc/odbcinst.ini

# Update library cache
RUN echo "/opt/ibm/db2/lib" > /etc/ld.so.conf.d/db2.conf && ldconfig

# Health check
HEALTHCHECK --interval=30s --timeout=10s --start-period=5s --retries=3 \
    CMD pg_isready -U postgres || exit 1
