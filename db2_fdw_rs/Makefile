# DB2 FDW Rust Makefile
# Build system for the Rust-based DB2 Foreign Data Wrapper

EXTENSION = db2_fdw
EXTVERSION = 19.0.0

# PostgreSQL configuration
PG_CONFIG ?= pg_config
PGXS := $(shell $(PG_CONFIG) --pgxs)
PG_VERSION := $(shell $(PG_CONFIG) --version | cut -d' ' -f2 | cut -d'.' -f1)
SHAREDIR := $(shell $(PG_CONFIG) --sharedir)
PKGLIBDIR := $(shell $(PG_CONFIG) --pkglibdir)

# Rust configuration
CARGO ?= cargo
CARGO_FLAGS ?= --release

# Feature flags based on PostgreSQL version
PG_FEATURE := pg$(PG_VERSION)

.PHONY: all build test install clean check fmt clippy doc

all: build

# Build the extension
build:
	@echo "Building db2_fdw for PostgreSQL $(PG_VERSION)..."
	$(CARGO) build $(CARGO_FLAGS) --features $(PG_FEATURE)
	@echo "Build complete!"

# Run tests
test:
	@echo "Running tests..."
	$(CARGO) test --all

# Run pgrx tests (requires PostgreSQL)
test-pg:
	@echo "Running PostgreSQL integration tests..."
	$(CARGO) pgrx test $(PG_FEATURE)

# Install the extension
install: build
	@echo "Installing db2_fdw..."
	install -d $(DESTDIR)$(PKGLIBDIR)
	install -d $(DESTDIR)$(SHAREDIR)/extension
	install -m 755 target/release/libdb2_fdw.so $(DESTDIR)$(PKGLIBDIR)/db2_fdw.so
	install -m 644 db2_fdw.control $(DESTDIR)$(SHAREDIR)/extension/
	install -m 644 sql/$(EXTENSION)--$(EXTVERSION).sql $(DESTDIR)$(SHAREDIR)/extension/
	@echo "Installation complete!"

# Uninstall the extension
uninstall:
	@echo "Uninstalling db2_fdw..."
	rm -f $(DESTDIR)$(PKGLIBDIR)/db2_fdw.so
	rm -f $(DESTDIR)$(SHAREDIR)/extension/db2_fdw.control
	rm -f $(DESTDIR)$(SHAREDIR)/extension/db2_fdw--*.sql
	@echo "Uninstallation complete!"

# Clean build artifacts
clean:
	$(CARGO) clean

# Check code without building
check:
	$(CARGO) check --all-features

# Format code
fmt:
	$(CARGO) fmt --all

# Check formatting
fmt-check:
	$(CARGO) fmt --all -- --check

# Run clippy lints
clippy:
	$(CARGO) clippy --all-features -- -D warnings

# Generate documentation
doc:
	$(CARGO) doc --no-deps --all-features

# Development setup
dev-setup:
	cargo install cargo-pgrx --locked
	cargo pgrx init

# Help
help:
	@echo "DB2 FDW Rust Build System"
	@echo ""
	@echo "Usage: make [target]"
	@echo ""
	@echo "Targets:"
	@echo "  all       - Build the extension (default)"
	@echo "  build     - Build the extension"
	@echo "  test      - Run unit tests"
	@echo "  test-pg   - Run PostgreSQL integration tests"
	@echo "  install   - Install the extension"
	@echo "  uninstall - Uninstall the extension"
	@echo "  clean     - Remove build artifacts"
	@echo "  check     - Check code without building"
	@echo "  fmt       - Format code"
	@echo "  clippy    - Run lints"
	@echo "  doc       - Generate documentation"
	@echo "  help      - Show this help"
	@echo ""
	@echo "Environment variables:"
	@echo "  PG_CONFIG   - Path to pg_config (default: pg_config)"
	@echo "  CARGO       - Path to cargo (default: cargo)"
	@echo "  CARGO_FLAGS - Additional cargo flags (default: --release)"
