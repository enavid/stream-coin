# stream-coin roadmap — Algorithmic Trading Platform

Last updated: 2026-06-27.

---

## Backlog

| # | Loop | Task | Status |
|---|------|------|--------|
| 1 | sec-1b | Money-safety: HMAC signing, exact partial-fill qty, per-user/persisted positions | In progress |
| 2 | 5c-2 | Stateless multi-instance: all shared state in Postgres + Kafka | Not started |

---

### Loop sec-1b — Money-safety (remaining)

Continues the 2026-06-27 code-review remediation (`docs/code-review-2026-06-27.md`):

- ~~**C8** — reconcile via `get_order_status_by_client_id` before marking an order
  `failed` on a transient placement failure~~ **(done)** — a live position is no
  longer silently orphaned on a timeout/5xx.
- **C10** — per-exchange HMAC request signing (needs `api_secret` in the credential schema).
- **M7** — exact partial-fill quantity (needs `OrderStatusResult.filled_quantity` on the port).
- **M8 / M11** — per-user position scoping + Postgres order persistence (overlap with Loop 5c-2).

---

### Loop 5c-2 — Stateless multi-instance

All shared state in Postgres + Kafka so multiple engine instances (price-feed / order /
strategy / API+WS workers) run simultaneously sharing one DB + Kafka.

Tests: `two_engine_instances_do_not_duplicate_orders`.

---

## Future (not yet planned)

- **Flink for candle aggregation** — standalone Flink job on `prices → candles` when exchanges ≥5.
- **ClickHouse** — when backtest queries over months of candles outgrow TimescaleDB.
- **Risk dashboard** — real-time per-user P&L, drawdown alerts, daily loss limits.
- **Strategy marketplace** — share/version strategies, rated by backtest signal accuracy.
