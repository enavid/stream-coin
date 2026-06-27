# stream-coin Architecture

Arbitrage engine for Iranian cryptocurrency exchanges. One backend (Rust/actix-web),
fed by per-exchange WebSocket adapters, persisting to Postgres/TimescaleDB and
streaming to Kafka, exposing a JWT-secured REST control-plane + a real-time
WebSocket feed, consumed by a CLI (`sc`) and a cross-platform UI (Dioxus).

## System diagram

![Architecture diagram](docs/architecture-diagram.png)

<details>
<summary>Mermaid source</summary>

```mermaid
graph TB
    subgraph EX["Exchanges"]
        TABDEAL["Tabdeal WS API"]
        HITOBIT["Hitobit WS API"]
        COINEX["CoinEx WS API"]
        FUTURE_EX["more exchanges later\n(Nobitex, ...)"]
    end

    subgraph SERVER["stream-coin server  (engine/bin/http.rs, actix-web)"]
        ADAPTER["ExchangeAdapter\nTabdeal / Hitobit / CoinEx WsAdapter"]
        HANDLER["exchange_handler\nstart / stop / list ticker"]
        STATE["AppState\nclients map · broadcaster · publisher"]
        WSH["ws_handler\nGET /v1/ws"]
        PG[("Postgres / TimescaleDB\nusers · exchanges · pairs · candles\nstrategies · orders · subscriptions")]
        REDIS[("Redis\nTickerRepository (cache)")]
    end

    KAFKA[("Kafka\nprices (JSON) · candles.proto · signals.proto")]
    FLINK["Flink\n(future stream processing /\narbitrage detection)"]

    subgraph CLI["sc  (cli/src/main.rs)"]
        CLI_AUTH["auth"]
        CLI_TICKER["ticker start/stop/list"]
        CLI_CONFIG["config"]
    end

    subgraph UI["ui/  (Dioxus workspace)"]
        subgraph CORE["ui_core — shared, platform-agnostic"]
            DOMAIN["domain\nformat, Ticker, Direction"]
            PROTO["protocol\nPriceMessage (mirrors backend JSON)"]
            STORE["state\nTickerStore (pure) + AppState (signals)"]
            API["api\nApiClient (REST, reqwest)"]
            COMPONENTS["components\nHeader · TickerCard · TickerGrid\nAddTickerForm · LiveFeed"]
            DASH["dashboard\nDashboard (composition root)"]
        end
        subgraph WEBCRATE["web — thin launcher (wasm32)"]
            WMAIN["main.rs\nprovide_app_state + render Dashboard"]
            WWS["ws.rs\ngloo-net WebSocket client"]
        end
        FUTURE_UI["mobile/ · desktop/ (future)\nsame pattern, native WS client,\nzero changes to ui_core"]
    end

    TABDEAL -->|depth stream| ADAPTER
    HITOBIT -->|depth stream| ADAPTER
    COINEX -->|depth stream| ADAPTER
    FUTURE_EX -.->|same ExchangeAdapter trait| ADAPTER
    ADAPTER -->|Price| HANDLER
    HANDLER -->|prices JSON · candles/signals protobuf| KAFKA
    HANDLER -->|serialize + broadcast| STATE
    HANDLER <-->|register/lookup ticker| REDIS
    STATE <-->|repositories: load/persist| PG
    STATE --> WSH
    KAFKA -.->|future| FLINK

    CLI_TICKER -->|REST POST /start /stop\nGET /tickers| HANDLER
    CLI_AUTH -->|REST POST /v1/auth/token| HANDLER

    WSH ===|"WS  ws://host/v1/ws\nJSON PriceMessage"| WWS
    WWS -->|parse + apply| STORE
    WMAIN -->|REST start/stop ticker| HANDLER
    STORE --> DASH
    DASH --> COMPONENTS
    WMAIN --> DASH
    CORE -.->|reused unchanged by| FUTURE_UI
```

</details>

## Components

| Layer | Crate / module | Responsibility |
|---|---|---|
| Exchange adapters | `engine/src/exchange/*` | One adapter per exchange implementing `ExchangeAdapter`; parses exchange-specific WS messages into the shared `Price` type. |
| Control plane | `engine/src/presentation/handlers/exchange_handler.rs` | REST: start/stop/list tickers. On every price tick: publishes to Kafka **and** broadcasts to WS clients; aggregates closed candles (persisted to Postgres + published as Protobuf). |
| Real-time feed | `engine/src/presentation/handlers/ws_handler.rs` | `GET /v1/ws` — upgrades to WebSocket, forwards every `AppState::broadcaster` message to the client (audience-routed: public vs per-user); answers ping/close. |
| Shared state | `engine/src/presentation/shared/app_state.rs` | `clients` (running ticker handles), `broadcaster` (`tokio::broadcast`), `publisher` (Kafka), and the repository handles (Postgres-backed) for users, exchanges/pairs, candles, strategies, orders, subscriptions, credentials; `ticker_repository` (Redis). |
| Persistence | `engine/src/infrastructure/db/*` (`postgres.rs`) | Postgres/TimescaleDB-backed repositories — the primary store for users/RBAC, exchanges/pairs/assets, candles (hypertable), strategies, orders, subscriptions, encrypted credentials. Schema in `engine/migrations/`, applied on startup; see `docs/database-schema.md`. |
| Cache | `engine/src/infrastructure/cache/*` | Redis-backed `TickerRepository` (optional ticker cache). |
| Messaging | `engine/src/kafka/*` | `MessagePublisher` port + `KafkaProducer` adapter (rdkafka). Topics: `prices` (JSON), `candles.proto` and `signals.proto` (Protobuf). |
| CLI | `cli/src/*` | `sc auth/ticker/config` — talks to the same REST control plane as the UI; zero dependency on `engine`. |
| UI shared core | `ui/ui_core/*` | Domain logic, wire protocol, reactive state, REST client, all Dioxus components, and the `Dashboard` composition — platform-agnostic, unit tested without WASM. |
| UI web launcher | `ui/web/*` | Thin binary: provides `AppState`, runs the `gloo-net` WebSocket client (`ws.rs`), renders `Dashboard`. |
| UI future platforms | `ui/mobile/`, `ui/desktop/` (not yet created) | Same pattern as `ui/web`: own `Cargo.toml`, own native WS transport, depends on `ui_core`, zero changes to shared code. |

## Data flow (one price tick)

1. A `WsAdapter` (Tabdeal/Hitobit/CoinEx) reads a depth message from the exchange's WebSocket and parses it into a `Price` (shared decimal parsing in `engine/src/exchange/mod.rs`).
2. `exchange_handler` receives the `Price` over an internal `mpsc` channel.
3. It serializes the price once to JSON and:
   - publishes that JSON to the `prices` Kafka topic (for downstream Flink/arbitrage processing), and
   - sends the same JSON on `AppState::broadcaster` (a `tokio::sync::broadcast` channel) — the price WS and Kafka payloads are byte-identical.
4. The same handler feeds the tick to per-interval candle aggregators; each closed candle is persisted to Postgres, published to `candles.proto` as Protobuf, and broadcast to the WS as JSON.
5. Every connected `ws_handler` session receives broadcasts for its audience (public ticks, or per-user order updates) and forwards them as WS text frames.
6. The UI's `ws.rs` (web) parses each message, calls the matching `AppState::apply_*`, which updates the relevant store — Dioxus signals re-render the affected components automatically.

## Extension points

- **New exchange**: implement `ExchangeAdapter` for it (see `engine/src/exchange/tabdeal/`), register it in `engine/bin/http.rs`'s adapter map. Nothing else changes.
- **New UI platform** (Android/desktop): add `ui/mobile/` (or `desktop/`) with its own `Cargo.toml` + native WebSocket transport module (mirroring `ui/web/src/ws.rs`), depend on `ui_core`, call `provide_app_state()` + render `Dashboard`. No changes to `ui_core`.
- **Arbitrage detection**: consume the `prices` Kafka topic (directly or via Flink), compare prices across exchanges for the same pair, publish opportunities to a new topic/endpoint.
- **Auth**: `sc auth` and the REST handlers already have the shape for it (`AppState`, CLI `auth` module); wiring a real device-flow/JWT is additive, not a redesign.
