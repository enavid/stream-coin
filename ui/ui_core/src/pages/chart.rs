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
use crate::icons::{
    IconArrowDown, IconArrowUp, IconCursor, IconFibonacci, IconHorizontalLine, IconRectangle,
    IconRuler, IconTrash, IconTrendLine, IconVerticalLine,
};
use crate::state::AppState;

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
  // Drawing tools that aren't natively supported by lightweight-charts
  // (vertical line, rectangle, fib labels, measure label) are plain
  // absolutely-positioned DOM children of the chart container, tagged
  // `.sc-overlay` so a re-init can find and clear stale ones. Colors for
  // these come from CSS variables (`--accent`/`--text` etc., already
  // theme-aware via `#app[data-theme]`), not from `colors`/JS — only the
  // canvas-rendered native series/price-lines need that, since canvas
  // isn't part of the CSS cascade.
  function makeOverlayDiv(el, className) {
    var d = document.createElement("div");
    d.className = "sc-overlay " + className;
    el.appendChild(d);
    return d;
  }
  function repositionOverlays(containerId) {
    var entry = window.scCharts[containerId];
    if (!entry) return;
    entry.drawings.forEach(function (d) {
      if (d.type === "vline") {
        var x = entry.chart.timeScale().timeToCoordinate(d.time);
        if (x === null) { d.el.style.display = "none"; return; }
        d.el.style.display = "block";
        d.el.style.left = x + "px";
      } else if (d.type === "rect") {
        var x1 = entry.chart.timeScale().timeToCoordinate(d.t1);
        var x2 = entry.chart.timeScale().timeToCoordinate(d.t2);
        var y1 = entry.series.priceToCoordinate(d.p1);
        var y2 = entry.series.priceToCoordinate(d.p2);
        if (x1 === null || x2 === null || y1 === null || y2 === null) { d.el.style.display = "none"; return; }
        d.el.style.display = "block";
        d.el.style.left = Math.min(x1, x2) + "px";
        d.el.style.top = Math.min(y1, y2) + "px";
        d.el.style.width = Math.abs(x2 - x1) + "px";
        d.el.style.height = Math.abs(y2 - y1) + "px";
      } else if (d.type === "fib") {
        var fx = entry.chart.timeScale().timeToCoordinate(d.t2);
        d.labels.forEach(function (lab, i) {
          var y = entry.series.priceToCoordinate(d.levels[i]);
          if (fx === null || y === null) { lab.style.display = "none"; return; }
          lab.style.display = "block";
          lab.style.left = (fx + 4) + "px";
          lab.style.top = (y - 7) + "px";
        });
      }
      // "measure" labels are intentionally static (placed once at draw
      // time) — not repositioned here. "priceline"/"lineseries"/"marker"
      // are native chart primitives that reposition themselves.
    });
  }
  window.scChartInit = function (containerId, legendId, theme) {
    var el = document.getElementById(containerId);
    if (!el || typeof LightweightCharts === "undefined") return;
    var existing = window.scCharts[containerId];
    if (existing) {
      if (existing.resizeObserver) existing.resizeObserver.disconnect();
      existing.chart.remove();
    }
    el.querySelectorAll(".sc-overlay").forEach(function (n) { n.remove(); });
    var colors = colorsFor(theme);
    var chart = LightweightCharts.createChart(el, {
      width: el.clientWidth,
      height: el.clientHeight,
      // `attributionLogo: false` drops the TradingView watermark — a
      // supported `layout` flag on this vendored build, not a license
      // requirement we're working around.
      layout: { background: { color: colors.bg }, textColor: colors.text, attributionLogo: false },
      grid: {
        vertLines: { color: colors.grid },
        horzLines: { color: colors.grid },
      },
      // `barSpacing: 6` matches TradingView's own default — denser than
      // this library's default (~9-10px), which read as "candles too big"
      // especially before the chart had full page width to spread into.
      timeScale: { timeVisible: true, secondsVisible: false, barSpacing: 6 },
    });
    var series = chart.addCandlestickSeries({
      upColor: colors.up,
      downColor: colors.down,
      borderUpColor: colors.up,
      borderDownColor: colors.down,
      wickUpColor: colors.up,
      wickDownColor: colors.down,
    });
    var entry = {
      chart: chart,
      series: series,
      colors: colors,
      // Drawing-tool state — `drawings` holds every artifact the user has
      // placed (native price lines/line series, plus the DOM-overlay
      // vline/rect/fib types) so `scChartClearDrawings` can remove them by
      // reference; `markers` is the running list passed to
      // `series.setMarkers` (that API replaces the whole array every
      // call, so it can't be tracked per-item the way `drawings` is);
      // `activeTool` is `null` (cursor) or one of the 9 tool ids below;
      // `pendingPoint` holds the first click of any two-click tool
      // (trend/rect/fib/measure — only one can be mid-draw at a time).
      drawings: [],
      markers: [],
      activeTool: null,
      pendingPoint: null,
    };
    if (window.ResizeObserver) {
      var ro = new ResizeObserver(function () {
        chart.applyOptions({ width: el.clientWidth, height: el.clientHeight });
        repositionOverlays(containerId);
      });
      ro.observe(el);
      entry.resizeObserver = ro;
    }
    chart.timeScale().subscribeVisibleTimeRangeChange(function () {
      repositionOverlays(containerId);
    });
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
    chart.subscribeClick(function (param) {
      var live = window.scCharts[containerId];
      if (!live || !live.activeTool) return;
      if (!param || !param.point || param.time === undefined) return;
      var price = series.coordinateToPrice(param.point.y);
      if (price === null || price === undefined) return;
      var tool = live.activeTool;

      if (tool === "hline") {
        var line = series.createPriceLine({
          price: price,
          color: live.colors.text,
          lineWidth: 1,
          lineStyle: LightweightCharts.LineStyle.Dashed,
          axisLabelVisible: true,
        });
        live.drawings.push({ type: "priceline", ref: line });
        return;
      }

      if (tool === "vline") {
        var vEl = makeOverlayDiv(el, "sc-vline");
        live.drawings.push({ type: "vline", time: param.time, el: vEl });
        repositionOverlays(containerId);
        return;
      }

      if (tool === "buy" || tool === "sell") {
        var marker = {
          time: param.time,
          position: tool === "buy" ? "belowBar" : "aboveBar",
          color: tool === "buy" ? live.colors.up : live.colors.down,
          shape: tool === "buy" ? "arrowUp" : "arrowDown",
          text: tool === "buy" ? "Buy" : "Sell",
        };
        live.markers.push(marker);
        live.markers.sort(function (a, b) { return a.time - b.time; });
        series.setMarkers(live.markers);
        live.drawings.push({ type: "marker", ref: marker });
        return;
      }

      // Every remaining tool (trend/rect/fib/measure) is a two-click
      // drawing — same "click, click" model as the already-working trend
      // line, not click-and-drag.
      if (!live.pendingPoint) {
        live.pendingPoint = { time: param.time, value: price };
        return;
      }
      var p1 = live.pendingPoint;
      var p2 = { time: param.time, value: price };
      live.pendingPoint = null;

      // `priceLineVisible`/`lastValueVisible` are meant for a
      // live-updating series' "current value" indicator — none of these
      // static two-point lines are that, and leaving them on clutters the
      // price axis with a label per drawn line.
      var staticLineOpts = {
        color: live.colors.text,
        lineWidth: 2,
        priceLineVisible: false,
        lastValueVisible: false,
        crosshairMarkerVisible: false,
      };

      if (tool === "trend") {
        var trendSeries = chart.addLineSeries(staticLineOpts);
        trendSeries.setData([p1, p2]);
        live.drawings.push({ type: "lineseries", ref: trendSeries });
      } else if (tool === "rect") {
        var rEl = makeOverlayDiv(el, "sc-rect");
        live.drawings.push({ type: "rect", t1: p1.time, p1: p1.value, t2: p2.time, p2: p2.value, el: rEl });
        repositionOverlays(containerId);
      } else if (tool === "fib") {
        var ratios = [0, 0.236, 0.382, 0.5, 0.618, 0.786, 1];
        var high = Math.max(p1.value, p2.value);
        var low = Math.min(p1.value, p2.value);
        var lines = [];
        var labels = [];
        var levels = [];
        ratios.forEach(function (r) {
          var levelPrice = high - (high - low) * r;
          levels.push(levelPrice);
          var fibSeries = chart.addLineSeries(Object.assign({}, staticLineOpts, { lineWidth: 1 }));
          fibSeries.setData([{ time: p1.time, value: levelPrice }, { time: p2.time, value: levelPrice }]);
          lines.push(fibSeries);
          var lab = makeOverlayDiv(el, "sc-fib-label");
          lab.textContent = (r * 100).toFixed(1) + "%";
          labels.push(lab);
        });
        live.drawings.push({ type: "fib", lines: lines, labels: labels, t1: p1.time, t2: p2.time, levels: levels });
        repositionOverlays(containerId);
      } else if (tool === "measure") {
        var measureSeries = chart.addLineSeries(staticLineOpts);
        measureSeries.setData([p1, p2]);
        var delta = p2.value - p1.value;
        var pct = p1.value !== 0 ? (delta / p1.value) * 100 : 0;
        var x1m = chart.timeScale().timeToCoordinate(p1.time);
        var x2m = chart.timeScale().timeToCoordinate(p2.time);
        var y1m = series.priceToCoordinate(p1.value);
        var y2m = series.priceToCoordinate(p2.value);
        var labelEl = makeOverlayDiv(el, "sc-measure-label");
        labelEl.textContent = (delta >= 0 ? "+" : "") + fmt(delta) + " (" + (pct >= 0 ? "+" : "") + pct.toFixed(2) + "%)";
        // Static placement, computed once here — doesn't track pan/zoom
        // like the vline/rect/fib overlays do (see `repositionOverlays`),
        // an accepted, documented limitation for this tool.
        if (x1m !== null && x2m !== null && y1m !== null && y2m !== null) {
          labelEl.style.left = ((x1m + x2m) / 2) + "px";
          labelEl.style.top = ((y1m + y2m) / 2 - 18) + "px";
        }
        live.drawings.push({ type: "measure", ref: measureSeries, el: labelEl });
      }
    });
    window.scCharts[containerId] = entry;
  };
  window.scChartSetData = function (containerId, candles) {
    var entry = window.scCharts[containerId];
    if (!entry) return;
    // A new symbol/interval invalidates every existing drawing's time
    // coordinates, so clear them rather than leave stale lines floating
    // over an unrelated instrument's candles.
    window.scChartClearDrawings(containerId);
    entry.series.setData(candles.map(toPoint));
    entry.chart.timeScale().fitContent();
  };
  window.scChartUpdate = function (containerId, candle) {
    var entry = window.scCharts[containerId];
    if (!entry) return;
    entry.series.update(toPoint(candle));
  };
  window.scChartSetTool = function (containerId, tool) {
    var entry = window.scCharts[containerId];
    if (!entry) return;
    entry.activeTool = tool === "cursor" ? null : tool;
    entry.pendingPoint = null;
    var el = document.getElementById(containerId);
    if (entry.activeTool) {
      // Disabling pan/zoom while a tool is armed isn't just cosmetic: a
      // real mouse click almost always has a pixel or two of jitter
      // between mousedown and mouseup, which this library can otherwise
      // read as "start of a drag" and swallow before `subscribeClick`
      // ever fires — the single most likely reason drawing felt broken
      // for an actual mouse instead of a scripted, pixel-perfect click.
      entry.chart.applyOptions({ handleScroll: false, handleScale: false });
      if (el) el.style.cursor = "crosshair";
    } else {
      entry.chart.applyOptions({ handleScroll: true, handleScale: true });
      if (el) el.style.cursor = "default";
    }
  };
  window.scChartClearDrawings = function (containerId) {
    var entry = window.scCharts[containerId];
    if (!entry) return;
    entry.drawings.forEach(function (d) {
      if (d.type === "priceline") entry.series.removePriceLine(d.ref);
      else if (d.type === "lineseries" || d.type === "measure") entry.chart.removeSeries(d.ref);
      else if (d.type === "vline" || d.type === "rect") d.el.remove();
      else if (d.type === "fib") {
        d.lines.forEach(function (l) { entry.chart.removeSeries(l); });
        d.labels.forEach(function (lab) { lab.remove(); });
      }
      if (d.type === "measure") d.el.remove();
      // "marker" entries are handled below via the shared `markers`
      // array reset, not per-item — `setMarkers` always takes the full set.
    });
    entry.drawings = [];
    entry.markers = [];
    entry.series.setMarkers([]);
    entry.pendingPoint = null;
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
    let mut symbol_query = use_signal(String::new);
    let mut symbol_dropdown_open = use_signal(|| false);
    let mut drawing_tool = use_signal(|| "cursor".to_string());

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
            let _ = document::eval(CHART_GLUE_JS).await;
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
                                    let _ = document::eval(&format!(
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
                                let _ = document::eval(&format!(
                                    "window.scChartClearDrawings('{CONTAINER_ID}')"
                                ))
                                .await;
                            });
                        },
                        IconTrash {}
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

#[cfg(test)]
mod tests {
    use super::*;

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
