#!/bin/bash
# DB2 FDW Rust Docker Test Script
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

log_info() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

log_header() {
    echo -e "\n${BLUE}=== $1 ===${NC}\n"
}

# Parse arguments
ACTION=${1:-"help"}

case $ACTION in
    build)
        log_header "Building Complete Rust FDW"
        docker compose -f docker-compose.rust.yml build --no-cache postgres
        log_info "Build complete!"
        ;;

    start)
        log_header "Starting Complete Rust FDW Environment"
        docker compose -f docker-compose.rust.yml up -d

        log_info "Waiting for DB2 to initialize (this may take 3-5 minutes)..."

        # Wait for DB2 to be healthy
        RETRIES=0
        MAX_RETRIES=30
        until docker compose -f docker-compose.rust.yml exec -T db2 su - db2inst1 -c "db2 connect to testdb" 2>/dev/null; do
            RETRIES=$((RETRIES + 1))
            if [ $RETRIES -ge $MAX_RETRIES ]; then
                log_error "DB2 failed to start after $MAX_RETRIES attempts"
                exit 1
            fi
            log_info "Still waiting for DB2... (attempt $RETRIES/$MAX_RETRIES)"
            sleep 10
        done

        log_info "DB2 is ready!"
        log_info "Running DB2 init script..."
        docker compose -f docker-compose.rust.yml exec -T db2 su - db2inst1 -c "db2 -tvf /var/custom/init.sql" || true

        log_info "Environment is ready. Run './test-rust.sh test' to run tests."
        ;;

    test)
        log_header "Running Rust FDW Tests"

        # Test PostgreSQL connectivity
        log_info "Testing PostgreSQL connectivity..."
        docker compose -f docker-compose.rust.yml exec -T postgres psql -U postgres -d testdb -c "SELECT 1 AS test;"

        # Check if extension is loaded
        log_info "Checking db2_fdw extension..."
        docker compose -f docker-compose.rust.yml exec -T postgres psql -U postgres -d testdb -c "SELECT * FROM pg_extension WHERE extname = 'db2_fdw';"

        # Check db2_diag
        log_info "Running db2_diag()..."
        docker compose -f docker-compose.rust.yml exec -T postgres psql -U postgres -d testdb -c "SELECT * FROM db2_diag();"

        # List foreign tables
        log_info "Listing foreign tables..."
        docker compose -f docker-compose.rust.yml exec -T postgres psql -U postgres -d testdb -c "SELECT * FROM information_schema.foreign_tables;"

        # Query employees table
        log_info "Querying employees from DB2..."
        docker compose -f docker-compose.rust.yml exec -T postgres psql -U postgres -d testdb -c "SELECT * FROM employees LIMIT 5;"

        log_info "All tests passed!"
        ;;

    logs)
        docker compose -f docker-compose.rust.yml logs -f
        ;;

    logs-postgres)
        docker compose -f docker-compose.rust.yml logs -f postgres
        ;;

    logs-db2)
        docker compose -f docker-compose.rust.yml logs -f db2
        ;;

    stop)
        log_info "Stopping containers..."
        docker compose -f docker-compose.rust.yml down
        ;;

    clean)
        log_info "Stopping and removing all data..."
        docker compose -f docker-compose.rust.yml down -v
        ;;

    psql)
        docker compose -f docker-compose.rust.yml exec postgres psql -U postgres -d testdb
        ;;

    db2)
        docker compose -f docker-compose.rust.yml exec db2 su - db2inst1
        ;;

    shell)
        docker compose -f docker-compose.rust.yml exec postgres bash
        ;;

    *)
        echo "Usage: $0 {build|start|test|logs|stop|clean|psql|db2|shell}"
        echo ""
        echo "Commands:"
        echo "  build        - Build PostgreSQL image with Rust FDW"
        echo "  start        - Start the Docker environment"
        echo "  test         - Run FDW tests"
        echo "  logs         - Follow all container logs"
        echo "  logs-postgres- Follow PostgreSQL logs"
        echo "  logs-db2     - Follow DB2 logs"
        echo "  stop         - Stop containers"
        echo "  clean        - Stop and remove all data"
        echo "  psql         - Connect to PostgreSQL"
        echo "  db2          - Connect to DB2 as db2inst1"
        echo "  shell        - Open shell in PostgreSQL container"
        echo ""
        echo "Example workflow:"
        echo "  $0 build     # Build the Rust FDW image"
        echo "  $0 start     # Start PostgreSQL + DB2"
        echo "  $0 test      # Run tests"
        echo "  $0 psql      # Interactive SQL"
        echo "  $0 clean     # Cleanup"
        exit 1
        ;;
esac
