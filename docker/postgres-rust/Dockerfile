# Complete Rust FDW Build Environment
# PostgreSQL 18 (beta) + pgrx + DB2 ODBC Driver

FROM debian:bookworm-slim AS builder

# Build arguments
ARG PG_MAJOR=18

# Install build dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential \
    ca-certificates \
    curl \
    git \
    pkg-config \
    libssl-dev \
    libclang-dev \
    clang \
    llvm \
    unixodbc-dev \
    gnupg2 \
    lsb-release \
    && rm -rf /var/lib/apt/lists/*

# Add PostgreSQL APT repository for PG 18
RUN curl -fsSL https://www.postgresql.org/media/keys/ACCC4CF8.asc | gpg --dearmor -o /usr/share/keyrings/postgresql.gpg \
    && echo "deb [signed-by=/usr/share/keyrings/postgresql.gpg] http://apt.postgresql.org/pub/repos/apt $(lsb_release -cs)-pgdg main" > /etc/apt/sources.list.d/pgdg.list

# Install PostgreSQL 18 (or 17 if 18 not available)
RUN apt-get update && (apt-get install -y --no-install-recommends \
    postgresql-${PG_MAJOR} \
    postgresql-server-dev-${PG_MAJOR} \
    || apt-get install -y --no-install-recommends \
    postgresql-17 \
    postgresql-server-dev-17) \
    && rm -rf /var/lib/apt/lists/*

# Detect installed PostgreSQL version
RUN PG_VERSION=$(ls /usr/lib/postgresql/ | sort -V | tail -1) && \
    echo "Installed PostgreSQL version: ${PG_VERSION}" && \
    echo "export PG_VERSION=${PG_VERSION}" >> /etc/profile.d/pg.sh

# Install Rust
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
ENV PATH="/root/.cargo/bin:${PATH}"

# Install cargo-pgrx
RUN cargo install cargo-pgrx --version "0.12.9" --locked

# Initialize pgrx with installed PostgreSQL
RUN . /etc/profile.d/pg.sh && \
    cargo pgrx init --pg${PG_VERSION} /usr/lib/postgresql/${PG_VERSION}/bin/pg_config

# Download IBM Data Server Driver (CLI driver - free for ODBC)
WORKDIR /tmp
RUN curl -L -o ibm_data_server_driver.tar.gz \
    "https://public.dhe.ibm.com/ibmdl/export/pub/software/data/db2/drivers/odbc_cli/linuxx64_odbc_cli.tar.gz" \
    && mkdir -p /opt/ibm \
    && tar -xzf ibm_data_server_driver.tar.gz -C /opt/ibm \
    && rm ibm_data_server_driver.tar.gz \
    && mv /opt/ibm/clidriver /opt/ibm/db2

# Set DB2 environment
ENV DB2_HOME=/opt/ibm/db2
ENV LD_LIBRARY_PATH=/opt/ibm/db2/lib:${LD_LIBRARY_PATH}
ENV LIBRARY_PATH=/opt/ibm/db2/lib:${LIBRARY_PATH}
ENV C_INCLUDE_PATH=/opt/ibm/db2/include:${C_INCLUDE_PATH}

# Update library cache
RUN echo "/opt/ibm/db2/lib" > /etc/ld.so.conf.d/db2.conf && ldconfig

# Copy source code
WORKDIR /build/db2_fdw_rs
COPY db2_fdw_rs/ ./

# Run Memory Safety Tests before building
RUN . /etc/profile.d/pg.sh && \
    echo "Running Memory Safety Tests..." && \
    cargo pgrx test --package db2_fdw --features pg${PG_VERSION} || \
    (echo "Memory Safety Tests FAILED!" && exit 1)

# Build the Rust FDW
RUN . /etc/profile.d/pg.sh && \
    cargo pgrx package --pg-config /usr/lib/postgresql/${PG_VERSION}/bin/pg_config

# --- Runtime stage ---
FROM debian:bookworm-slim

ARG PG_MAJOR=18

# Install runtime dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    unixodbc \
    libxml2 \
    gnupg2 \
    lsb-release \
    && rm -rf /var/lib/apt/lists/*

# Add PostgreSQL APT repository
RUN curl -fsSL https://www.postgresql.org/media/keys/ACCC4CF8.asc | gpg --dearmor -o /usr/share/keyrings/postgresql.gpg \
    && echo "deb [signed-by=/usr/share/keyrings/postgresql.gpg] http://apt.postgresql.org/pub/repos/apt $(lsb_release -cs)-pgdg main" > /etc/apt/sources.list.d/pgdg.list

# Install PostgreSQL
RUN apt-get update && (apt-get install -y --no-install-recommends \
    postgresql-${PG_MAJOR} \
    || apt-get install -y --no-install-recommends \
    postgresql-17) \
    && rm -rf /var/lib/apt/lists/*

# Detect installed PostgreSQL version
RUN PG_VERSION=$(ls /usr/lib/postgresql/ | sort -V | tail -1) && \
    echo "export PG_VERSION=${PG_VERSION}" >> /etc/profile.d/pg.sh

# Copy DB2 driver from builder
COPY --from=builder /opt/ibm/db2 /opt/ibm/db2

# Copy built extension from builder
COPY --from=builder /build/db2_fdw_rs/target/release/db2_fdw-pg*/usr/ /usr/

# Set runtime environment
ENV DB2_HOME=/opt/ibm/db2
ENV LD_LIBRARY_PATH=/opt/ibm/db2/lib:${LD_LIBRARY_PATH}

# Configure ODBC
RUN echo "[IBM DB2 ODBC Driver]" > /etc/odbcinst.ini && \
    echo "Description = IBM DB2 ODBC Driver" >> /etc/odbcinst.ini && \
    echo "Driver = /opt/ibm/db2/lib/libdb2o.so" >> /etc/odbcinst.ini && \
    echo "FileUsage = 1" >> /etc/odbcinst.ini

# Update library cache
RUN echo "/opt/ibm/db2/lib" > /etc/ld.so.conf.d/db2.conf && ldconfig

# Create postgres user home
RUN mkdir -p /var/run/postgresql && chown -R postgres:postgres /var/run/postgresql

USER postgres

# Health check
HEALTHCHECK --interval=30s --timeout=10s --start-period=5s --retries=3 \
    CMD pg_isready -U postgres || exit 1

CMD ["postgres"]
