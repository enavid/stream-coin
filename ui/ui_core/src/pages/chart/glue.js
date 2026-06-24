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
        if (x1 === null || x2 === null || y1 === null || y2 === null) {
          d.el.style.display = "none";
          d.handle1.style.display = "none";
          d.handle2.style.display = "none";
          return;
        }
        d.el.style.display = "block";
        d.el.style.left = Math.min(x1, x2) + "px";
        d.el.style.top = Math.min(y1, y2) + "px";
        d.el.style.width = Math.abs(x2 - x1) + "px";
        d.el.style.height = Math.abs(y2 - y1) + "px";
        // Handles sit at the drawing's *stored* corners (t1,p1 / t2,p2),
        // not at the box's min/max — that's what makes each handle resize
        // the specific corner the user grabbed rather than always the
        // top-left/bottom-right of the visual box.
        d.handle1.style.display = entry.activeTool ? "none" : "block";
        d.handle2.style.display = entry.activeTool ? "none" : "block";
        d.handle1.style.left = x1 + "px";
        d.handle1.style.top = y1 + "px";
        d.handle2.style.left = x2 + "px";
        d.handle2.style.top = y2 + "px";
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
  // --- Drag-to-move for placed drawings (Cursor tool only) --------------
  // Every drawing tool until now was "place once, never touch again" —
  // traders expect to nudge a trend line or a position box after drawing
  // it, same as every other charting tool. Hit-testing and movement happen
  // in pixel space (via the same `timeToCoordinate`/`priceToCoordinate`
  // calls `repositionOverlays` already uses), then the delta is converted
  // back to time/price and applied to the drawing's stored coordinates —
  // not to the pixel position directly, so the shape still tracks pan/zoom
  // correctly afterward through the existing reposition/update paths.
  var HIT_TOLERANCE_PX = 6;
  function pointToSegmentDistance(px, py, x1, y1, x2, y2) {
    var dx = x2 - x1, dy = y2 - y1;
    var lenSq = dx * dx + dy * dy;
    var t = lenSq === 0 ? 0 : ((px - x1) * dx + (py - y1) * dy) / lenSq;
    t = Math.max(0, Math.min(1, t));
    var nx = x1 + t * dx, ny = y1 + t * dy;
    return Math.hypot(px - nx, py - ny);
  }
  // Returns the drawing under (x, y) in pixel space, or null. Checked in
  // reverse placement order so the most recently drawn (topmost-looking)
  // shape wins when two overlap.
  function hitTestDrawing(entry, x, y) {
    for (var i = entry.drawings.length - 1; i >= 0; i--) {
      var d = entry.drawings[i];
      var ts = entry.chart.timeScale();
      if (d.type === "vline") {
        var vx = ts.timeToCoordinate(d.time);
        if (vx !== null && Math.abs(vx - x) <= HIT_TOLERANCE_PX) return d;
      } else if (d.type === "priceline") {
        var py = entry.series.priceToCoordinate(d.price);
        if (py !== null && Math.abs(py - y) <= HIT_TOLERANCE_PX) return d;
      } else if (d.type === "rect") {
        var rx1 = ts.timeToCoordinate(d.t1), rx2 = ts.timeToCoordinate(d.t2);
        var ry1 = entry.series.priceToCoordinate(d.p1), ry2 = entry.series.priceToCoordinate(d.p2);
        if (rx1 === null || rx2 === null || ry1 === null || ry2 === null) continue;
        if (x >= Math.min(rx1, rx2) && x <= Math.max(rx1, rx2) && y >= Math.min(ry1, ry2) && y <= Math.max(ry1, ry2)) return d;
      } else if (d.type === "fib") {
        var fx1 = ts.timeToCoordinate(d.t1), fx2 = ts.timeToCoordinate(d.t2);
        if (fx1 === null || fx2 === null) continue;
        var fys = d.levels.map(function (lv) { return entry.series.priceToCoordinate(lv); }).filter(function (v) { return v !== null; });
        if (fys.length === 0) continue;
        if (x >= Math.min(fx1, fx2) && x <= Math.max(fx1, fx2) && y >= Math.min.apply(null, fys) - HIT_TOLERANCE_PX && y <= Math.max.apply(null, fys) + HIT_TOLERANCE_PX) return d;
      } else if (d.type === "lineseries" || d.type === "measure") {
        var lx1 = ts.timeToCoordinate(d.t1), lx2 = ts.timeToCoordinate(d.t2);
        var ly1 = entry.series.priceToCoordinate(d.v1), ly2 = entry.series.priceToCoordinate(d.v2);
        if (lx1 === null || lx2 === null || ly1 === null || ly2 === null) continue;
        if (pointToSegmentDistance(x, y, lx1, ly1, lx2, ly2) <= HIT_TOLERANCE_PX) return d;
      }
    }
    return null;
  }
  // Resize handles only exist on "rect" drawings (the corner squares drawn
  // in `repositionOverlays`) — checked separately and with priority over
  // `hitTestDrawing` since a handle sits inside the rectangle's own
  // bounding box. Returns `{ drawing, corner }` (1 = t1/p1, 2 = t2/p2) or
  // `null`.
  function hitTestRectHandle(entry, x, y) {
    var ts = entry.chart.timeScale();
    for (var i = entry.drawings.length - 1; i >= 0; i--) {
      var d = entry.drawings[i];
      if (d.type !== "rect") continue;
      var x1 = ts.timeToCoordinate(d.t1), y1 = entry.series.priceToCoordinate(d.p1);
      if (x1 !== null && y1 !== null && Math.hypot(x - x1, y - y1) <= HIT_TOLERANCE_PX + 4) {
        return { drawing: d, corner: 1 };
      }
      var x2 = ts.timeToCoordinate(d.t2), y2 = entry.series.priceToCoordinate(d.p2);
      if (x2 !== null && y2 !== null && Math.hypot(x - x2, y - y2) <= HIT_TOLERANCE_PX + 4) {
        return { drawing: d, corner: 2 };
      }
    }
    return null;
  }
  // Applies a (deltaTime, deltaPrice) shift to one drawing's stored
  // coordinates and redraws it — the inverse of `hitTestDrawing`'s pixel
  // read, one case per drawing type already handled there.
  function moveDrawingBy(entry, d, deltaTime, deltaPrice) {
    if (d.type === "vline") {
      d.time = d.time + deltaTime;
    } else if (d.type === "priceline") {
      d.price = d.price + deltaPrice;
      d.ref.applyOptions({ price: d.price });
    } else if (d.type === "rect") {
      d.t1 += deltaTime; d.t2 += deltaTime; d.p1 += deltaPrice; d.p2 += deltaPrice;
    } else if (d.type === "fib") {
      d.t1 += deltaTime; d.t2 += deltaTime;
      d.levels = d.levels.map(function (lv) { return lv + deltaPrice; });
      d.lines.forEach(function (line, i) {
        line.setData([{ time: d.t1, value: d.levels[i] }, { time: d.t2, value: d.levels[i] }]);
      });
    } else if (d.type === "lineseries" || d.type === "measure") {
      d.t1 += deltaTime; d.t2 += deltaTime; d.v1 += deltaPrice; d.v2 += deltaPrice;
      d.ref.setData([{ time: d.t1, value: d.v1 }, { time: d.t2, value: d.v2 }]);
      if (d.type === "measure" && d.el) {
        var ts2 = entry.chart.timeScale();
        var ex1 = ts2.timeToCoordinate(d.t1), ex2 = ts2.timeToCoordinate(d.t2);
        var ey1 = entry.series.priceToCoordinate(d.v1), ey2 = entry.series.priceToCoordinate(d.v2);
        if (ex1 !== null && ex2 !== null && ey1 !== null && ey2 !== null) {
          d.el.style.left = ((ex1 + ex2) / 2) + "px";
          d.el.style.top = ((ey1 + ey2) / 2 - 18) + "px";
        }
      }
    }
    repositionOverlays(entry.containerId);
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
      // The forming candle is now rebroadcast on every WS price tick (not
      // just on close), so this series' built-in "current value" price
      // line/axis-label — designed for a slow-moving last price — instead
      // jumps continuously between the forming bar's high/low many times a
      // second, reading as a second, jittery crosshair line. The header
      // bar's live price/OHLC summary and the hover legend already show the
      // same number without the jitter, so the canvas line is redundant.
      priceLineVisible: false,
      lastValueVisible: false,
    });
    var entry = {
      containerId: containerId,
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
      // Set while the user is mid-drag of an existing drawing (Cursor tool
      // only) — `drawing` is the hit-tested target, `lastTime`/`lastPrice`
      // is the previous mousemove's position so each frame applies an
      // incremental delta rather than re-deriving an absolute offset.
      dragState: null,
      // Trade-overlay primitives (`scChartSetTrades`) — separate from
      // `drawings` since they're cleared independently on every backtest
      // re-run, not on every symbol/interval switch. `tradeDensity` is
      // "all" or "markers-only" (the "Rectangles" toolbar toggle).
      tradePrimitives: [],
      allTrades: [],
      tradeDensity: "all",
      statsEl: null,
      liveOrderMarker: null,
      tradeMarkers: [],
      tooltipEl: makeOverlayDiv(el, "sc-trade-tooltip"),
    };
    entry.tooltipEl.style.display = "none";
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
      refreshVisibleTrades(containerId);
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
    // Trade-exit tooltip — `setMarkers` has no native hover support (a
    // plain shape-and-text API), so full trade context on hover needs its
    // own crosshair-proximity hit test against the closed trades' exit
    // times, same pattern as TradingView's own Tooltip/Delta Tooltip
    // plugin examples.
    chart.subscribeCrosshairMove(function (param) {
      var live = window.scCharts[containerId];
      if (!live || !live.tooltipEl) return;
      var trade = null;
      if (param && param.time !== undefined && live.allTrades) {
        trade = live.allTrades.find(function (t) { return tradeTime(t.exit_time) === param.time; });
      }
      if (trade && param.point) {
        live.tooltipEl.style.display = "block";
        live.tooltipEl.style.left = (param.point.x + 14) + "px";
        live.tooltipEl.style.top = (param.point.y + 14) + "px";
        live.tooltipEl.textContent = formatTradeTooltip(trade);
      } else {
        live.tooltipEl.style.display = "none";
      }
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
        live.drawings.push({ type: "priceline", ref: line, price: price });
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
        refreshAllMarkers(containerId);
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
        live.drawings.push({
          type: "lineseries", ref: trendSeries,
          t1: p1.time, v1: p1.value, t2: p2.time, v2: p2.value,
        });
      } else if (tool === "rect") {
        var rEl = makeOverlayDiv(el, "sc-rect");
        var dividerEl = document.createElement("div");
        dividerEl.className = "sc-rect-divider";
        rEl.appendChild(dividerEl);
        var handle1 = makeOverlayDiv(el, "sc-rect-handle");
        var handle2 = makeOverlayDiv(el, "sc-rect-handle");
        live.drawings.push({
          type: "rect", t1: p1.time, p1: p1.value, t2: p2.time, p2: p2.value,
          el: rEl, handle1: handle1, handle2: handle2,
        });
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
        live.drawings.push({
          type: "measure", ref: measureSeries, el: labelEl,
          t1: p1.time, v1: p1.value, t2: p2.time, v2: p2.value,
        });
      }
    });
    // Native DOM mouse events, not `subscribeClick` — dragging needs every
    // intermediate mousemove, which the library's own click/crosshair
    // subscriptions don't provide. Only takes effect in Cursor mode
    // (`activeTool === null`); while a drawing tool is armed, clicks are
    // already claimed by `subscribeClick` above for placing new shapes.
    el.addEventListener("mousedown", function (ev) {
      var live = window.scCharts[containerId];
      if (!live || live.activeTool) return;
      var rect = el.getBoundingClientRect();
      var x = ev.clientX - rect.left, y = ev.clientY - rect.top;
      // Resize handles take priority over whole-shape dragging — a handle
      // sits inside the rectangle's own bounding box, so without this
      // check first, grabbing a corner would just move the whole shape.
      var handleHit = hitTestRectHandle(live, x, y);
      if (handleHit) {
        live.dragState = { drawing: handleHit.drawing, corner: handleHit.corner };
        live.chart.applyOptions({ handleScroll: false, handleScale: false });
        el.style.cursor = "nwse-resize";
        ev.preventDefault();
        return;
      }
      var hit = hitTestDrawing(live, x, y);
      if (!hit) return;
      var time = live.chart.timeScale().coordinateToTime(x);
      var price = live.series.coordinateToPrice(y);
      if (time === null || price === null || price === undefined) return;
      live.dragState = { drawing: hit, lastTime: time, lastPrice: price };
      live.chart.applyOptions({ handleScroll: false, handleScale: false });
      el.style.cursor = "move";
      ev.preventDefault();
    });
    el.addEventListener("mousemove", function (ev) {
      var live = window.scCharts[containerId];
      if (!live || !live.dragState) return;
      var rect = el.getBoundingClientRect();
      var x = ev.clientX - rect.left, y = ev.clientY - rect.top;
      var time = live.chart.timeScale().coordinateToTime(x);
      var price = live.series.coordinateToPrice(y);
      if (time === null || price === null || price === undefined) return;
      // Resize: the grabbed corner snaps exactly to the mouse (absolute),
      // not delta-applied like a whole-shape move — that's what makes it
      // stretch from that corner instead of translating the whole shape.
      if (live.dragState.corner) {
        var rd = live.dragState.drawing;
        if (live.dragState.corner === 1) { rd.t1 = time; rd.p1 = price; }
        else { rd.t2 = time; rd.p2 = price; }
        repositionOverlays(containerId);
        return;
      }
      var deltaTime = time - live.dragState.lastTime;
      var deltaPrice = price - live.dragState.lastPrice;
      if (deltaTime !== 0 || deltaPrice !== 0) {
        moveDrawingBy(live, live.dragState.drawing, deltaTime, deltaPrice);
        live.dragState.lastTime = time;
        live.dragState.lastPrice = price;
      }
    });
    function endDrag() {
      var live = window.scCharts[containerId];
      if (!live || !live.dragState) return;
      live.dragState = null;
      live.chart.applyOptions({ handleScroll: true, handleScale: true });
      el.style.cursor = "default";
    }
    el.addEventListener("mouseup", endDrag);
    el.addEventListener("mouseleave", endDrag);
    window.scCharts[containerId] = entry;
  };
  window.scChartSetData = function (containerId, candles) {
    var entry = window.scCharts[containerId];
    if (!entry) return;
    // A new symbol/interval invalidates every existing drawing's time
    // coordinates, so clear them rather than leave stale lines floating
    // over an unrelated instrument's candles.
    window.scChartClearDrawings(containerId);
    window.scChartClearTrades(containerId);
    window.scChartClearStats(containerId);
    window.scChartSetLiveOrderMarker(containerId, null);
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
    // Rectangle resize handles only make sense in Cursor mode — toggles
    // their visibility immediately rather than waiting for the next pan/
    // zoom/resize to call this incidentally.
    repositionOverlays(containerId);
  };
  window.scChartClearDrawings = function (containerId) {
    var entry = window.scCharts[containerId];
    if (!entry) return;
    entry.drawings.forEach(function (d) {
      if (d.type === "priceline") entry.series.removePriceLine(d.ref);
      else if (d.type === "lineseries" || d.type === "measure") entry.chart.removeSeries(d.ref);
      else if (d.type === "vline") d.el.remove();
      else if (d.type === "rect") { d.el.remove(); d.handle1.remove(); d.handle2.remove(); }
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
    refreshAllMarkers(containerId);
    entry.pendingPoint = null;
  };
  // `setMarkers` always replaces the whole array, so the manually-drawn
  // buy/sell markers, the one live-order entry marker (Stage 10), and the
  // closed-trade exit markers (Stage 11 gap fix) all have to be merged and
  // re-sent together every time any one of them changes.
  function refreshAllMarkers(containerId) {
    var entry = window.scCharts[containerId];
    if (!entry) return;
    var markers = entry.markers.slice().concat(entry.tradeMarkers || []);
    if (entry.liveOrderMarker) markers.push(entry.liveOrderMarker);
    markers.sort(function (a, b) { return a.time - b.time; });
    entry.series.setMarkers(markers);
  }
  // Entry marker only — a live trade rectangle or SL/TP lines would need
  // `OrderUpdatePayload` to carry those prices, which it doesn't yet (see
  // `ROADMAP.md` Phase 7, Stage 10).
  window.scChartSetLiveOrderMarker = function (containerId, marker) {
    var entry = window.scCharts[containerId];
    if (!entry) return;
    entry.liveOrderMarker = marker
      ? {
          time: tradeTime(marker.time),
          position: marker.side === "buy" ? "belowBar" : "aboveBar",
          color: marker.side === "buy" ? entry.colors.up : entry.colors.down,
          shape: marker.side === "buy" ? "arrowUp" : "arrowDown",
          text: marker.side === "buy" ? "Buy" : "Sell",
        }
      : null;
    refreshAllMarkers(containerId);
  };
  // --- Trade overlay (backtest closed trades) ---------------------------
  // Canvas-based Series Primitives (`series.attachPrimitive`), not DOM
  // overlays like the manual drawing tools above — TradingView's own
  // plugin examples (rectangle-drawing-tool, vertical-line, tooltip) all
  // use this technique, and it's the one that doesn't fall over at
  // hundreds of trades the way hundreds of repositioned DOM nodes would.
  function tradeTime(iso) {
    return Math.floor(new Date(iso).getTime() / 1000);
  }
  function tradePriceExtent(trade) {
    var vals = [trade.entry_price, trade.exit_price];
    if (trade.stop_loss !== null && trade.stop_loss !== undefined) vals.push(trade.stop_loss);
    if (trade.take_profit !== null && trade.take_profit !== undefined) vals.push(trade.take_profit);
    return { min: Math.min.apply(null, vals), max: Math.max.apply(null, vals) };
  }
  function formatTradeLabel(trade) {
    var parts = [trade.side === "long" ? "LONG" : "SHORT"];
    if (trade.rr !== null && trade.rr !== undefined) parts.push("RR: " + trade.rr.toFixed(2));
    parts.push("E: " + trade.entry_price);
    if (trade.stop_loss !== null && trade.stop_loss !== undefined) parts.push("SL: " + trade.stop_loss);
    if (trade.take_profit !== null && trade.take_profit !== undefined) parts.push("TP: " + trade.take_profit);
    return parts.join(" | ");
  }
  // Mirrors Rust's `format_trade_tooltip` — kept in sync by hand, same as
  // every other JS/Rust pair in this file, since Series Primitives and
  // crosshair hit-testing can't be driven from Rust directly.
  function formatTradeTooltip(trade) {
    var outcomeText = trade.outcome === "win" ? "Win" : (trade.outcome === "loss" ? "Loss" : "Breakeven");
    var sign = trade.pnl_pct >= 0 ? "+" : "";
    return outcomeText + " " + sign + trade.pnl_pct.toFixed(2) + "% | " + formatTradeLabel(trade);
  }
  // Exit markers via the existing `setMarkers` mechanism (cheap — unlike
  // the rectangle/line primitives, these stay attached for every trade
  // regardless of the density cap, since "Rectangles off" is meant to
  // leave them visible). Marker color encodes *outcome* (win/loss),
  // independent of the rectangle's side color — so a losing Long and a
  // losing Short read identically, matching how FreqUI does it.
  function buildTradeMarkers(trades, colors) {
    return trades.map(function (trade) {
      var color = trade.outcome === "win" ? colors.up : (trade.outcome === "loss" ? colors.down : colors.text);
      return {
        time: tradeTime(trade.exit_time),
        position: "inBar",
        color: color,
        shape: "circle",
        text: trade.outcome === "win" ? "W" : (trade.outcome === "loss" ? "L" : "BE"),
      };
    });
  }
  // One primitive instance per trade rectangle. `attached`/`detached` are
  // the standard `ISeriesPrimitive` lifecycle hooks this build exposes
  // (confirmed via `attachPrimitive`/`detachPrimitive` in the vendored
  // bundle) — they hand back the chart/series refs needed for coordinate
  // conversion, since a primitive itself has no access to either.
  function makeTradeRectanglePrimitive(trade, colors) {
    var chartRef = null;
    var seriesRef = null;
    var label = formatTradeLabel(trade);
    var sideColor = trade.side === "long" ? colors.up : colors.down;
    var borderWidth = trade.rr !== null && trade.rr !== undefined && trade.rr > 3 ? 3 : 2;
    return {
      attached: function (p) { chartRef = p.chart; seriesRef = p.series; },
      detached: function () { chartRef = null; seriesRef = null; },
      updateAllViews: function () {},
      paneViews: function () {
        return [{
          renderer: {
            draw: function (target) {
              target.useBitmapCoordinateSpace(function (scope) {
                if (!chartRef || !seriesRef) return;
                var x1 = chartRef.timeScale().timeToCoordinate(tradeTime(trade.entry_time));
                var x2 = chartRef.timeScale().timeToCoordinate(tradeTime(trade.exit_time));
                var extent = tradePriceExtent(trade);
                var yTop = seriesRef.priceToCoordinate(extent.max);
                var yBot = seriesRef.priceToCoordinate(extent.min);
                if (x1 === null || x2 === null || yTop === null || yBot === null) return;
                var ctx = scope.context;
                var hr = scope.horizontalPixelRatio;
                var vr = scope.verticalPixelRatio;
                var left = Math.min(x1, x2) * hr;
                var right = Math.max(x1, x2) * hr;
                var top = yTop * vr;
                var bottom = yBot * vr;
                ctx.save();
                ctx.globalAlpha = 0.2;
                ctx.fillStyle = sideColor;
                ctx.fillRect(left, top, right - left, bottom - top);
                ctx.globalAlpha = 1;
                ctx.lineWidth = borderWidth * hr;
                ctx.strokeStyle = sideColor;
                ctx.strokeRect(left, top, right - left, bottom - top);
                ctx.fillStyle = colors.text;
                ctx.font = (11 * vr) + "px sans-serif";
                ctx.textBaseline = "top";
                ctx.fillText(label, left + 4 * hr, top + 4 * vr);
                ctx.restore();
              });
            },
          },
        }];
      },
      priceAxisViews: function () { return []; },
      timeAxisViews: function () { return []; },
    };
  }
  // Used 3x per trade (entry/SL/TP) — a short segment bounded to just the
  // entry-time..exit-time range, same "short segment, not infinite line"
  // approach already used for the manual Fibonacci tool above.
  function makeTradeLinePrimitive(trade, price, color, dashed) {
    var chartRef = null;
    var seriesRef = null;
    return {
      attached: function (p) { chartRef = p.chart; seriesRef = p.series; },
      detached: function () { chartRef = null; seriesRef = null; },
      updateAllViews: function () {},
      paneViews: function () {
        return [{
          renderer: {
            draw: function (target) {
              target.useBitmapCoordinateSpace(function (scope) {
                if (!chartRef || !seriesRef || price === null || price === undefined) return;
                var x1 = chartRef.timeScale().timeToCoordinate(tradeTime(trade.entry_time));
                var x2 = chartRef.timeScale().timeToCoordinate(tradeTime(trade.exit_time));
                var y = seriesRef.priceToCoordinate(price);
                if (x1 === null || x2 === null || y === null) return;
                var ctx = scope.context;
                var hr = scope.horizontalPixelRatio;
                var vr = scope.verticalPixelRatio;
                ctx.save();
                ctx.strokeStyle = color;
                ctx.lineWidth = hr;
                if (dashed) ctx.setLineDash([4 * hr, 4 * hr]);
                ctx.beginPath();
                ctx.moveTo(Math.min(x1, x2) * hr, y * vr);
                ctx.lineTo(Math.max(x1, x2) * hr, y * vr);
                ctx.stroke();
                ctx.restore();
              });
            },
          },
        }];
      },
      priceAxisViews: function () { return []; },
      timeAxisViews: function () { return []; },
    };
  }
  var TRADE_DENSITY_CAP = 500;
  // Filters to trades overlapping the chart's currently visible time
  // range, capped at the `cap` most-recent (by exit time) — same logic as
  // `pages::chart::visible_trades` in Rust, kept in sync by hand since this
  // half runs only in JS (Series Primitives can't be driven from Rust
  // directly). Recency-within-visible-range, not a hard global drop, per
  // the Pine-Script/FreqUI/vectorbt precedent in `ROADMAP.md` Phase 7.
  function visibleTradesJs(trades, fromSec, toSec, cap) {
    var overlapping = trades.filter(function (t) {
      var t1 = tradeTime(t.entry_time);
      var t2 = tradeTime(t.exit_time);
      return t1 <= toSec && t2 >= fromSec;
    });
    overlapping.sort(function (a, b) { return tradeTime(b.exit_time) - tradeTime(a.exit_time); });
    return overlapping.slice(0, cap);
  }
  function attachTradePrimitives(entry, trades) {
    var primitives = [];
    trades.forEach(function (trade) {
      var rect = makeTradeRectanglePrimitive(trade, entry.colors);
      entry.series.attachPrimitive(rect);
      primitives.push(rect);

      var sideColor = trade.side === "long" ? entry.colors.up : entry.colors.down;
      var entryLine = makeTradeLinePrimitive(trade, trade.entry_price, sideColor, false);
      entry.series.attachPrimitive(entryLine);
      primitives.push(entryLine);

      if (trade.stop_loss !== null && trade.stop_loss !== undefined) {
        var slLine = makeTradeLinePrimitive(trade, trade.stop_loss, entry.colors.down, true);
        entry.series.attachPrimitive(slLine);
        primitives.push(slLine);
      }
      if (trade.take_profit !== null && trade.take_profit !== undefined) {
        var tpLine = makeTradeLinePrimitive(trade, trade.take_profit, entry.colors.up, true);
        entry.series.attachPrimitive(tpLine);
        primitives.push(tpLine);
      }
    });
    entry.tradePrimitives = primitives;
  }
  // Recomputes which trades have primitives attached for the chart's
  // current visible range — a no-op below the density cap (every trade
  // just stays attached), only filters once the set is too large to draw
  // all at once. Respects `entry.cursorTime` when set (playback mode).
  function refreshVisibleTrades(containerId) {
    var entry = window.scCharts[containerId];
    if (!entry || !entry.allTrades || entry.tradeDensity === "markers-only") return;
    var relevantTrades = entry.cursorTime
      ? entry.allTrades.filter(function (t) { return t.entry_time <= entry.cursorTime; })
      : entry.allTrades;
    if (relevantTrades.length <= TRADE_DENSITY_CAP) return;
    var range = entry.chart.timeScale().getVisibleRange();
    if (!range) return;
    detachTradePrimitives(entry);
    attachTradePrimitives(entry, visibleTradesJs(relevantTrades, range.from, range.to, TRADE_DENSITY_CAP));
  }
  // Detaches rectangle/line primitives only — leaves `allTrades`/
  // `tradeMarkers` untouched, since "Rectangles off" (the density toggle)
  // is meant to keep exit markers visible, only drop the expensive part.
  function detachTradePrimitives(entry) {
    (entry.tradePrimitives || []).forEach(function (p) { entry.series.detachPrimitive(p); });
    entry.tradePrimitives = [];
  }
  // Single entry point: clears any previously-attached trade primitives,
  // then attaches a rectangle + up to 3 lines (entry/SL/TP) per trade —
  // mirrors the `scChartSetData`/`scChartClearDrawings` naming already
  // used above. Above `TRADE_DENSITY_CAP` trades, only the most-recent
  // ones in the currently visible range get primitives; the rest follow as
  // the user pans/zooms, via `refreshVisibleTrades` below. Exit markers are
  // built for every trade regardless of the cap — cheap, and meant to stay
  // visible even with "Rectangles" toggled off.
  //
  // `cursorTime` (ISO string or null) — when set, only trades with
  // `exit_time <= cursorTime` are rendered as closed trades (Loop 6i
  // playback). Trades with `entry_time <= cursorTime < exit_time` (the
  // "open at the cursor" case) are shown with a rectangle but no exit
  // marker. `null` means "show all trades" (no playback active).
  window.scChartSetTrades = function (containerId, closedTrades, cursorTime) {
    var entry = window.scCharts[containerId];
    if (!entry) return;
    window.scChartClearTrades(containerId);
    entry.allTrades = closedTrades;
    entry.cursorTime = cursorTime || null;
    // Split the trade list by cursor position: exited trades get the full
    // treatment; entered-but-not-yet-exited trades get a rectangle only.
    var exited = cursorTime
      ? closedTrades.filter(function (t) { return t.exit_time <= cursorTime; })
      : closedTrades;
    var openAtCursor = cursorTime
      ? closedTrades.filter(function (t) {
          return t.entry_time <= cursorTime && t.exit_time > cursorTime;
        })
      : [];
    entry.tradeMarkers = buildTradeMarkers(exited, entry.colors);
    refreshAllMarkers(containerId);
    if (entry.tradeDensity === "markers-only") return;
    var tradesToDraw = exited.concat(openAtCursor);
    if (tradesToDraw.length <= TRADE_DENSITY_CAP) {
      attachTradePrimitives(entry, tradesToDraw);
    } else {
      // For the density cap, prefer exited trades over open-at-cursor ones
      // (exited is more informative), then pad with open-at-cursor.
      var capped = exited.slice(0, TRADE_DENSITY_CAP)
        .concat(openAtCursor.slice(0, Math.max(0, TRADE_DENSITY_CAP - exited.length)));
      attachTradePrimitives(entry, capped);
    }
  };
  window.scChartClearTrades = function (containerId) {
    var entry = window.scCharts[containerId];
    if (!entry) return;
    detachTradePrimitives(entry);
    entry.allTrades = [];
    entry.tradeMarkers = [];
    refreshAllMarkers(containerId);
  };
  // Toggle for the "Rectangles" button: "markers-only" detaches the
  // rectangle/line primitives only, leaving the underlying trade list and
  // exit markers intact, so switching back to "all" redraws correctly
  // without needing a fresh `scChartSetTrades` call.
  window.scChartSetTradeDensity = function (containerId, density) {
    var entry = window.scCharts[containerId];
    if (!entry) return;
    entry.tradeDensity = density;
    if (density === "markers-only") {
      detachTradePrimitives(entry);
    } else if (entry.allTrades) {
      if (entry.allTrades.length <= TRADE_DENSITY_CAP) {
        attachTradePrimitives(entry, entry.allTrades);
      } else {
        refreshVisibleTrades(containerId);
      }
    }
  };
  // Stats dashboard table — a DOM overlay, not a primitive: text-heavy UI
  // is what DOM is for (stage 5's own reasoning, applied in the other
  // direction). Fixed top-right; unlike vline/rect/fib it never needs
  // `repositionOverlays` since it doesn't track any time/price coordinate.
  // Builds one "<span>label</span><b class=cls>value</b>" stats row via
  // real DOM nodes + `textContent`, not an HTML template string — `title`
  // (below) is built server-side from `strategy_id`, a name the user picks
  // when deploying a Python strategy, so it must never reach `innerHTML`
  // unescaped (was a stored-XSS hole: a `strategy_id` like
  // `<img src=x onerror=...>` used to execute in the browser of anyone
  // who viewed that backtest result).
  function appendStatsRow(parent, label, value, valueClass) {
    var row = document.createElement("div");
    row.className = "sc-stats-row";
    var labelEl = document.createElement("span");
    labelEl.textContent = label;
    var valueEl = document.createElement("b");
    if (valueClass) valueEl.className = valueClass;
    valueEl.textContent = value;
    row.appendChild(labelEl);
    row.appendChild(valueEl);
    parent.appendChild(row);
  }
  window.scChartSetStats = function (containerId, statsRow) {
    var entry = window.scCharts[containerId];
    if (!entry) return;
    window.scChartClearStats(containerId);
    var el = document.getElementById(containerId);
    var table = makeOverlayDiv(el, "sc-stats-table");
    var title = document.createElement("div");
    title.className = "sc-stats-title";
    title.textContent = statsRow.title;
    table.appendChild(title);
    appendStatsRow(table, "Trades", statsRow.total_trades, null);
    appendStatsRow(table, "Win rate", statsRow.win_rate_pct, statsRow.win_rate_class);
    appendStatsRow(table, "PnL", statsRow.total_pnl, statsRow.total_pnl_class);
    appendStatsRow(table, "Avg RR", statsRow.avg_rr, null);
    entry.statsEl = table;
  };
  window.scChartClearStats = function (containerId) {
    var entry = window.scCharts[containerId];
    if (!entry || !entry.statsEl) return;
    entry.statsEl.remove();
    entry.statsEl = null;
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
