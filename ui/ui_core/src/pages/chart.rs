//! TradingView-style candlestick chart page. Renders via TradingView's own
//! open-source `lightweight-charts` JS library (vendored into
//! `ui/web/assets/`, loaded by `ui/web/src/main.rs` as a Dioxus asset) — no
//! Dioxus-native candlestick component exists, and hand-rolling OHLCV canvas
//! rendering isn't worth it when the reference implementation is free and
//! purpose-built. `document::eval` is the only JS-interop mechanism Dioxus
//! 0.7 offers without a hand-written `wasm-bindgen` extern module, so the
//! glue below is plain JS strings.

use dioxus::prelude::*;

mod trade_format;

use super::current_token;
use crate::api::ApiClient;
use crate::domain::format::format_price;
use crate::icons::{
    IconArrowDown, IconArrowUp, IconCursor, IconFibonacci, IconHorizontalLine, IconRectangle,
    IconRuler, IconTrash, IconTrendLine, IconVerticalLine,
};
use crate::state::AppState;
use trade_format::format_stats_row;

const CONTAINER_ID: &str = "sc-candlestick-chart";
const LEGEND_ID: &str = "sc-candlestick-chart-legend";
const INTERVALS: &[&str] = &["1m", "5m", "15m", "1h"];
const HISTORY_LIMIT: u32 = 300;

struct DrawTool {
    id: &'static str,
    title: &'static str,
    icon: fn() -> Element,
}

/// Drives the draw-toolbar's button row — same `fn() -> Element` icon
/// convention as `app_shell.rs`'s `NavItem`. "Clear" isn't in this list:
/// it's a one-shot action, not a selectable tool, and is rendered
/// separately after this row.
const DRAW_TOOLS: &[DrawTool] = &[
    DrawTool {
        id: "cursor",
        title: "Cursor",
        icon: || rsx! { IconCursor {} },
    },
    DrawTool {
        id: "trend",
        title: "Trend line",
        icon: || rsx! { IconTrendLine {} },
    },
    DrawTool {
        id: "hline",
        title: "Horizontal line",
        icon: || rsx! { IconHorizontalLine {} },
    },
    DrawTool {
        id: "vline",
        title: "Vertical line",
        icon: || rsx! { IconVerticalLine {} },
    },
    DrawTool {
        id: "rect",
        title: "Rectangle",
        icon: || rsx! { IconRectangle {} },
    },
    DrawTool {
        id: "fib",
        title: "Fibonacci retracement",
        icon: || rsx! { IconFibonacci {} },
    },
    DrawTool {
        id: "buy",
        title: "Buy marker",
        icon: || rsx! { IconArrowUp {} },
    },
    DrawTool {
        id: "sell",
        title: "Sell marker",
        icon: || rsx! { IconArrowDown {} },
    },
    DrawTool {
        id: "measure",
        title: "Price range",
        icon: || rsx! { IconRuler {} },
    },
];

/// Defines `window.scChartInit/SetData/Update/SetTheme`, scoped by a
/// container id so a future multi-chart page wouldn't collide. Idempotent —
/// safe to `eval` again on every mount (e.g. hot reload). Uses
/// `ResizeObserver` rather than a `window` resize listener so the chart also
/// resizes when the sidebar opens/closes or the container's flex layout
/// changes without the viewport itself changing size.
const CHART_GLUE_JS: &str = include_str!("chart/glue.js");

/// Runs `js` and logs to the browser console on failure. Every
/// `document::eval` call on this page used to drop its `Result` with
/// `let _ = ...` — a JS throw inside the glue code (a real bug, or a stale
/// `lightweight-charts` global on a slow load) was therefore completely
/// invisible in production. Logs via a second `eval` round-trip rather than
/// `web_sys::console` directly: `ui_core` stays platform-agnostic (no
/// wasm-only deps), and `document::eval` is already the one bridge this
/// page uses to talk to the browser.
async fn eval_logged(js: &str) {
    if let Err(e) = document::eval(js).await {
        let message = format!("chart.rs: eval failed: {e}");
        if let Ok(json) = serde_json::to_string(&message) {
            let _ = document::eval(&format!("console.error({json})")).await;
        }
    }
}

#[component]
pub fn Chart(server_url: String) -> Element {
    let state = use_context::<AppState>();
    let api = use_signal(|| {
        ApiClient::new(server_url).with_unauthorized_handler(move || {
            let mut state = state;
            state.clear_session();
        })
    });

    let mut exchange_choice = use_signal(String::new);
    let mut pair_choice = use_signal(String::new);
    let mut interval_choice = use_signal(|| INTERVALS[0].to_string());
    let mut load_error = use_signal(|| None::<String>);
    let mut loading = use_signal(|| true);
    let mut chart_ready = use_signal(|| false);
    let mut last_pushed_candle = use_signal(|| None::<crate::state::Candle>);
    let mut symbol_query = use_signal(String::new);
    let mut symbol_dropdown_open = use_signal(|| false);
    let mut drawing_tool = use_signal(|| "cursor".to_string());
    let mut trade_rectangles_on = use_signal(|| true);

    // `use_memo` (not a plain `let` derived from signals) so the values are
    // themselves reactive reads — capturing the plain `String` they'd
    // otherwise produce into a `use_effect` closure does NOT make that
    // effect track exchange/pair/interval changes, since Dioxus only
    // subscribes an effect to the signals it actually reads inside its own
    // closure body. That bug is exactly why switching timeframe/pair used
    // to silently do nothing — the seeding effect only ever read
    // `chart_ready`, so it ran once and never again.
    let exchanges = use_memo(move || state.catalog.read().exchanges().to_vec());
    let selected_exchange = use_memo(move || {
        let exchanges = exchanges();
        if exchanges.iter().any(|e| e.name == exchange_choice()) {
            exchange_choice()
        } else {
            exchanges
                .first()
                .map(|e| e.name.clone())
                .unwrap_or_default()
        }
    });
    let pairs = use_memo(move || {
        state
            .catalog
            .read()
            .pairs_for(&selected_exchange())
            .to_vec()
    });
    let selected_pair = use_memo(move || {
        let pairs = pairs();
        let wanted = pair_choice();
        if pairs
            .iter()
            .any(|p| format!("{}/{}", p.base, p.quote) == wanted)
        {
            wanted
        } else {
            pairs
                .first()
                .map(|p| format!("{}/{}", p.base, p.quote))
                .unwrap_or_default()
        }
    });
    #[allow(clippy::redundant_closure)]
    let selected_interval = use_memo(move || interval_choice());
    let series_key = use_memo(move || {
        format!(
            "{}:{}:{}",
            selected_exchange(),
            selected_pair(),
            selected_interval()
        )
    });

    // Symbol search combobox — flattens every exchange's pairs into one
    // filterable list (see `ExchangeCatalog::symbol_options`) so the user
    // isn't forced to pick an exchange before seeing its pairs.
    let symbol_options = use_memo(move || state.catalog.read().symbol_options());
    let filtered_symbols = use_memo(move || filter_symbols(&symbol_options(), &symbol_query()));
    // Keeps the input's displayed text in sync with the real selection
    // whenever the dropdown is closed, without clobbering what the user is
    // actively typing while it's open.
    use_effect(move || {
        let exchange = selected_exchange();
        let pair = selected_pair();
        if !symbol_dropdown_open() {
            symbol_query.set(if exchange.is_empty() && pair.is_empty() {
                String::new()
            } else {
                format!("{exchange}  {pair}")
            });
        }
    });

    let theme = (state.theme)();

    // Load the JS glue once and create the chart instance — runs once per
    // mount, not per render, since `use_future`'s closure only re-runs if a
    // tracked reactive value it reads changes (it reads none here).
    use_future(move || {
        let theme_str = theme.as_str().to_string();
        async move {
            eval_logged(CHART_GLUE_JS).await;
            // `lightweight-charts.standalone.production.js` is injected by
            // `ui/web`'s root component as a Dioxus-rendered `<script>` tag,
            // which the browser fetches asynchronously — there's no
            // guarantee it has finished executing by the time this
            // `use_future` runs (it usually hasn't, on a cold load), so
            // calling `scChartInit` immediately used to silently no-op
            // (`typeof LightweightCharts === "undefined"` bailed it out)
            // and the chart never rendered for the rest of the page's
            // life. Poll until the global actually exists before
            // initializing, capped so a genuinely missing/broken asset
            // fails loudly in the console instead of looping forever.
            let init = document::eval(&format!(
                r#"
                await new Promise((resolve, reject) => {{
                    let attempts = 0;
                    (function poll() {{
                        if (typeof LightweightCharts !== "undefined") {{
                            resolve();
                            return;
                        }}
                        if (++attempts > 100) {{
                            console.error("lightweight-charts failed to load after 5s");
                            reject(new Error("lightweight-charts failed to load"));
                            return;
                        }}
                        setTimeout(poll, 50);
                    }})();
                }});
                window.scChartInit('{CONTAINER_ID}', '{LEGEND_ID}', '{theme_str}');
                // `document::eval`'s bridge round-trips the eval's result
                // through `serde_json` — a script with no return value
                // resolves to JS `undefined`, which fails to deserialize
                // and makes the `Eval` future resolve `Err` even though
                // every statement above ran successfully. An explicit
                // return value avoids that false negative.
                return true;
                "#
            ))
            .await;
            // Only flip ready when init actually succeeded — leaving it
            // `false` on a load failure keeps the page's loading/empty
            // state showing instead of pretending the chart exists.
            if init.is_ok() {
                chart_ready.set(true);
            }
        }
    });

    // Re-theme the existing chart instance in place when the user toggles
    // light/dark, instead of tearing it down and reseeding. Reads
    // `(state.theme)()` directly inside the closure rather than the outer
    // `theme` snapshot — same reactivity rule as the exchange/pair/interval
    // memos above: an effect only tracks signals it reads inside its own
    // closure body, so capturing a plain value computed outside means this
    // would otherwise run once at mount and never again, leaving the
    // canvas's colors frozen on every later theme toggle even though the
    // rest of the page re-themes fine via the `data-theme` CSS attribute.
    use_effect(move || {
        let theme_str = (state.theme)().as_str().to_string();
        if chart_ready() {
            spawn(async move {
                eval_logged(&format!(
                    "window.scChartSetTheme('{CONTAINER_ID}', '{theme_str}')"
                ))
                .await;
            });
        }
    });

    // Seed full history whenever the chart is ready, the selected
    // exchange/pair/interval changes, or the WS transport just resynced
    // after a reconnect (`AppState::resync_epoch` — see its doc comment).
    // Reads every memo via its getter *inside* the closure so Dioxus
    // actually subscribes this effect to them — see the comment above the
    // memos.
    use_effect(move || {
        let exchange = selected_exchange();
        let pair = selected_pair();
        let interval = selected_interval();
        let key = series_key();
        let _resync = (state.resync_epoch)();
        if !chart_ready() || exchange.is_empty() || pair.is_empty() {
            return;
        }
        let api = api();
        let Some(token) = current_token(&state) else {
            return;
        };
        let mut state = state;
        last_pushed_candle.set(None);
        load_error.set(None);
        loading.set(true);
        spawn(async move {
            let result = api
                .list_candles(&token, &exchange, &pair, &interval, HISTORY_LIMIT)
                .await;
            // `series_key()` reads the memo's *current* value (not a stale
            // snapshot — memos are plain signals outside the reactivity
            // rule that applies to `use_effect`/`use_memo` setup closures),
            // so this catches the case where the user switched symbol/
            // interval again while this request was in flight. Without it,
            // a slower, older request resolving last would repaint the
            // chart with the wrong symbol's candles.
            if !should_apply_candle_response(&key, &series_key()) {
                return;
            }
            match result {
                Ok(items) => {
                    state.seed_candles(&key, &items);
                    if let Ok(json) = serde_json::to_string(&items) {
                        eval_logged(&format!(
                            "window.scChartSetData('{CONTAINER_ID}', {json})"
                        ))
                        .await;
                    }
                    load_error.set(None);
                }
                Err(e) => load_error.set(Some(e)),
            }
            loading.set(false);
        });
    });

    // Mirrors the most recent backtest result onto the chart whenever
    // `BacktestStore` changes — reads it *inside* the closure (not a
    // snapshot captured outside) so Dioxus actually subscribes this effect
    // to it, the same reactivity rule the exchange/pair/interval memos
    // above call out; getting this wrong is the exact bug that made
    // timeframe switching silently do nothing earlier in this page's life.
    use_effect(move || {
        let result = state.backtest.read().result.clone();
        if !chart_ready() {
            return;
        }
        spawn(async move {
            match result {
                Some(result) => {
                    let stats_row = format_stats_row(&result);
                    if let (Ok(trades_json), Ok(stats_json)) = (
                        serde_json::to_string(&result.closed_trades),
                        serde_json::to_string(&stats_row),
                    ) {
                        eval_logged(&format!(
                            "window.scChartSetTrades('{CONTAINER_ID}', {trades_json}); \
                             window.scChartSetStats('{CONTAINER_ID}', {stats_json})"
                        ))
                        .await;
                    }
                }
                None => {
                    eval_logged(&format!(
                        "window.scChartClearTrades('{CONTAINER_ID}'); \
                         window.scChartClearStats('{CONTAINER_ID}')"
                    ))
                    .await;
                }
            }
        });
    });

    // Live order overlay (Stage 10, intentionally partial) — an entry
    // marker only, since `OrderUpdatePayload` carries no SL/TP to draw a
    // full trade rectangle/lines from. Reads `state.orders` and the
    // exchange/pair memos inside the closure for the same reactivity
    // reason as every other effect on this page.
    use_effect(move || {
        let exchange = selected_exchange();
        let pair = selected_pair();
        let marker = state.orders.read().open_position_for(&exchange, &pair);
        if !chart_ready() {
            return;
        }
        spawn(async move {
            let json = match &marker {
                Some(m) => serde_json::to_string(m).unwrap_or_else(|_| "null".to_string()),
                None => "null".to_string(),
            };
            eval_logged(&format!(
                "window.scChartSetLiveOrderMarker('{CONTAINER_ID}', {json})"
            ))
            .await;
        });
    });

    // Push only the latest bar from the live WS feed. The engine rebroadcasts
    // the still-forming bar on every tick (same `time`, growing high/low/
    // close) so the chart's current candle animates exactly like a real
    // exchange feed — dedupe on full bar equality, not just `time`, or every
    // tick after the first one for a given bar gets silently dropped here.
    // Reads `series_key()` inside the closure for the same reason as the
    // seeding effect above — otherwise a pair/interval switch keeps pushing
    // updates into the chart for the *previous* selection.
    use_effect(move || {
        let key = series_key();
        let candles = state.candles.read();
        let Some(latest) = candles.series_for(&key).last().cloned() else {
            return;
        };
        drop(candles);
        if !chart_ready() || last_pushed_candle() == Some(latest.clone()) {
            return;
        }
        last_pushed_candle.set(Some(latest.clone()));
        spawn(async move {
            if let Ok(json) = serde_json::to_string(&latest) {
                eval_logged(&format!("window.scChartUpdate('{CONTAINER_ID}', {json})")).await;
            }
        });
    });

    let candles = state.candles.read();
    let series = candles.series_for(&series_key());
    let summary = series.first().zip(series.last()).map(|(first, last)| {
        let change = last.close as f64 - first.open as f64;
        let pct = if first.open != 0 {
            change / first.open as f64 * 100.0
        } else {
            0.0
        };
        (last.clone(), change, pct)
    });
    let is_empty = series.is_empty();
    drop(candles);

    rsx! {
        div { class: "chart-page-full",
            // Combined header bar — symbol search, last price/change/OHLC,
            // and interval buttons in one dense row instead of a page
            // title + two separate toolbar rows, so the chart canvas below
            // gets the rest of the viewport instead of a small card.
            div { class: "chart-header-bar",
                div { class: "chart-symbol-search",
                    input {
                        class: "finput",
                        r#type: "text",
                        placeholder: "Search symbol…",
                        value: "{symbol_query()}",
                        oninput: move |e| {
                            symbol_query.set(e.value());
                            symbol_dropdown_open.set(true);
                        },
                        // Clears to a blank search on focus (rather than
                        // leaving the current "exchange pair" text sitting
                        // there) so typing immediately starts a fresh
                        // filter instead of inserting mid-string — the
                        // sync-back effect above restores the display text
                        // once the dropdown closes without a selection.
                        onfocus: move |_| {
                            symbol_query.set(String::new());
                            symbol_dropdown_open.set(true);
                        },
                        onblur: move |_| symbol_dropdown_open.set(false),
                    }
                    if symbol_dropdown_open() {
                        div { class: "chart-symbol-dropdown",
                            for (ex, pair) in filtered_symbols().into_iter() {
                                div {
                                    class: "chart-symbol-option",
                                    key: "{ex}:{pair}",
                                    onmousedown: move |_| {
                                        exchange_choice.set(ex.clone());
                                        pair_choice.set(pair.clone());
                                        symbol_dropdown_open.set(false);
                                    },
                                    span { class: "chart-symbol-option-exchange", "{ex}" }
                                    span { class: "chart-symbol-option-pair", "{pair}" }
                                }
                            }
                            if filtered_symbols().is_empty() {
                                div { class: "chart-symbol-empty", "No matching symbol" }
                            }
                        }
                    }
                }
                if let Some((last, change, pct)) = &summary {
                    span { class: "chart-last-price", "{format_price(last.close as f64)}" }
                    span {
                        class: if *change >= 0.0 { "chart-change pos" } else { "chart-change neg" },
                        if *change >= 0.0 { "▲ " } else { "▼ " }
                        "{format_price(change.abs())} ({pct.abs():.2}%)"
                    }
                    span { class: "chart-ohlc",
                        span { "O " b { "{format_price(last.open as f64)}" } }
                        span { "H " b { "{format_price(last.high as f64)}" } }
                        span { "L " b { "{format_price(last.low as f64)}" } }
                        span { "C " b { "{format_price(last.close as f64)}" } }
                    }
                }
                div { class: "interval-group",
                    for i in INTERVALS {
                        button {
                            class: if selected_interval() == *i { "interval-btn active" } else { "interval-btn" },
                            r#type: "button",
                            onclick: move |_| interval_choice.set(i.to_string()),
                            "{i}"
                        }
                    }
                }
            }

            div { class: "chart-canvas-wrap",
                div { id: CONTAINER_ID, style: "width:100%; height:100%;" }
                div { id: LEGEND_ID, class: "chart-legend" }
                // Hidden while there's nothing to draw on (loading/error/
                // empty all render a centered overlay message) — besides
                // being pointless, the floating rail sits at the chart's
                // left-center and would otherwise overlap that centered
                // text, badly on narrow viewports.
                if !loading() && load_error().is_none() && !is_empty {
                div { class: "chart-draw-toolbar",
                    for tool in DRAW_TOOLS {
                        button {
                            key: "{tool.id}",
                            class: if drawing_tool() == tool.id { "draw-btn active" } else { "draw-btn" },
                            title: tool.title,
                            r#type: "button",
                            onclick: move |_| {
                                let id = tool.id;
                                drawing_tool.set(id.to_string());
                                spawn(async move {
                                    eval_logged(&format!(
                                        "window.scChartSetTool('{CONTAINER_ID}', '{id}')"
                                    ))
                                    .await;
                                });
                            },
                            {(tool.icon)()}
                        }
                    }
                    div { class: "draw-btn-sep" }
                    button {
                        class: "draw-btn",
                        title: "Clear all drawings",
                        r#type: "button",
                        onclick: move |_| {
                            spawn(async move {
                                eval_logged(&format!(
                                    "window.scChartClearDrawings('{CONTAINER_ID}')"
                                ))
                                .await;
                            });
                        },
                        IconTrash {}
                    }
                    div { class: "draw-btn-sep" }
                    button {
                        class: if trade_rectangles_on() { "draw-btn active" } else { "draw-btn" },
                        title: "Toggle trade rectangles (backtest overlay)",
                        r#type: "button",
                        onclick: move |_| {
                            let next = !trade_rectangles_on();
                            trade_rectangles_on.set(next);
                            let density = if next { "all" } else { "markers-only" };
                            spawn(async move {
                                eval_logged(&format!(
                                    "window.scChartSetTradeDensity('{CONTAINER_ID}', '{density}')"
                                ))
                                .await;
                            });
                        },
                        IconRectangle {}
                    }
                }
                }
                if loading() {
                    div { class: "chart-state",
                        div { class: "chart-spinner" }
                        "Loading chart data…"
                    }
                } else if let Some(err) = load_error() {
                    div { class: "chart-state error", "{err}" }
                } else if is_empty {
                    div { class: "chart-state", "No candles yet for this exchange/pair/interval — waiting for live ticks." }
                }
            }
        }
    }
}

/// Case-insensitive substring filter over `(exchange, "BASE/QUOTE")` rows
/// for the chart page's symbol search combobox — a free function so it's
/// testable without a Dioxus runtime, same convention as
/// `ticker_request_body` in `api/mod.rs`.
fn filter_symbols(options: &[(String, String)], query: &str) -> Vec<(String, String)> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return options.to_vec();
    }
    options
        .iter()
        .filter(|(exchange, pair)| format!("{exchange} {pair}").to_lowercase().contains(&query))
        .cloned()
        .collect()
}


/// Guards against a stale `list_candles` response overwriting the chart:
/// switching exchange/pair/interval quickly can leave an older request
/// in flight, and without this check a slower response landing *after* a
/// newer one would repaint the chart with the wrong symbol's candles. The
/// seeding effect captures `requested_key` when it fires the request, then
/// compares against the *current* selection's key once the response lands.
fn should_apply_candle_response(requested_key: &str, current_key: &str) -> bool {
    requested_key == current_key
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_apply_candle_response_true_when_selection_unchanged() {
        assert!(should_apply_candle_response(
            "tabdeal:USDT/IRT:1m",
            "tabdeal:USDT/IRT:1m"
        ));
    }

    #[test]
    fn should_apply_candle_response_false_when_selection_changed_since_request() {
        assert!(!should_apply_candle_response(
            "tabdeal:USDT/IRT:1m",
            "tabdeal:BTC/IRT:1m"
        ));
    }

    fn options() -> Vec<(String, String)> {
        vec![
            ("tabdeal".to_string(), "USDT/IRT".to_string()),
            ("tabdeal".to_string(), "BTC/IRT".to_string()),
            ("hitobit".to_string(), "USDT/IRT".to_string()),
        ]
    }

    #[test]
    fn filter_symbols_returns_all_when_query_is_empty() {
        assert_eq!(filter_symbols(&options(), "").len(), 3);
    }

    #[test]
    fn filter_symbols_matches_case_insensitive_substring() {
        let result = filter_symbols(&options(), "TABDEAL");
        assert_eq!(result.len(), 2);
        assert!(result.iter().all(|(ex, _)| ex == "tabdeal"));
    }

    #[test]
    fn filter_symbols_matches_against_the_pair_too() {
        let result = filter_symbols(&options(), "btc");
        assert_eq!(result, vec![("tabdeal".to_string(), "BTC/IRT".to_string())]);
    }

    #[test]
    fn filter_symbols_returns_empty_when_nothing_matches() {
        assert!(filter_symbols(&options(), "nonexistent").is_empty());
    }


}
