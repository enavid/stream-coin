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

# Run unit tests (engine lib + cli bin)
test:
    cargo test --lib --bins

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
    cargo test --lib --bins
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

# Run the CLI (debug)
sc *args:
    cargo run --bin sc -- {{args}}

# Build release .deb for server
deb-server:
    cargo deb -p stream-coin

# Build release .deb for CLI
deb-sc:
    cargo deb -p sc

# Security audit
audit:
    cargo audit

# Remove build artifacts
clean:
    cargo clean

# UI (Dioxus, separate workspace in ui/)

# Run the web UI in dev mode (hot reload)
ui-dev:
    cd ui && dx serve --platform web --package web

# Build the web UI for production (static assets in ui/target/dx/web/release/web/public)
ui-build-web:
    cd ui && dx build --platform web --package web --release

# Build an Android APK. Not wired up yet: add a `ui/mobile` package (same
# pattern as `ui/web`, depending on ui_core, with `dioxus/mobile` enabled)
# then point this at it — ui_core's components/state/protocol are already
# platform-agnostic and need no changes.
ui-build-android:
    cd ui && dx build --platform android --package mobile --release

# Run ui_core's unit tests (pure logic: format, state, protocol)
ui-test:
    cd ui && cargo test -p ui_core

# Lint the UI workspace for both native and wasm32 targets
ui-lint:
    cd ui && cargo clippy -p ui_core -p web --target wasm32-unknown-unknown -- -D warnings
