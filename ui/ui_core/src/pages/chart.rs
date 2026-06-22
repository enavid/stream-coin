//! TradingView-style candlestick chart page. Renders via TradingView's own
//! open-source `lightweight-charts` JS library (vendored into
//! `ui/web/assets/`, loaded by `ui/web/src/main.rs` as a Dioxus asset) — no
//! Dioxus-native candlestick component exists, and hand-rolling OHLCV canvas
//! rendering isn't worth it when the reference implementation is free and
//! purpose-built. `document::eval` is the only JS-interop mechanism Dioxus
//! 0.7 offers without a hand-written `wasm-bindgen` extern module, so the
//! glue below is plain JS strings.

use dioxus::prelude::*;

use super::current_token;
use crate::api::ApiClient;
use crate::domain::format::format_price;
use crate::state::AppState;

const CONTAINER_ID: &str = "sc-candlestick-chart";
const LEGEND_ID: &str = "sc-candlestick-chart-legend";
const INTERVALS: &[&str] = &["1m", "5m", "15m", "1h"];
const HISTORY_LIMIT: u32 = 300;

/// Defines `window.scChartInit/SetData/Update/SetTheme`, scoped by a
/// container id so a future multi-chart page wouldn't collide. Idempotent —
/// safe to `eval` again on every mount (e.g. hot reload). Uses
/// `ResizeObserver` rather than a `window` resize listener so the chart also
/// resizes when the sidebar opens/closes or the container's flex layout
/// changes without the viewport itself changing size.
const CHART_GLUE_JS: &str = r##"
(function () {
  window.scCharts = window.scCharts || {};
  function colorsFor(theme) {
    return theme === "light"
      ? { bg: "#ffffff", text: "#353a48", grid: "#e1e4ec", up: "#059669", down: "#e11d48" }
      : { bg: "#111219", text: "#c9cedb", grid: "#20232f", up: "#10b981", down: "#f43f5e" };
  }
  function toPoint(c) {
    return {
      time: Math.floor(new Date(c.time).getTime() / 1000),
      open: c.open,
      high: c.high,
      low: c.low,
      close: c.close,
    };
  }
  function fmt(v) {
    return Math.round(v).toLocaleString("en-US");
  }
  function renderLegend(legendEl, colors, o, h, l, c) {
    var up = c >= o;
    legendEl.style.color = colors.text;
    legendEl.innerHTML =
      '<span>O <b>' + fmt(o) + '</b></span>' +
      '<span>H <b>' + fmt(h) + '</b></span>' +
      '<span>L <b>' + fmt(l) + '</b></span>' +
      '<span style="color:' + (up ? colors.up : colors.down) + '">C <b>' + fmt(c) + '</b></span>';
  }
  window.scChartInit = function (containerId, legendId, theme) {
    var el = document.getElementById(containerId);
    if (!el || typeof LightweightCharts === "undefined") return;
    var existing = window.scCharts[containerId];
    if (existing) {
      if (existing.resizeObserver) existing.resizeObserver.disconnect();
      existing.chart.remove();
    }
    var colors = colorsFor(theme);
    var chart = LightweightCharts.createChart(el, {
      width: el.clientWidth,
      height: el.clientHeight,
      layout: { background: { color: colors.bg }, textColor: colors.text },
      grid: {
        vertLines: { color: colors.grid },
        horzLines: { color: colors.grid },
      },
      timeScale: { timeVisible: true, secondsVisible: false },
    });
    var series = chart.addCandlestickSeries({
      upColor: colors.up,
      downColor: colors.down,
      borderUpColor: colors.up,
      borderDownColor: colors.down,
      wickUpColor: colors.up,
      wickDownColor: colors.down,
    });
    var entry = { chart: chart, series: series, colors: colors };
    if (window.ResizeObserver) {
      var ro = new ResizeObserver(function () {
        chart.applyOptions({ width: el.clientWidth, height: el.clientHeight });
      });
      ro.observe(el);
      entry.resizeObserver = ro;
    }
    var legendEl = document.getElementById(legendId);
    chart.subscribeCrosshairMove(function (param) {
      if (!legendEl) return;
      if (!param || !param.time) {
        legendEl.style.display = "none";
        return;
      }
      var data = param.seriesData.get(series);
      if (!data) {
        legendEl.style.display = "none";
        return;
      }
      legendEl.style.display = "flex";
      renderLegend(legendEl, window.scCharts[containerId].colors, data.open, data.high, data.low, data.close);
    });
    window.scCharts[containerId] = entry;
  };
  window.scChartSetData = function (containerId, candles) {
    var entry = window.scCharts[containerId];
    if (!entry) return;
    entry.series.setData(candles.map(toPoint));
    entry.chart.timeScale().fitContent();
  };
  window.scChartUpdate = function (containerId, candle) {
    var entry = window.scCharts[containerId];
    if (!entry) return;
    entry.series.update(toPoint(candle));
  };
  window.scChartSetTheme = function (containerId, theme) {
    var entry = window.scCharts[containerId];
    if (!entry) return;
    var colors = colorsFor(theme);
    entry.colors = colors;
    entry.chart.applyOptions({
      layout: { background: { color: colors.bg }, textColor: colors.text },
      grid: {
        vertLines: { color: colors.grid },
        horzLines: { color: colors.grid },
      },
    });
    entry.series.applyOptions({
      upColor: colors.up,
      downColor: colors.down,
      borderUpColor: colors.up,
      borderDownColor: colors.down,
      wickUpColor: colors.up,
      wickDownColor: colors.down,
    });
  };
})();
"##;

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
    let mut last_pushed_time = use_signal(|| None::<String>);

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

    let theme = (state.theme)();

    // Load the JS glue once and create the chart instance — runs once per
    // mount, not per render, since `use_future`'s closure only re-runs if a
    // tracked reactive value it reads changes (it reads none here).
    use_future(move || {
        let theme_str = theme.as_str().to_string();
        async move {
            let _ = document::eval(CHART_GLUE_JS).await;
            let _ = document::eval(&format!(
                "window.scChartInit('{CONTAINER_ID}', '{LEGEND_ID}', '{theme_str}')"
            ))
            .await;
            chart_ready.set(true);
        }
    });

    // Re-theme the existing chart instance in place when the user toggles
    // light/dark, instead of tearing it down and reseeding.
    use_effect(move || {
        let theme_str = theme.as_str().to_string();
        if chart_ready() {
            spawn(async move {
                let _ = document::eval(&format!(
                    "window.scChartSetTheme('{CONTAINER_ID}', '{theme_str}')"
                ))
                .await;
            });
        }
    });

    // Seed full history whenever the chart is ready or the selected
    // exchange/pair/interval changes. Reads every memo via its getter
    // *inside* the closure so Dioxus actually subscribes this effect to
    // them — see the comment above the memos.
    use_effect(move || {
        let exchange = selected_exchange();
        let pair = selected_pair();
        let interval = selected_interval();
        let key = series_key();
        if !chart_ready() || exchange.is_empty() || pair.is_empty() {
            return;
        }
        let api = api();
        let Some(token) = current_token(&state) else {
            return;
        };
        let mut state = state;
        last_pushed_time.set(None);
        load_error.set(None);
        loading.set(true);
        spawn(async move {
            match api
                .list_candles(&token, &exchange, &pair, &interval, HISTORY_LIMIT)
                .await
            {
                Ok(items) => {
                    state.seed_candles(&key, &items);
                    if let Ok(json) = serde_json::to_string(&items) {
                        let _ = document::eval(&format!(
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

    // Push only the latest bar from the live WS feed (cheap — avoids
    // re-serializing the whole history on every tick); skips re-sending a
    // bar whose `time` was already pushed. Reads `series_key()` inside the
    // closure for the same reason as the seeding effect above — otherwise
    // a pair/interval switch keeps pushing updates into the chart for the
    // *previous* selection.
    use_effect(move || {
        let key = series_key();
        let candles = state.candles.read();
        let Some(latest) = candles.series_for(&key).last().cloned() else {
            return;
        };
        drop(candles);
        if !chart_ready() || last_pushed_time() == Some(latest.time.clone()) {
            return;
        }
        last_pushed_time.set(Some(latest.time.clone()));
        spawn(async move {
            if let Ok(json) = serde_json::to_string(&latest) {
                let _ = document::eval(&format!("window.scChartUpdate('{CONTAINER_ID}', {json})"))
                    .await;
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
        div { class: "page-head",
            div {
                div { class: "page-title", "Chart" }
                div { class: "page-sub", "Live candlestick chart for any active exchange/pair" }
            }
        }

        section { class: "block card",
            div { class: "chart-toolbar",
                div { class: "chart-selectors",
                    div { class: "field chart-select",
                        label { "Exchange" }
                        select {
                            class: "finput",
                            value: "{selected_exchange()}",
                            onchange: move |e| {
                                exchange_choice.set(e.value());
                                pair_choice.set(String::new());
                            },
                            for ex in exchanges().iter() { option { value: "{ex.name}", "{ex.name}" } }
                        }
                    }
                    div { class: "field chart-select",
                        label { "Pair" }
                        select {
                            class: "finput",
                            value: "{selected_pair()}",
                            onchange: move |e| pair_choice.set(e.value()),
                            for p in pairs().iter() { option { value: "{p.base}/{p.quote}", "{p.base}/{p.quote}" } }
                        }
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

            div { class: "chart-summary",
                span { class: "chart-symbol", "{selected_exchange()}:{selected_pair()}" }
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
            }

            div { class: "chart-canvas-wrap",
                div { id: CONTAINER_ID, style: "width:100%; height:100%;" }
                div { id: LEGEND_ID, class: "chart-legend" }
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
