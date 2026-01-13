# DB2 FDW Rust Edition - Development & Testing Dockerfile
# Includes full Memory Safety Test Suite

FROM postgres:18.1 AS builder

USER root

# Build dependencies
RUN apt-get update && apt-get install -y \
    git build-essential postgresql-server-dev-18 \
    wget curl tar ksh unzip iputils-ping \
    unixodbc-dev unixodbc pkg-config libssl-dev \
    clang llvm libclang-dev \
    && rm -rf /var/lib/apt/lists/*

# Install Rust
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
ENV PATH="/root/.cargo/bin:${PATH}"

# Install cargo-pgrx
RUN cargo install cargo-pgrx --version "0.16.1" --locked

# Clone repository
RUN git clone https://github.com/rpx99/db2_fdw-jwt /tmp/db2_fdw && \
    cd /tmp/db2_fdw && \
    git checkout claude/rust-rewrite-project-2LX5o

WORKDIR /tmp/db2_fdw/db2_fdw_rs

# COPY IBM Data Server Driver (falls vorhanden)
COPY v11.5.9_linuxx64_dsdriver.tar.gz /tmp/ 2>/dev/null || echo "Skipping IBM driver - tests only"

# Install dsdriver (optional für Tests)
RUN if [ -f /tmp/v11.5.9_linuxx64_dsdriver.tar.gz ]; then \
        mkdir -p /opt/ibm && \
        cd /opt/ibm && \
        tar -xzf /tmp/v11.5.9_linuxx64_dsdriver.tar.gz && \
        rm /tmp/v11.5.9_linuxx64_dsdriver.tar.gz && \
        cd dsdriver && \
        ./installDSDriver && \
        ln -s /opt/ibm/dsdriver/lib /opt/ibm/dsdriver/lib64 && \
        echo "DB2_DRIVER_INSTALLED=1" && \
    else \
        echo "DB2_DRIVER_INSTALLED=0" && \
    fi

# Set DB2 environment (falls installiert)
ENV DB2HOME=/opt/ibm/dsdriver
ENV LD_LIBRARY_PATH=/opt/ibm/dsdriver/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}
ENV LIBRARY_PATH=/opt/ibm/dsdriver/lib
ENV C_INCLUDE_PATH=/opt/ibm/dsdriver/include
RUN if [ "$DB2_DRIVER_INSTALLED" = "1" ]; then \
        echo "/opt/ibm/dsdriver/lib" > /etc/ld.so.conf.d/db2.conf && \
        ldconfig; \
    fi

# ============================================================================
# AUTOMATED MEMORY SAFETY TESTS
# ============================================================================
# These tests catch the bugs we fixed BEFORE they reach production
# ============================================================================

RUN echo "==========================================" && \
    echo "Running Memory Safety Tests..." && \
    echo "==========================================" && \
    cargo test --package db2_fdw --features pg18 --no-fail-fast \
        -- -Z unstable-options --test-threads=1 && \
    echo "==========================================" && \
    echo "✓ ALL MEMORY SAFETY TESTS PASSED!" && \
    echo "==========================================" || \
    (echo "==========================================" && \
     echo "✗ MEMORY SAFETY TESTS FAILED!" && \
     echo "Build aborted - would ship with memory bugs!" && \
     echo "==========================================" && \
     exit 1)

# ============================================================================
# RUST LINTER & ANALYZER
# ============================================================================

RUN echo "Running Rust linters (clippy)..." && \
    cargo clippy --package db2_fdw --features pg18 -- \
        -D warnings -D clippy::all || \
    (echo "Clippy found issues! Fix before shipping." && exit 1)

# ============================================================================
# BUILD EXTENSION
# ============================================================================

RUN echo "Building db2_fdw extension in release mode..." && \
    cargo build --release --package db2_fdw --features pg18

# ============================================================================
# VERIFICATION
# ============================================================================

RUN echo "Verifying built artifacts..." && \
    ls -lh target/release/libdb2_fdw.so && \
    file target/release/libdb2_fdw.so | grep -q "shared object" && \
    echo "✓ Extension built successfully"

# ============================================================================
# COPY ARTIFACTS FOR INSTALLATION
# ============================================================================

RUN mkdir -p /tmp/artifacts/lib /tmp/artifacts/share && \
    cp target/release/libdb2_fdw.so /tmp/artifacts/lib/db2_fdw.so && \
    cp db2_fdw.control /tmp/artifacts/share/ && \
    cp sql/*.sql /tmp/artifacts/share/

# ============================================================================
# RUNTIME IMAGE
# ============================================================================

FROM postgres:18.1

USER root

# Runtime dependencies
RUN apt-get update && apt-get install -y \
    wget curl tar ksh unzip iputils-ping unixodbc libodbc2 \
    && rm -rf /var/lib/apt/lists/*

# Copy DB2 driver from builder
COPY --from=builder /opt/ibm/dsdriver /opt/ibm/dsdriver

# Copy built extension from builder
COPY --from=builder /tmp/artifacts/lib/db2_fdw.so /usr/lib/postgresql/18/lib/
COPY --from=builder /tmp/artifacts/share/db2_fdw.control /usr/share/postgresql/18/extension/
COPY --from=builder /tmp/artifacts/share/*.sql /usr/share/postgresql/18/extension/

# Set runtime environment
ENV DB2HOME=/opt/ibm/dsdriver
ENV DB2LIB=/opt/ibm/dsdriver/lib
ENV LD_LIBRARY_PATH=/opt/ibm/dsdriver/lib
ENV PATH=/opt/ibm/dsdriver/bin:$PATH

# Update library cache
RUN echo "/opt/ibm/dsdriver/lib" > /etc/ld.so.conf.d/db2.conf && ldconfig

# DSN configuration
COPY db2dsdriver.cfg /opt/ibm/dsdriver/cfg/db2dsdriver.cfg
ENV DB2DSDRIVER_CFG_PATH=/opt/ibm/dsdriver/cfg
ENV DB2CLIINIPATH=/opt/ibm/dsdriver/cfg

# Hartcodiertes PostgreSQL-Passwort
ENV POSTGRES_PASSWORD=Admindb123

# Init SQL (IMPORT noch deaktiviert wegen malloc Bug)
COPY docker-init.sql /docker-entrypoint-initdb.d/init.sql

# Health check
HEALTHCHECK --interval=30s --timeout=10s --start-period=5s --retries=3 \
    CMD pg_isready -U postgres || exit 1

USER postgres

CMD ["postgres"]
