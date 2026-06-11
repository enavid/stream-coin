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

# Run all tests
test:
    cargo test --lib

# Full quality cycle: fmt → lint → test
check:
    @echo "→ Formatting..."
    cargo fmt
    @echo "→ Linting..."
    cargo clippy -- -D warnings
    @echo "→ Testing..."
    cargo test --lib
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
