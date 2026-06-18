# stream-coin

Arbitrage engine for Iranian crypto exchanges — three independently separable
projects in one repo: `engine` (server), `cli` (`sc`), `ui` (Dioxus SPA).

## Session Rules

- Never write Persian text or decorative separator lines (`── ─── ──`) in code comments.
- Never use `sed`/`awk` for multi-line struct or import edits — use the Edit tool; sed has silently corrupted files here before (inserted literal `\n`, or no-op'd).
- IMPORTANT: TDD is non-negotiable — write the failing test before the implementation, every time.
- Only say a change is "ready to commit" after the full quality cycle (see Workflow) passes clean.

## Stack

| Layer | Tech |
|---|---|
| engine | Rust, actix-web, tokio, Redis (cache), Kafka (`rdkafka`) |
| cli | Rust, clap, reqwest — zero dependency on `engine` |
| ui | Dioxus 0.7 (Web/Android), separate Cargo workspace in `ui/` |
| Real-time | WebSocket: engine broadcasts → `ui/web` consumes via `gloo-net` |

## Commands

- `just check` — full cycle: fmt → clippy -D warnings → unit tests → integration tests. Run before declaring any task done.
- `just test` — unit tests only (`cargo test --lib --bins`; plain `cargo test` misses bin-crate tests in `cli`).
- `just test-integration` — `cargo test --tests`.
- `just run` / `just sc <args>` — run the engine / CLI locally.
- `cd ui && just ui-test` / `ui-lint` — UI workspace has its own test/lint targets (wasm32 target for lint).

## Workflow

**New feature / change:**
1. Write the failing test first.
2. Implement until it passes.
3. `just check`. Fix anything red before moving on.
4. Tell the user it's ready to commit — never commit unprompted.

**Bug fix:**
1. Reproduce with a new test that fails for the right reason.
2. Fix the root cause, not the symptom.
3. `just check`.

## Hard Rules

- IMPORTANT: `cli/` must never `use` anything from `engine`'s crate — that coupling is the one thing this workspace split exists to prevent.
- Any new exchange integration implements `ExchangeAdapter` (`engine/src/exchange/port.rs`) — don't special-case a new exchange elsewhere.
- `ui_core` stays platform-agnostic (no `gloo-net`/wasm-only deps) — platform crates (`ui/web`, future `ui/mobile`) own the transport, see @ui/ui_core/src/state/mod.rs.
- Backend WS broadcast payload and Kafka payload must stay byte-identical (computed once in `exchange_handler.rs`, sent twice) — don't fork the serialization.
- Don't run `cargo deb`/`cargo generate-rpm`/etc. without `-p <package>` — this is now a multi-package workspace, not a single crate.

## Key Files

| File | Purpose |
|---|---|
| `Cargo.toml` | workspace root — members: `engine`, `cli` |
| `engine/src/lib.rs` | engine module map |
| `engine/src/presentation/shared/app_state.rs` | shared server state: clients, broadcaster, publisher |
| `cli/src/main.rs` | CLI entry point and subcommand wiring |
| `ui/ui_core/src/dashboard.rs` | UI composition root (shared across all platforms) |
| `justfile` | all task-runner commands (engine/cli); `ui/` has its own |

## Extended Docs

- System architecture + diagram: @ARCHITECTURE.md — read when touching cross-component data flow (exchange → engine → Kafka/WS → CLI/UI).
- Outstanding standardization work: @ROADMAP.md — read before starting any task to check if it's already tracked there.
