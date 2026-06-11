# stream-coin task runner

# Show all available commands
default:
    @just --list

# Format code
fmt:
    cargo fmt

# Check formatting without modifying
fmt-check:
    cargo fmt --check

# Run linter (warnings = errors)
lint:
    cargo clippy -- -D warnings

# Run unit tests
test:
    cargo test --lib

# Run integration tests
test-integration:
    cargo test --tests

# Full quality cycle: fmt → lint → unit tests → integration tests
check:
    @echo "→ Formatting..."
    cargo fmt
    @echo "→ Linting..."
    cargo clippy -- -D warnings
    @echo "→ Testing (unit)..."
    cargo test --lib
    @echo "→ Testing (integration)..."
    cargo test --tests
    @echo "✓ All checks passed"

# Build debug
build:
    cargo build

# Build release
build-release:
    cargo build --release

# Run the server
run:
    cargo run --bin stream-coin

# Security audit
audit:
    cargo audit

# Remove build artifacts
clean:
    cargo clean
