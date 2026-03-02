#!/usr/bin/env bash
# VideoLAN Audio Streamer - Test Runner Script
# Quick commands for running tests

set -e

echo "🧪 VideoLAN Test Runner"
echo "======================="
echo ""

# Function to print colored output
print_success() {
    echo "✅ $1"
}

print_info() {
    echo "ℹ️  $1"
}

# Check if an argument is provided
case "${1:-all}" in
    "all"|"")
        print_info "Running all tests..."
        cargo test
        ;;
    
    "unit")
        print_info "Running unit tests only..."
        cargo test --lib
        ;;
    
    "integration")
        print_info "Running integration tests only..."
        cargo test --test integration_tests
        ;;
    
    "network")
        print_info "Running network module tests..."
        cargo test --lib network
        ;;
    
    "audio")
        print_info "Running audio module tests..."
        cargo test --lib audio
        ;;
    
    "ui")
        print_info "Running UI module tests..."
        cargo test --lib ui
        ;;
    
    "quick")
        print_info "Running quick test (unit tests only, single thread)..."
        cargo test --lib -- --test-threads=1
        ;;
    
    "verbose")
        print_info "Running tests with output..."
        cargo test -- --nocapture
        ;;
    
    "watch")
        print_info "Running tests in watch mode (requires cargo-watch)..."
        if command -v cargo-watch &> /dev/null; then
            cargo watch -x test
        else
            echo "cargo-watch not installed. Install with: cargo install cargo-watch"
            exit 1
        fi
        ;;
    
    "coverage")
        print_info "Running tests with coverage (requires tarpaulin)..."
        if command -v cargo-tarpaulin &> /dev/null; then
            cargo tarpaulin --out Html --output-dir coverage
            print_success "Coverage report generated in coverage/"
        else
            echo "tarpaulin not installed. Install with: cargo install cargo-tarpaulin"
            exit 1
        fi
        ;;
    
    "bench")
        print_info "Running benchmarks..."
        cargo bench
        ;;
    
    "clean")
        print_info "Cleaning test artifacts..."
        cargo clean
        print_success "Clean complete"
        ;;
    
    "help"|"-h"|"--help")
        echo "Usage: ./run_tests.sh [command]"
        echo ""
        echo "Commands:"
        echo "  all          Run all tests (default)"
        echo "  unit         Run unit tests only"
        echo "  integration  Run integration tests only"
        echo "  network      Run network module tests"
        echo "  audio        Run audio module tests"
        echo "  ui           Run UI module tests"
        echo "  quick        Run quick tests (single thread)"
        echo "  verbose      Run tests with output"
        echo "  watch        Run tests in watch mode (auto-rerun)"
        echo "  coverage     Generate test coverage report"
        echo "  bench        Run benchmarks"
        echo "  clean        Clean test artifacts"
        echo "  help         Show this help message"
        echo ""
        echo "Examples:"
        echo "  ./run_tests.sh              # Run all tests"
        echo "  ./run_tests.sh unit         # Run only unit tests"
        echo "  ./run_tests.sh watch         # Watch mode"
        ;;
    
    *)
        echo "❌ Unknown command: $1"
        echo "Run './run_tests.sh help' for usage information"
        exit 1
        ;;
esac

print_success "Done!"
