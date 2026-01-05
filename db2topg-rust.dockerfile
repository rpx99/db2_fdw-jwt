# DB2 FDW Rust Edition - PostgreSQL 18
# Replaces C implementation with memory-safe Rust

FROM postgres:18.1 AS builder

USER root

# Build dependencies
RUN apt-get update && apt-get install -y \
    git build-essential postgresql-server-dev-18 \
    wget curl tar ksh unzip \
    pkg-config libssl-dev libclang-dev clang llvm \
    && rm -rf /var/lib/apt/lists/*

# Install Rust
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
ENV PATH="/root/.cargo/bin:${PATH}"

# Install cargo-pgrx (must match pgrx version in Cargo.toml)
RUN cargo install cargo-pgrx --version "0.12.9" --locked

# Initialize pgrx with PostgreSQL 18
RUN cargo pgrx init --pg18 /usr/lib/postgresql/18/bin/pg_config

# Copy IBM Data Server Driver
COPY v11.5.9_linuxx64_dsdriver.tar.gz /tmp/
RUN mkdir -p /opt/ibm && \
    cd /opt/ibm && \
    tar -xzf /tmp/v11.5.9_linuxx64_dsdriver.tar.gz && \
    rm /tmp/v11.5.9_linuxx64_dsdriver.tar.gz

# Install dsdriver
RUN cd /opt/ibm/dsdriver && \
    ./installDSDriver && \
    echo "dsdriver installation completed"

# Symlink lib to lib64 if needed
RUN if [ ! -d /opt/ibm/dsdriver/lib64 ]; then \
        ln -s /opt/ibm/dsdriver/lib /opt/ibm/dsdriver/lib64; \
    fi

# Set DB2 environment for build
ENV DB2_HOME=/opt/ibm/dsdriver
ENV LD_LIBRARY_PATH=/opt/ibm/dsdriver/lib:${LD_LIBRARY_PATH}
ENV LIBRARY_PATH=/opt/ibm/dsdriver/lib:${LIBRARY_PATH}
ENV C_INCLUDE_PATH=/opt/ibm/dsdriver/include:${C_INCLUDE_PATH}

# Update library cache
RUN echo "/opt/ibm/dsdriver/lib" > /etc/ld.so.conf.d/db2.conf && ldconfig

# Clone and build Rust FDW
RUN git clone https://github.com/rpx99/db2_fdw-jwt /tmp/db2_fdw && \
    cd /tmp/db2_fdw && \
    git checkout claude/rust-rewrite-project-2LX5o

WORKDIR /tmp/db2_fdw/db2_fdw_rs

# Build with pgrx
RUN cargo pgrx package --pg-config /usr/lib/postgresql/18/bin/pg_config

# --- Runtime Stage ---
FROM postgres:18.1

USER root

# Runtime dependencies
RUN apt-get update && apt-get install -y \
    wget curl tar ksh unzip iputils-ping \
    && rm -rf /var/lib/apt/lists/*

# Copy DB2 driver from builder
COPY --from=builder /opt/ibm/dsdriver /opt/ibm/dsdriver

# Copy built Rust extension from builder
COPY --from=builder /tmp/db2_fdw/db2_fdw_rs/target/release/db2_fdw-pg18/usr/share/postgresql/18/extension/* /usr/share/postgresql/18/extension/
COPY --from=builder /tmp/db2_fdw/db2_fdw_rs/target/release/db2_fdw-pg18/usr/lib/postgresql/18/lib/* /usr/lib/postgresql/18/lib/

# Set runtime environment
ENV DB2_HOME=/opt/ibm/dsdriver
ENV DB2LIB=$DB2_HOME/lib
ENV LD_LIBRARY_PATH=$DB2LIB:${LD_LIBRARY_PATH}
ENV PATH=$DB2_HOME/bin:$PATH

# Update library cache
RUN echo "/opt/ibm/dsdriver/lib" > /etc/ld.so.conf.d/db2.conf && ldconfig

# DSN configuration
COPY db2dsdriver.cfg /opt/ibm/dsdriver/cfg/db2dsdriver.cfg
ENV DB2DSDRIVER_CFG_PATH=/opt/ibm/dsdriver/cfg
ENV DB2CLIINIPATH=/opt/ibm/dsdriver/cfg

USER postgres
