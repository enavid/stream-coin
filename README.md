# stream-coin

> Real-time arbitrage engine for Iranian cryptocurrency exchanges.

[![Rust](https://img.shields.io/badge/rust-%23000000.svg?style=flat-square&logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![Actix Web](https://img.shields.io/badge/actix--web-000?style=flat-square&logo=rust&logoColor=white)](https://actix.rs/)
[![Kafka](https://img.shields.io/badge/Apache%20Kafka-000?style=flat-square&logo=apachekafka)](https://kafka.apache.org/)
[![PostgreSQL](https://img.shields.io/badge/postgres-%23316192.svg?style=flat-square&logo=postgresql&logoColor=white)](https://www.postgresql.org/)
[![Redis](https://img.shields.io/badge/redis-%23DD0031.svg?style=flat-square&logo=redis&logoColor=white)](https://redis.io/)
[![Dioxus](https://img.shields.io/badge/dioxus-0.7-blue?style=flat-square)](https://dioxuslabs.com/)
[![License: GPL v3](https://img.shields.io/badge/license-GPLv3-blue?style=flat-square)](LICENSE)

Three independently buildable projects in one repository: an async Rust
engine that streams live prices from exchange WebSockets into Kafka and a
real-time WS feed, a CLI (`sc`) to control it, and a Dioxus web UI to watch
it.

## How it works

Each exchange adapter parses its own WebSocket stream into a common `Price`
type. Every tick is published to Kafka (for downstream arbitrage detection)
and broadcast over WebSocket to connected clients at the same time. The CLI
and UI both talk to the same REST control plane to start/stop/list tickers.

See [ARCHITECTURE.md](ARCHITECTURE.md) for the full data-flow diagram.

```
stream-coin/
├── engine/   # the server — actix-web, exchange adapters, Postgres, Kafka, Redis, WS feed
├── cli/      # `sc` — controls the engine over REST, zero dependency on engine
└── ui/       # Dioxus SPA — shared core + a web launcher, consumes the WS feed
```

Persistent state (users, exchanges/pairs, candles, strategies, orders,
subscriptions) lives in PostgreSQL; Redis is an optional ticker cache, and
Kafka carries the price/candle/signal streams for downstream processing.

## Supported exchanges

| Exchange | Price feed (WS) | Order placement |
|---|---|---|
| Tabdeal | ✅ | ✅ |
| Hitobit | ✅ | ✅ |
| CoinEx | ✅ (seeded disabled) | — |
| Exir | — | ✅ |

New price feeds are added by implementing one trait (`ExchangeAdapter`); new
order venues implement `OrderAdapter`. Nothing else in the engine changes.

## Quick start

**Infra** (Postgres, Redis, Kafka, Kafka UI):

```bash
cp .env.example .env   # fill in passwords (POSTGRES_PASSWORD is required)
docker compose up -d
```

The engine applies its SQL migrations to Postgres automatically on startup.

**Engine:**

```bash
just run               # cargo run --bin stream-coin
```

**CLI:**

```bash
just sc auth login                         # authenticate first (saves a token)
just sc ticker start tabdeal USDT/IRT      # exchange and pair are positional
just sc ticker list
```

**UI** (dev server with hot reload):

```bash
just ui-dev
```

## Development

This project uses [`just`](https://github.com/casey/just) as its task runner.

```bash
just check    # fmt + clippy -D warnings + unit tests + integration tests
just test     # unit tests only
just sc --help
```

The `ui/` workspace has its own test/lint targets: `cd ui && just ui-test` /
`ui-lint`.

## Releases

Tagged pushes (`vX.Y.Z`) trigger a CI pipeline that builds and publishes,
independently: the engine (`stream-coin`, tar/deb/rpm/msi/AppImage + Docker
image), the CLI (`sc`, tar/deb/rpm), and the web UI (static bundle) — see
[Releases](../../releases).

## Roadmap

Outstanding standardization work is tracked in [ROADMAP.md](ROADMAP.md).

## Contributing

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/amazing-feature`)
3. `just check` before opening a PR
4. Open a Pull Request

## License

GPL-3.0 — see [LICENSE](LICENSE).
