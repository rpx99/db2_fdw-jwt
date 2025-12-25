#!/bin/bash
# DB2 FDW Docker Test Script
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
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

# Parse arguments
ACTION=${1:-"start"}

case $ACTION in
    start)
        log_info "Starting DB2 FDW test environment..."
        docker compose up -d

        log_info "Waiting for DB2 to initialize (this may take 2-3 minutes)..."
        docker compose logs -f db2 &
        LOG_PID=$!

        # Wait for DB2 to be healthy
        until docker compose exec -T db2 su - db2inst1 -c "db2 connect to testdb" 2>/dev/null; do
            sleep 10
            log_info "Still waiting for DB2..."
        done

        kill $LOG_PID 2>/dev/null || true

        log_info "DB2 is ready!"
        log_info "Running DB2 init script..."
        docker compose exec -T db2 su - db2inst1 -c "db2 -tvf /var/custom/init.sql" || true

        log_info "Environment is ready. Run './test.sh test' to run tests."
        ;;

    test)
        log_info "Running FDW tests..."

        # Test basic connectivity
        log_info "Testing PostgreSQL connectivity..."
        docker compose exec -T postgres psql -U postgres -d testdb -c "SELECT 1 AS test;"

        # Check if extension is loaded
        log_info "Checking db2_fdw extension..."
        docker compose exec -T postgres psql -U postgres -d testdb -c "SELECT * FROM pg_extension WHERE extname = 'db2_fdw';"

        # List foreign tables
        log_info "Listing foreign tables..."
        docker compose exec -T postgres psql -U postgres -d testdb -c "SELECT * FROM information_schema.foreign_tables;"

        # Query employees table
        log_info "Querying employees from DB2..."
        docker compose exec -T postgres psql -U postgres -d testdb -c "SELECT * FROM employees LIMIT 5;"

        log_info "Tests completed!"
        ;;

    logs)
        docker compose logs -f
        ;;

    stop)
        log_info "Stopping containers..."
        docker compose down
        ;;

    clean)
        log_info "Stopping and removing all data..."
        docker compose down -v
        ;;

    rebuild)
        log_info "Rebuilding PostgreSQL image..."
        docker compose build --no-cache postgres
        ;;

    psql)
        docker compose exec postgres psql -U postgres -d testdb
        ;;

    db2)
        docker compose exec db2 su - db2inst1
        ;;

    *)
        echo "Usage: $0 {start|test|logs|stop|clean|rebuild|psql|db2}"
        echo ""
        echo "Commands:"
        echo "  start   - Start the Docker environment"
        echo "  test    - Run FDW tests"
        echo "  logs    - Follow container logs"
        echo "  stop    - Stop containers"
        echo "  clean   - Stop and remove all data"
        echo "  rebuild - Rebuild PostgreSQL image"
        echo "  psql    - Connect to PostgreSQL"
        echo "  db2     - Connect to DB2 as db2inst1"
        exit 1
        ;;
esac
