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

# Run linter (warnings = errors, all targets including test code)
lint:
    cargo clippy --all-targets -- -D warnings

# Run unit tests (engine lib + cli bin)
test:
    cargo test --lib --bins

# Run integration tests
test-integration:
    cargo test --tests

# Build the engine in a clean container with no host-installed system libs.
# Catches missing system deps (e.g. rdkafka's cmake/curl headers) that a
# long-lived dev machine already has installed and CI runners don't.
check-clean-env:
    docker build --target builder -t stream-coin-check .

# Full quality cycle: fmt → lint → unit tests → integration tests
check:
    @echo "→ Formatting..."
    cargo fmt
    @echo "→ Linting..."
    cargo clippy --all-targets -- -D warnings
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

# Run Python SDK unit tests (betterproto + consumer decode, no Kafka required)
python-sdk-test:
    cd python_sdk && .venv/bin/python3 -m pytest tests/ -v

# Security audit
audit:
    cargo audit

# Remove build artifacts
clean:
    cargo clean

# Start all infrastructure services (Redis, Kafka, Schema Registry, Flink, UI, Nginx)
docker-up:
    docker compose up -d

# Stop all infrastructure services and remove containers
docker-down:
    docker compose down

# UI (Dioxus, separate workspace in ui/)

# Run the web UI in dev mode (hot reload).
# Pinned to a fixed port (rather than dx's default random one) so it
# matches CORS_ALLOWED_ORIGINS in .env — see engine/src/presentation/
# middlewares/cors.rs.
ui-dev:
    cd ui && dx serve --platform web --package web --port 8081

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
