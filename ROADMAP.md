# stream-coin roadmap

Tracks the standardization work agreed on 2026-06-18: separability of the three
sub-projects (engine, CLI, UI), test depth, comment hygiene, and CI/release
artifact separation.

## 1. Workspace split — engine / cli / ui

Today `engine/bin/http.rs` (engine) and `cli/src/main.rs` (CLI) are independent
crates in a Cargo workspace, each with its own `Cargo.toml`/dependencies; `ui/`
was already separate. `cli/*` never imports engine code.

## 2. Comment hygiene

Remove decorative `── ... ──` separator lines and Persian text from code comments.

## 3. Strengthen tests with challenging edge cases

Current coverage is mostly happy-path. Add cases for:

- WS reconnect/drop mid-stream in `TabdealWsAdapter`
- `broadcast::Receiver` under `RecvError::Lagged` (slow WS client)
- Redis/Kafka failure mid-operation (register/publish) — does the handler fail soft?
- Concurrent start of the same exchange+pair ticker (race on `clients` map)
- Malformed/partial JSON at every boundary (REST body, Kafka payload, WS frame)
- Boundary numeric values in `Price`/`format_price` (zero, negative, very large)

## 4. CI / release — separate, independently downloadable artifacts

`ci.yml` only builds one Docker image (engine). `release.yml` only packages
the `stream-coin` binary (tar/deb/rpm/msi/AppImage) — `sc` is never built or
released standalone, and the UI is absent from CI entirely.

Still needed:

- per-target build + upload for `sc` (mirroring the existing `stream-coin` matrix)
- a job that runs `dx build --platform web --package web --release` in `ui/`
  and uploads the static bundle (`ui/target/dx/web/release/web/public`) as
  its own release asset
- `release.yml`'s `build-deb`/`build-rpm` jobs call `cargo deb`/`cargo generate-rpm`
  without `-p`, which is now ambiguous in a multi-package workspace — needs
  `-p stream-coin` / `-p sc` variants for each binary

## Status

- [x] 1. Workspace split — `engine/` and `cli/` are independent crates in a
      Cargo workspace; `ui/` was already separate. Verified: `cargo build`,
      `cargo test --lib --bins`, `cargo test --tests`,
      `cargo clippy --all-targets -- -D warnings`, `docker build` all pass.
      `Dockerfile` updated for the per-member dummy-build cache trick (also
      fixed a pre-existing, unrelated bug: missing `cmake`/`build-essential`/
      `libcurl4-openssl-dev` for `rdkafka`'s `cmake-build` feature).
      `justfile`'s `deb-server`/`deb-sc` now use `cargo deb -p <pkg>`.
- [x] 2. Comment hygiene — removed all `── ... ──` separator lines
      (`justfile`, `engine/tests/ticker_api.rs`,
      `engine/src/presentation/handlers/exchange_handler.rs`).
- [ ] 3. Test depth — not started, beyond two fixes made opportunistically:
      (a) `engine/tests/kafka_producer.rs`'s two tests need a live Kafka
      broker on `localhost:9092` and were never going to pass in CI or a
      clean checkout — marked `#[ignore]` with a clear reason instead of
      silently failing `just check`; (b) `cli/src/config.rs`'s tests that
      mutate `SC_CONFIG_PATH` raced under parallel test execution — fixed
      with a `Mutex` guard.
- [ ] 4. CI/release separation — partially done: `ci.yml` and `release.yml`
      now install `cmake`/`build-essential`/`libcurl4-openssl-dev` (Linux),
      `brew install cmake` (macOS) wherever `cargo build`/`test`/`clippy`
      runs, fixing the production CI failure where `rdkafka-sys`'s
      `cmake-build` feature couldn't find `curl/curl.h` on the GitHub-hosted
      runner. `justfile`'s `check` now also runs `check-clean-env` (a Docker
      build using the project's own `Dockerfile`, with no host-installed
      system libs) specifically so missing-system-dependency bugs like this
      one are caught locally before push, not just in CI. The deeper
      per-artifact CI/release split (separate sc/ui jobs, fixing
      `cargo deb`/`cargo generate-rpm` ambiguity) is still open — see above.
      Windows (`build-msi` job) was not touched: rdkafka's cmake-build on
      Windows likely needs a different fix (vcpkg) and wasn't verified.
