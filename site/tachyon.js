// "What is a tachyon?" page: reading levels and the interactive diagrams.
// Progressive enhancement: without this file the Standard text renders in full, and the level
// switch still works through CSS :has(). This adds: remembering the level (localStorage
// "tachyon-level"), ?level=eli5|eli13|standard|phd for shared links (not saved), keeping the
// reader's place when the level changes, and the diagrams. The 3D experiment loads Three.js on
// demand (tachyon-3d.js), only when its figure is about to scroll into view and WebGL works.
(function () {
  "use strict";

  var doc = document;
  doc.documentElement.classList.add("js");
  var reduced = window.matchMedia("(prefers-reduced-motion: reduce)");
  var LEVELS = ["eli5", "eli13", "standard", "phd"];
  var STORE = "tachyon-level";
  var level = "standard";

  // Helpers
  function $(s, r) { return (r || doc).querySelector(s); }
  function $all(s, r) { return Array.prototype.slice.call((r || doc).querySelectorAll(s)); }
  function f(v, d) { return (Math.round(v * Math.pow(10, d)) / Math.pow(10, d)).toFixed(d); }
  function pct(b) { return Math.round(b * 100) + "%"; }
  function esc(s) { return String(s).replace(/&/g, "&amp;").replace(/</g, "&lt;"); }
  function stored() { try { return localStorage.getItem(STORE); } catch (e) { return null; } }
  function remember(v) { try { localStorage.setItem(STORE, v); } catch (e) { /* private mode */ } }
  function pick(map) { return map[level] || map.standard; }
  function live(el, html) {
    var span = el.querySelector(".lvt-live");
    if (span && span.innerHTML !== html) span.innerHTML = html;
  }
  // Coalesce slider input into one update per frame.
  function onFrame(fn) {
    var queued = false;
    return function () {
      if (queued) return;
      queued = true;
      requestAnimationFrame(function () { queued = false; fn(); });
    };
  }

  // ---------------------------------------------------------------------------------------
  // Reading levels
  // ---------------------------------------------------------------------------------------
  var radios = $all('.levels input[name="level"]');
  var anchor = null;

  // The shared element (a heading or a diagram, the same at every level) nearest the reader's
  // eye line, about a third of the way down the space under the sticky bars.
  function trackAnchor() {
    var top = 140, line = top + (window.innerHeight - top) / 3, best = null, bestD = Infinity;
    var marks = $all(".explainer-body h2, .explainer-body .viz, .explainer-head h1");
    for (var i = 0; i < marks.length; i++) {
      var r = marks[i].getBoundingClientRect(), d = Math.abs(r.top - line);
      if (d < bestD) { bestD = d; best = { el: marks[i], y: r.top }; }
    }
    anchor = best;
  }
  window.addEventListener("scroll", onFrame(trackAnchor), { passive: true });

  function applyLevel(next, save) {
    if (LEVELS.indexOf(next) < 0) next = "standard";
    level = next;
    radios.forEach(function (r) { r.checked = r.value === next; });
    $all(".lvt").forEach(function (el) {
      var t = el.getAttribute("data-" + next);
      if (t != null) el.textContent = t;
    });
    if (save) remember(next);
    renderAll();
  }

  radios.forEach(function (r) {
    r.addEventListener("change", function () {
      if (!r.checked) return;
      var keep = anchor;
      applyLevel(r.value, true);
      if (keep) {
        // The new level's text has a different length: put the same heading back where it was.
        var dy = keep.el.getBoundingClientRect().top - keep.y;
        // "instant": the site sets scroll-behavior: smooth, which would visibly slide the page.
        if (Math.abs(dy) > 1) window.scrollBy({ top: dy, behavior: "instant" });
        trackAnchor();
      }
    });
  });

  // ---------------------------------------------------------------------------------------
  // 1. Energy against speed
  // ---------------------------------------------------------------------------------------
  var energy = (function () {
    var fig = $("#viz-energy");
    if (!fig) return null;
    var svg = $("svg", fig), sb = $("#e-brady"), st = $("#e-tachy");
    var ob = $("#e-brady-out"), ot = $("#e-tachy-out"), read = $("#energy-read");
    var X0 = 64, X1 = 612, Y0 = 318, Y1 = 34, BMAX = 4, EMAX = 8;
    function sx(b) { return X0 + (b / BMAX) * (X1 - X0); }
    function sy(e) { return Y0 - (Math.min(e, EMAX) / EMAX) * (Y0 - Y1); }
    function gB(b) { return 1 / Math.sqrt(1 - b * b); }
    function gT(b) { return 1 / Math.sqrt(b * b - 1); }
    // Each branch is sampled densely near the wall, where it is steep, and ends exactly where
    // it leaves the top of the plot (γ = EMAX), so both visibly run off to infinity.
    function path(from, to, g, denseAt) {
      var d = "", n = 220;
      for (var i = 0; i <= n; i++) {
        var q = i / n, w = denseAt === "to" ? 1 - Math.pow(1 - q, 3) : Math.pow(q, 3);
        var b = from + (to - from) * w;
        d += (d ? "L" : "M") + f(sx(b), 1) + " " + f(sy(g(b)), 1);
      }
      return d;
    }
    var bTop = Math.sqrt(1 - 1 / (EMAX * EMAX)), tTop = Math.sqrt(1 + 1 / (EMAX * EMAX));
    var bradyPath = path(0, bTop, gB, "to"), tachyPath = path(tTop, BMAX, gT, "from");

    function labels() {
      return pick({
        eli5: { x: "how fast", y: "energy it needs", wall: "light speed", b: "normal thing", t: "tachyon" },
        eli13: { x: "speed", y: "energy (× rest energy)", wall: "c, the speed of light", b: "ordinary", t: "tachyon" },
        standard: { x: "speed v", y: "energy E, in units of |m|c²", wall: "c", b: "ordinary (m² > 0)", t: "tachyon (m² < 0)" },
        phd: { x: "β = v/c", y: "E / |m|c² = |1 − β²|^−½", wall: "β = 1", b: "bradyon", t: "tachyon" }
      });
    }

    function draw() {
      var b = sb.value / 1000, t = st.value / 1000, L = labels();
      var eb = gB(b), et = gT(t);
      var s = "";
      // grid and axes
      [1, 2, 4, 6, 8].forEach(function (e) {
        s += '<line class="grid" x1="' + X0 + '" x2="' + X1 + '" y1="' + sy(e) + '" y2="' + sy(e) + '"/>';
        s += '<text x="' + (X0 - 8) + '" y="' + (sy(e) + 4) + '" text-anchor="end">' + e + "×</text>";
      });
      [[0, "0"], [0.5, "½c"], [2, "2c"], [3, "3c"], [4, "4c"]].forEach(function (k) {
        s += '<text x="' + sx(k[0]) + '" y="' + (Y0 + 20) + '" text-anchor="middle">' + k[1] + "</text>";
      });
      s += '<line class="axis" x1="' + X0 + '" x2="' + X1 + '" y1="' + Y0 + '" y2="' + Y0 + '"/>';
      s += '<line class="axis" x1="' + X0 + '" x2="' + X0 + '" y1="' + Y0 + '" y2="' + (Y1 - 6) + '"/>';
      s += '<text x="' + X1 + '" y="' + (Y0 + 40) + '" text-anchor="end">' + esc(L.x) + " →</text>";
      s += '<text x="' + (X0 + 8) + '" y="' + (Y1 - 12) + '">↑ ' + esc(L.y) + "</text>";
      // wall at c
      s += '<rect x="' + (sx(1) - 3) + '" y="' + Y1 + '" width="6" height="' + (Y0 - Y1) + '" fill="var(--surface-sunken)"/>';
      s += '<line class="wall" x1="' + sx(1) + '" x2="' + sx(1) + '" y1="' + (Y1 - 6) + '" y2="' + Y0 + '"/>';
      s += '<text class="wall-label" x="' + sx(1) + '" y="' + (Y0 + 20) + '" text-anchor="middle">c</text>';
      s += '<text class="t-strong" x="' + (sx(1) + 8) + '" y="' + (Y1 + 6) + '">' + esc(L.wall) + "</text>";
      // curves
      s += '<path class="curve curve-brady" d="' + bradyPath + '"/>';
      s += '<path class="curve curve-tachy" d="' + tachyPath + '"/>';
      s += '<text class="t-accent" x="' + (sx(0.25)) + '" y="' + (sy(1) - 12) + '">' + esc(L.b) + "</text>";
      s += '<text class="t-violet" x="' + (sx(2.4)) + '" y="' + (sy(gT(2.4)) - 14) + '">' + esc(L.t) + "</text>";
      // the two particles, with guide lines to the axes
      var bx = sx(b), by = sy(eb), tx = sx(t), ty = sy(et);
      s += '<line class="grid" stroke-dasharray="3 4" x1="' + bx + '" x2="' + bx + '" y1="' + by + '" y2="' + Y0 + '"/>';
      s += '<line class="grid" stroke-dasharray="3 4" x1="' + tx + '" x2="' + tx + '" y1="' + ty + '" y2="' + Y0 + '"/>';
      if (eb > EMAX) s += '<text class="t-accent" x="' + (bx - 6) + '" y="' + (Y1 + 22) + '" text-anchor="end">↑ ' + f(eb, 0) + "×</text>";
      if (et > EMAX) s += '<text class="t-violet" x="' + (tx + 6) + '" y="' + (Y1 + 22) + '">↑ ' + f(et, 0) + "×</text>";
      s += '<circle class="dot dot-brady" data-drag="b" r="10" cx="' + f(bx, 1) + '" cy="' + f(by, 1) + '"/>';
      s += '<circle class="dot dot-tachy" data-drag="t" r="10" cx="' + f(tx, 1) + '" cy="' + f(ty, 1) + '"/>';
      svg.innerHTML = '<title id="energy-title">' + $("#energy-title").textContent + '</title><desc id="energy-desc">' + esc($("#energy-desc").textContent) + "</desc>" + s;

      ob.innerHTML = f(b, b > 0.99 ? 3 : 2) + "<i>c</i>";
      ot.innerHTML = f(t, 2) + "<i>c</i>";
      live(read, pick({
        eli5: "The normal thing at " + pct(b) + " of light speed needs " + f(eb, 1) + " times the energy it has standing still" + (b > 0.98 ? ", and it gets worse very fast from here." : ".") +
          " The tachyon at " + f(t, 1) + " times light speed has " + (et < 1 ? "less" : "more") + " energy the " + (et < 1 ? "faster" : "closer to light speed") + " it goes.",
        eli13: "Ordinary particle at " + f(b, 3) + "c: γ = " + f(eb, 2) + ", so " + f(eb, 2) + "× its rest energy. Tachyon at " + f(t, 2) + "c: energy " + f(et, 2) + " (in units of |m|c²); slower means more energy, faster means less.",
        standard: "Ordinary particle at " + f(b, 3) + "c: E = " + f(eb, 2) + " mc². Tachyon at " + f(t, 2) + "c: E = " + f(et, 2) + " |m|c². Neither can reach c: both energies grow without limit there.",
        phd: "β = " + f(b, 3) + ": γ = " + f(eb, 3) + ". Tachyon β = " + f(t, 3) + ": E = " + f(et, 3) + " |m|c², p = " + f(t * et, 3) + " |m|c, E² − p²c² = −m²c⁴ (|m| = 1)."
      }));
    }

    // Drag a dot along its branch; the slider stays the single source of truth.
    var dragging = null;
    function toBeta(e) {
      var pt = svg.createSVGPoint(); pt.x = e.clientX; pt.y = e.clientY;
      var p = pt.matrixTransform(svg.getScreenCTM().inverse());
      return (p.x - X0) / (X1 - X0) * BMAX;
    }
    svg.addEventListener("pointerdown", function (e) {
      var k = e.target.getAttribute && e.target.getAttribute("data-drag");
      if (!k) return;
      dragging = k; svg.setPointerCapture(e.pointerId); e.preventDefault();
    });
    svg.addEventListener("pointermove", function (e) {
      if (!dragging) return;
      var b = toBeta(e), input = dragging === "b" ? sb : st;
      var v = Math.round(b * 1000);
      input.value = Math.max(+input.min, Math.min(+input.max, v));
      update();
    });
    function end() { dragging = null; }
    svg.addEventListener("pointerup", end);
    svg.addEventListener("pointercancel", end);
    var update = onFrame(draw);
    sb.addEventListener("input", update);
    st.addEventListener("input", update);
    return { draw: draw };
  })();

  // ---------------------------------------------------------------------------------------
  // 2. Spacetime diagram (active view: the observer's axes are square, the events move)
  // ---------------------------------------------------------------------------------------
  var cones = (function () {
    var fig = $("#viz-cones");
    if (!fig) return null;
    var svg = $("svg", fig), sl = $("#c-beta"), out = $("#c-beta-out"), axes = $("#c-axes"), read = $("#cones-read");
    var W = 640, H = 420, S = 34, OX = 232, OY = 250;
    var A = [0, 0], B = [3, 1], C = [1, 3]; // [x, t]: light-seconds, seconds, at rest
    function X(x) { return OX + x * S; }
    function Y(t) { return OY - t * S; }
    function boost(p, b) {
      var g = 1 / Math.sqrt(1 - b * b);
      return [g * (p[0] - b * p[1]), g * (p[1] - b * p[0])];
    }
    function clipLine(x0, t0, dx, dt) {
      // a long segment through (x0, t0) along (dx, dt); the svg clipPath trims it
      return '"' + X(x0 - dx * 30) + '" y1="' + Y(t0 - dt * 30) + '" x2="' + X(x0 + dx * 30) + '" y2="' + Y(t0 + dt * 30) + '"';
    }
    function hyper(kind) {
      var d = "", n = 120;
      for (var i = 0; i <= n; i++) {
        var s = -6 + 12 * i / n, x, t;
        if (kind === "space") { x = Math.sqrt(8 + s * s); t = s; } else { t = Math.sqrt(8 + s * s); x = s; }
        d += (d ? "L" : "M") + f(X(x), 1) + " " + f(Y(t), 1);
      }
      return d;
    }
    var hB = hyper("space"), hC = hyper("time");

    function draw() {
      var b = sl.value / 100, g = 1 / Math.sqrt(1 - b * b);
      var Bp = boost(B, b), Cp = boost(C, b), first = Bp[1] < -1e-9;
      var L = pick({
        eli5: { x: "distance →", t: "↑ time", now: "the moment of A", a: "A", b: "B", c: "C", flip: "B came first!", lt: "light" },
        eli13: { x: "distance (light-seconds) →", t: "↑ time (seconds)", now: "same moment as A", a: "A", b: "B", c: "C", flip: "B before A", lt: "light" },
        standard: { x: "x′ (light-seconds) →", t: "↑ t′ (s)", now: "simultaneous with A", a: "A", b: "B (spacelike)", c: "C (timelike)", flip: "B precedes A in this frame", lt: "light cone" },
        phd: { x: "x′ [c·s]", t: "t′ [s]", now: "t′ = 0", a: "A", b: "B: s² = −8", c: "C: s² = +8", flip: "Δt′(AB) < 0", lt: "s² = 0" }
      });
      var s = '<defs><clipPath id="cones-clip"><rect x="0" y="0" width="' + W + '" height="' + H + '" rx="10"/></clipPath></defs><g clip-path="url(#cones-clip)">';
      for (var i = -7; i <= 12; i++) s += '<line class="grid" x1="' + X(i) + '" x2="' + X(i) + '" y1="0" y2="' + H + '"/>';
      for (var j = -5; j <= 8; j++) s += '<line class="grid" x1="0" x2="' + W + '" y1="' + Y(j) + '" y2="' + Y(j) + '"/>';
      // light cone of A
      s += '<polygon class="cone-fill" points="' + X(0) + "," + Y(0) + " " + X(12) + "," + Y(12) + " " + X(-12) + "," + Y(12) + '"/>';
      s += '<polygon class="cone-fill" points="' + X(0) + "," + Y(0) + " " + X(12) + "," + Y(-12) + " " + X(-12) + "," + Y(-12) + '"/>';
      s += '<line class="light" x1=' + clipLine(0, 0, 1, 1) + "/>";
      s += '<line class="light" x1=' + clipLine(0, 0, 1, -1) + "/>";
      s += '<line class="axis" x1="' + X(0) + '" x2="' + X(0) + '" y1="0" y2="' + H + '"/>';
      s += '<line class="now" x1="0" x2="' + W + '" y1="' + Y(0) + '" y2="' + Y(0) + '"/>';
      if (axes.checked) {
        // the rest frame's axes, seen by the moving observer: x = 0 is x′ = −βt′, t = 0 is t′ = −βx′
        s += '<line class="frame-axis" x1=' + clipLine(0, 0, -b, 1) + "/>";
        s += '<line class="frame-axis" x1=' + clipLine(0, 0, 1, -b) + "/>";
        var ta = [-b * 6.4, 6.4], xa = [11, -b * 11];
        s += '<text x="' + (X(ta[0]) + 6) + '" y="' + (Y(ta[1]) + 4) + '">t (rest frame)</text>';
        s += '<text x="' + (X(xa[0]) - 4) + '" y="' + (Y(xa[1]) - 6) + '" text-anchor="end">x (rest frame)</text>';
      }
      s += '<path class="hyper" d="' + hB + '"/><path class="hyper" d="' + hC + '"/>';
      s += '<line class="tachy-line" x1="' + X(0) + '" y1="' + Y(0) + '" x2="' + X(Bp[0]) + '" y2="' + Y(Bp[1]) + '"/>';
      s += '<line class="brady-line" x1="' + X(0) + '" y1="' + Y(0) + '" x2="' + X(Cp[0]) + '" y2="' + Y(Cp[1]) + '"/>';
      s += '<circle class="event ev-a" r="8" cx="' + X(0) + '" cy="' + Y(0) + '"/>';
      s += '<circle class="event ev-b" r="8" cx="' + f(X(Bp[0]), 1) + '" cy="' + f(Y(Bp[1]), 1) + '"/>';
      s += '<circle class="event ev-c" r="8" cx="' + f(X(Cp[0]), 1) + '" cy="' + f(Y(Cp[1]), 1) + '"/>';
      s += "</g>";
      s += '<text class="t-strong" x="' + (X(0) - 12) + '" y="' + (Y(0) - 12) + '" text-anchor="end">' + L.a + "</text>";
      s += '<text class="t-violet" x="' + (X(Bp[0]) + 12) + '" y="' + (Y(Bp[1]) + 5) + '">' + esc(L.b) + "</text>";
      s += '<text class="t-accent" x="' + (X(Cp[0]) + 12) + '" y="' + (Y(Cp[1]) + 5) + '">' + esc(L.c) + "</text>";
      s += '<text x="' + (W - 8) + '" y="' + (Y(0) - 8) + '" text-anchor="end">' + esc(L.now) + "</text>";
      s += '<text x="' + (X(3.6)) + '" y="' + (Y(4.2)) + '">' + esc(L.lt) + "</text>";
      s += '<text x="' + (W - 8) + '" y="' + (H - 10) + '" text-anchor="end">' + esc(L.x) + "</text>";
      s += '<text x="' + (X(0) + 8) + '" y="18">' + esc(L.t) + "</text>";
      if (first) s += '<text class="t-warn" x="' + (X(Bp[0]) + 12) + '" y="' + (Y(Bp[1]) + 24) + '">' + esc(L.flip) + "</text>";
      svg.innerHTML = "<title>" + esc($("title", svg) ? $("title", svg).textContent : "") + "</title>" + s;

      out.innerHTML = (b >= 0 ? "" : "−") + f(Math.abs(b), 2) + "<i>c</i>";
      var tb = Bp[1], tc = Cp[1];
      var dir = b > 0 ? "towards B" : b < 0 ? "away from B" : "";
      live(read, pick({
        eli5: b === 0 ? "Standing still: A happens first, then B a second later, then C." :
          "Flying " + dir + " at " + pct(Math.abs(b)) + " of light speed. " + (tb < -1e-9 ? "<strong>For you, B happened before A!</strong>" : Math.abs(tb) < 1e-9 ? "For you, A and B happen at exactly the same moment." : "A still happens before B.") + " C always comes after A.",
        eli13: b === 0 ? "At rest: A at 0 s, B at 1 s, C at 3 s." :
          "Moving " + dir + " at " + f(Math.abs(b), 2) + "c: B happens " + f(Math.abs(tb), 2) + " s " + (tb < -1e-9 ? "<strong>before</strong>" : "after") + " A. C happens " + f(tc, 2) + " s after A, at every speed.",
        standard: "Observer at " + (b < 0 ? "−" : "") + f(Math.abs(b), 2) + "c: Δt′(A→B) = " + f(tb, 2) + " s" + (tb < -1e-9 ? ", so <strong>B precedes A</strong>" : "") + "; Δt′(A→C) = +" + f(tc, 2) + " s. The order of B flips at v = c/3; the order of C never flips.",
        phd: "β = " + f(b, 2) + ", γ = " + f(g, 3) + ": Δt′(AB) = " + f(tb, 3) + " s, Δx′(AB) = " + f(Bp[0], 3) + "; Δt′(AC) = " + f(tc, 3) + " s. Invariants s²(AB) = −8, s²(AC) = +8 (c = 1)."
      }));
    }

    // Drag across the diagram to change the observer's speed.
    var startX = null, startV = 0;
    svg.addEventListener("pointerdown", function (e) { startX = e.clientX; startV = +sl.value; svg.setPointerCapture(e.pointerId); });
    svg.addEventListener("pointermove", function (e) {
      if (startX == null) return;
      var w = svg.getBoundingClientRect().width;
      sl.value = Math.max(-80, Math.min(80, Math.round(startV + (e.clientX - startX) / w * 160)));
      update();
    });
    function end() { startX = null; }
    svg.addEventListener("pointerup", end);
    svg.addEventListener("pointercancel", end);
    var update = onFrame(draw);
    sl.addEventListener("input", update);
    axes.addEventListener("change", update);
    return { draw: draw };
  })();

  // ---------------------------------------------------------------------------------------
  // 3. Cherenkov cone
  // ---------------------------------------------------------------------------------------
  var cher = (function () {
    var fig = $("#viz-cher");
    if (!fig) return null;
    var svg = $("svg", fig), sl = $("#ch-beta"), out = $("#ch-beta-out"), read = $("#cher-read");
    var media = $all('input[name="medium"]', fig);
    var W = 640, H = 320, PX = 540, PY = 160, DT = 0.1, SC = 470, N = 9;
    function n() { for (var i = 0; i < media.length; i++) if (media[i].checked) return +media[i].value; return 1.333; }

    function draw() {
      var b = sl.value / 100, idx = n(), on = b * idx > 1;
      var name = idx === 1 ? "vacuum" : idx > 1.4 ? "glass" : "water";
      var s = '<defs><clipPath id="cher-clip"><rect x="0" y="0" width="' + W + '" height="' + H + '" rx="10"/></clipPath>' +
        '<marker id="cher-arrow" viewBox="0 0 10 10" refX="8" refY="5" markerWidth="6" markerHeight="6" orient="auto-start-reverse"><path d="M0 0L10 5L0 10z" fill="var(--viz-light)"/></marker></defs>';
      s += '<g clip-path="url(#cher-clip)"><rect class="medium" x="0" y="0" width="' + W + '" height="' + H + '"/>';
      // wavefronts emitted at earlier positions
      for (var k = 1; k <= N; k++) {
        var cx = PX - b * k * DT * SC, r = k * DT * SC / idx;
        s += '<circle class="wave" cx="' + f(cx, 1) + '" cy="' + PY + '" r="' + f(r, 1) + '"/>';
      }
      var theta = 0, alpha = 0;
      if (on) {
        alpha = Math.asin(1 / (idx * b));          // half-angle of the cone of wavefronts
        theta = Math.acos(1 / (idx * b));          // direction the light travels, from the path
        var len = 900, ex = PX - len * Math.cos(alpha), ey = len * Math.sin(alpha);
        s += '<polygon class="shock-fill" points="' + PX + "," + PY + " " + f(ex, 1) + "," + f(PY - ey, 1) + " " + f(ex, 1) + "," + f(PY + ey, 1) + '"/>';
        s += '<line class="shock" x1="' + PX + '" y1="' + PY + '" x2="' + f(ex, 1) + '" y2="' + f(PY - ey, 1) + '"/>';
        s += '<line class="shock" x1="' + PX + '" y1="' + PY + '" x2="' + f(ex, 1) + '" y2="' + f(PY + ey, 1) + '"/>';
        // two light rays leaving the front at theta to the path
        [0.38, 0.62].forEach(function (q) {
          var d = q * 330, ox = PX - d * Math.cos(alpha), oy = PY - d * Math.sin(alpha);
          s += '<line stroke="var(--viz-light)" stroke-width="2" marker-end="url(#cher-arrow)" x1="' + f(ox, 1) + '" y1="' + f(oy, 1) + '" x2="' + f(ox + 54 * Math.cos(theta), 1) + '" y2="' + f(oy - 54 * Math.sin(theta), 1) + '"/>';
        });
      }
      s += '<line class="axis" x1="20" x2="' + PX + '" y1="' + PY + '" y2="' + PY + '" stroke-dasharray="4 5"/>';
      s += '<circle class="particle" r="9" cx="' + PX + '" cy="' + PY + '"/></g>';
      var L = pick({
        eli5: { m: { water: "water", glass: "glass", vacuum: "empty space" }[name], p: "particle", c: "light’s ripples" },
        eli13: { m: name, p: "particle →", c: "light from earlier positions" },
        standard: { m: name + ", light at " + f(1 / idx, 2) + "c", p: "charged particle", c: "wavefronts" },
        phd: { m: "n = " + idx, p: "q, β", c: "spherical wavelets, radius ct/n" }
      });
      s += '<text class="t-strong" x="16" y="26">' + esc(L.m) + "</text>";
      s += '<text class="t-accent" x="' + (PX - 10) + '" y="' + (PY - 18) + '" text-anchor="end">' + esc(L.p) + "</text>";
      s += '<text x="16" y="' + (H - 14) + '">' + esc(L.c) + "</text>";
      svg.innerHTML = "<title>" + esc($("#cher-title") ? $("#cher-title").textContent : "Cherenkov cone") + "</title>" + s;

      out.innerHTML = f(b, 2) + "<i>c</i>";
      var th = theta * 180 / Math.PI, al = alpha * 180 / Math.PI, vl = 1 / idx;
      var msg;
      if (idx === 1) {
        msg = pick({
          eli5: "In empty space light is the fastest thing, and nothing ordinary can catch it, so there is never a cone. Only a tachyon could make one here.",
          eli13: "In a vacuum, light travels at c. Nothing with mass reaches c, so there is never a cone here. Only a tachyon could make one.",
          standard: "Vacuum: light travels at c, and β = " + f(b, 2) + " < 1. No massive particle can reach β = 1, so there is no Cherenkov light in a vacuum; a charged tachyon would be the exception.",
          phd: "n = 1: threshold β > 1, unreachable for m² > 0. A charged tachyon would radiate in vacuum."
        });
      } else if (!on) {
        msg = pick({
          eli5: "Too slow: the particle isn’t beating the light in the " + L.m + ", so the ripples stay inside each other. No cone.",
          eli13: "At " + f(b, 2) + "c the particle is slower than light in " + name + " (" + f(vl, 2) + "c): no cone. Speed it up past " + f(vl, 2) + "c.",
          standard: "β = " + f(b, 2) + " is below the threshold 1/n = " + f(vl, 2) + " for " + name + ": the wavefronts stay nested and no Cherenkov light is emitted.",
          phd: "nβ = " + f(idx * b, 3) + " < 1: no coherent emission."
        });
      } else {
        msg = pick({
          eli5: "The particle is beating the light in the " + L.m + "! The ripples pile up into a cone, and that’s where the blue glow comes from. Go faster and the cone gets pointier.",
          eli13: "At " + f(b, 2) + "c the particle beats light in " + name + " (" + f(vl, 2) + "c). The light leaves at " + f(th, 1) + "° to its path. Faster makes that angle wider and the cone behind it pointier.",
          standard: name.charAt(0).toUpperCase() + name.slice(1) + ", β = " + f(b, 2) + ": above the threshold " + f(vl, 2) + ". The light leaves at θ = " + f(th, 1) + "° to the path (cos θ = 1/nβ); the shock front is at 90° − θ = " + f(al, 1) + "°.",
          phd: "nβ = " + f(idx * b, 3) + ": θ_c = " + f(th, 2) + "°, shock-front half-angle 90° − θ_c = " + f(al, 2) + "°; θ_max = " + f(Math.acos(1 / idx) * 180 / Math.PI, 1) + "°."
        });
      }
      live(read, msg);
    }
    var update = onFrame(draw);
    sl.addEventListener("input", update);
    media.forEach(function (m) { m.addEventListener("change", update); });
    return { draw: draw };
  })();

  // ---------------------------------------------------------------------------------------
  // 4. The tachyonic antitelephone. A 2D spacetime diagram in Alice's frame (always drawn:
  // it is the text-equivalent picture and the fallback), plus an optional 3D view.
  // Units: c = 1, light-seconds and seconds. Bob starts L = 2 light-seconds away at t = 0.
  // ---------------------------------------------------------------------------------------
  var phone = (function () {
    var fig = $("#viz-phone");
    if (!fig) return null;
    var stage = $(".viz-stage-3d", fig), svg = $(".phone-fallback", fig);
    var sb = $("#p-beta"), su = $("#p-u"), ob = $("#p-beta-out"), ou = $("#p-u-out"), read = $("#phone-read");
    var steps = $all("[data-step]", fig), step = 0, LEN = 2, three = null;

    function model() {
      var b = sb.value / 100, uv = +su.value;
      var u = uv >= 100 ? Infinity : uv / 10;
      var t1, x1, w, t2;
      if (u === Infinity) { t1 = 0; x1 = LEN; t2 = -b * LEN; w = 1 / b; }
      else {
        t1 = LEN / (u - b); x1 = u * t1;
        var den = 1 - u * b;
        if (Math.abs(den) < 1e-9) { w = Infinity; t2 = t1; }
        else { w = (b - u) / den; t2 = t1 - x1 / w; }
      }
      // back: the reply runs backwards in Alice's time (w > 0 while heading to smaller x),
      // which happens exactly when u·β > 1, and always for instantaneous signals.
      return { b: b, u: u, t1: t1, x1: x1, t2: t2, back: u === Infinity || u * b > 1, early: t2 < -1e-9, thr: u === Infinity ? 0 : 2 * u / (1 + u * u) };
    }

    function draw2d(m) {
      var W = 640, H = 420;
      var xs = [0, LEN, m.x1, LEN + m.b * 1.5], ts = [0, m.t1, m.t2, -1, 1.5];
      var xmin = -0.8, xmax = Math.max.apply(null, xs) + 0.9;
      var tmin = Math.min.apply(null, ts) - 0.8, tmax = Math.max.apply(null, ts) + 0.8;
      var S = Math.min((W - 60) / (xmax - xmin), (H - 40) / (tmax - tmin));
      var ox = 40 - xmin * S + ((W - 60) - (xmax - xmin) * S) / 2, oy = 20 + tmax * S;
      function X(x) { return f(ox + x * S, 1); }
      function Y(t) { return f(oy - t * S, 1); }
      var L = pick({
        eli5: { a: "Alice", bob: "Bob", q: "question", r: "answer", back: "answer arrives before the question!", ok: "answer arrives after the question", now: "Bob’s “now”" },
        eli13: { a: "Alice (at home)", bob: "Bob (flying away)", q: "question", r: "reply", back: "reply arrives before she asked", ok: "reply arrives after she asked", now: "Bob’s “now”" },
        standard: { a: "Alice", bob: "Bob", q: "message", r: "reply", back: "reply received before the message was sent", ok: "reply received after the message was sent", now: "Bob’s line of simultaneity" },
        phd: { a: "A (x = 0)", bob: "B (x = L + βt)", q: "u, A→B", r: "u′ = u in B", back: "t₂ < 0", ok: "t₂ > 0", now: "t = t₁ + β(x − x₁)" }
      });
      var s = '<defs><marker id="arrow-violet" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path d="M0 0L10 5L0 10z" fill="var(--violet)"/></marker>' +
        '<marker id="arrow-cyan" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path d="M0 0L10 5L0 10z" fill="var(--cyan-strong)"/></marker>' +
        '<clipPath id="phone-clip"><rect x="0" y="0" width="' + W + '" height="' + H + '"/></clipPath></defs><g clip-path="url(#phone-clip)">';
      // Alice's light cone at the moment she asks
      var big = 40;
      s += '<line class="light" x1="' + X(0) + '" y1="' + Y(0) + '" x2="' + X(big) + '" y2="' + Y(big) + '"/>';
      s += '<line class="light" x1="' + X(0) + '" y1="' + Y(0) + '" x2="' + X(big) + '" y2="' + Y(-big) + '"/>';
      s += '<line class="now" x1="' + X(-1) + '" y1="' + Y(0) + '" x2="' + X(xmax + 2) + '" y2="' + Y(0) + '"/>';
      // worldlines
      s += '<line class="worldline" x1="' + X(0) + '" y1="' + Y(tmin - 2) + '" x2="' + X(0) + '" y2="' + Y(tmax + 2) + '"/>';
      s += '<line class="worldline-b" x1="' + X(LEN + m.b * (tmin - 2)) + '" y1="' + Y(tmin - 2) + '" x2="' + X(LEN + m.b * (tmax + 2)) + '" y2="' + Y(tmax + 2) + '"/>';
      if (step >= 1) s += '<line class="msg" x1="' + X(0) + '" y1="' + Y(0) + '" x2="' + X(m.x1) + '" y2="' + Y(m.t1) + '"/>';
      if (step >= 2) {
        if (m.u === Infinity) s += '<line class="now" x1="' + X(m.x1 - 20) + '" y1="' + Y(m.t1 - 20 * m.b) + '" x2="' + X(m.x1 + 20) + '" y2="' + Y(m.t1 + 20 * m.b) + '"/>';
        s += '<line class="reply" x1="' + X(m.x1) + '" y1="' + Y(m.t1) + '" x2="' + X(0) + '" y2="' + Y(m.t2) + '"/>';
      }
      if (step >= 3 && m.early) s += '<line stroke="var(--warn-text)" stroke-width="6" stroke-linecap="round" x1="' + X(0) + '" y1="' + Y(0) + '" x2="' + X(0) + '" y2="' + Y(m.t2) + '"/>';
      s += '<circle class="event ev-a" r="7" cx="' + X(0) + '" cy="' + Y(0) + '"/>';
      if (step >= 1) s += '<circle class="event ev-b" r="7" cx="' + X(m.x1) + '" cy="' + Y(m.t1) + '"/>';
      if (step >= 2) s += '<circle class="event" r="7" fill="var(--cyan-strong)" cx="' + X(0) + '" cy="' + Y(m.t2) + '"/>';
      s += "</g>";
      s += '<text class="t-strong" x="' + (+X(0) - 10) + '" y="' + (+Y(tmax) + 6) + '" text-anchor="end">' + esc(L.a) + "</text>";
      s += '<text class="t-accent" x="' + (+X(LEN + m.b * tmax) + 10) + '" y="' + (+Y(tmax) + 6) + '">' + esc(L.bob) + "</text>";
      s += '<text x="' + (+X(0) - 10) + '" y="' + (+Y(0) + 4) + '" text-anchor="end">' + esc(L.q) + " sent</text>";
      if (step >= 1) s += '<text class="t-violet" x="' + (+X(m.x1 / 2) + 6) + '" y="' + (+Y(m.t1 / 2) - 8) + '">' + esc(L.q) + "</text>";
      if (step >= 2 && m.u === Infinity) s += '<text x="' + (+X(m.x1) + 10) + '" y="' + (+Y(m.t1) + 18) + '">' + esc(L.now) + "</text>";
      if (step >= 3) s += '<text class="' + (m.early ? "t-warn" : "t-accent") + '" x="' + (+X(0) + 12) + '" y="' + (+Y(m.t2) + (m.early ? 18 : -10)) + '">' + esc(m.early ? L.back : L.ok) + "</text>";
      svg.innerHTML = '<title id="phone-title">' + esc($("#phone-title").textContent) + "</title>" + s;
    }

    function describe(m) {
      var sec = function (v) { return f(Math.abs(v), 2) + " s"; };
      var u = m.u === Infinity ? null : m.u;
      var M = {
        0: {
          eli5: "Alice is at home. Bob is far away in a spaceship, flying away from her at " + pct(m.b) + " of light speed. They both have tachyon phones.",
          eli13: "Alice is at rest; Bob starts 2 light-seconds away, flying off at " + f(m.b, 2) + "c. Their phones send tachyons at " + (u ? f(u, 1) + "c" : "infinite speed") + ", measured by whoever sends.",
          standard: "Alice at rest at x = 0; Bob starts 2 light-seconds away, receding at " + f(m.b, 2) + "c. Each transmitter sends tachyons at " + (u ? f(u, 1) + "c" : "infinite speed") + " in its own rest frame.",
          phd: "A: x = 0. B: x = 2 + βt, β = " + f(m.b, 2) + ". Signal speed u = " + (u ? f(u, 2) : "∞") + " in each emitter's frame."
        },
        1: {
          eli5: "Alice asks her question at noon. Her tachyon message zooms off and reaches Bob " + (m.t1 > 0.005 ? sec(m.t1) + " later" : "straight away") + ".",
          eli13: "Alice sends at t = 0. The message reaches Bob at t = " + f(m.t1, 2) + " s, " + f(m.x1, 2) + " light-seconds away.",
          standard: "Emitted at t = 0; absorbed by Bob at t₁ = " + f(m.t1, 2) + " s, x₁ = " + f(m.x1, 2) + " light-seconds (Alice's frame).",
          phd: "t₁ = L/(u − β) = " + f(m.t1, 3) + ", x₁ = " + f(m.x1, 3) + "."
        },
        2: {
          eli5: "Bob answers straight away. His answer is a tachyon too, super fast in his own way of counting time. " + (m.back ? "On Alice’s picture it heads <em>down</em>, into her past!" : "This time it still heads up, into Alice’s future."),
          eli13: "Bob replies at once, at " + (u ? f(u, 1) + " times light speed" : "infinite speed") + " in his frame. " + (m.back ? "Because uv > c², that reply runs into Alice’s past." : "Here uv < c², so in Alice’s frame the reply still moves forward in time, just slower."),
          standard: "Bob replies immediately at " + (u ? f(u, 1) + "c" : "infinite speed") + " in his own frame. " + (m.back ? "Since u·v > c², the reply runs backwards in Alice’s time." : "Since u·v < c², the reply still runs forwards in Alice’s time."),
          phd: "Reply velocity in A's frame: w = (β − u)/(1 − uβ)" + (u ? " = " + f((m.b - u) / (1 - u * m.b), 3) : " → 1/β") + "."
        },
        3: {
          eli5: m.early ? "Alice gets Bob’s answer " + sec(m.t2) + " <strong>before she asked her question!</strong> What if the answer said “don’t ask”? That’s the paradox." : "This time the answer arrives " + sec(m.t2) + " after she asked: no paradox. Try making Bob faster, or the messages faster.",
          eli13: m.early ? "The reply reaches Alice at t = −" + sec(m.t2) + ": <strong>before she sent the question.</strong> She could now decide not to send it." : "The reply reaches Alice at t = " + f(m.t2, 2) + " s, after she asked. For signals at " + (u ? f(u, 1) + "c" : "this speed") + ", Bob would need to go faster than " + f(m.thr, 2) + "c to break causality.",
          standard: m.early ? "Reply received at t₂ = −" + sec(m.t2) + ": <strong>before the question was sent.</strong> A closed causal loop." : "Reply received at t₂ = +" + f(m.t2, 2) + " s. The loop closes when Bob's speed exceeds 2u/(1 + u²) = " + f(m.thr, 2) + "c.",
          phd: "t₂ = L(2u − (1 + u²)β)/(u − β)² = " + f(m.t2, 3) + (m.early ? " < 0: causal loop." : " > 0; β_crit = " + f(m.thr, 3) + ".")
        }
      };
      return pick(M[step]);
    }

    function render() {
      var m = model();
      ob.innerHTML = f(m.b, 2) + "<i>c</i>";
      ou.innerHTML = m.u === Infinity ? pick({ eli5: "instant", eli13: "instant", standard: "instantaneous", phd: "∞" }) : f(m.u, 1) + "<i>c</i>";
      steps.forEach(function (btn) {
        var k = +btn.getAttribute("data-step");
        btn.setAttribute("aria-pressed", k === step ? "true" : "false");
      });
      draw2d(m);
      live(read, describe(m));
      if (three) three.update({ m: m, step: step, level: level, len: LEN });
    }

    steps.forEach(function (btn) {
      btn.addEventListener("click", function () { step = +btn.getAttribute("data-step"); render(); });
    });
    var update = onFrame(render);
    sb.addEventListener("input", update);
    su.addEventListener("input", update);

    // 3D view: load Three.js only when this figure approaches the viewport.
    function canWebGL() {
      try { var c = doc.createElement("canvas"); return !!(c.getContext("webgl2") || c.getContext("webgl")); } catch (e) { return false; }
    }
    var saveData = navigator.connection && navigator.connection.saveData;
    if (window.IntersectionObserver && canWebGL() && !saveData) {
      var io = new IntersectionObserver(function (entries) {
        if (!entries[0].isIntersecting) return;
        io.disconnect();
        import("./tachyon-3d.js").then(function (mod) {
          three = mod.mount(stage, { reduced: reduced.matches });
          fig.classList.add("is-3d");
          $(".phone-badge", fig).hidden = false;
          render();
        }).catch(function () { /* keep the 2D diagram */ });
      }, { rootMargin: "400px 0px" });
      io.observe(fig);
    }
    return { draw: render };
  })();

  function renderAll() {
    [energy, cones, cher, phone].forEach(function (v) { if (v) v.draw(); });
  }

  // Initial level: ?level= (not saved), else the saved choice, else Standard.
  var param = null;
  try { param = new URLSearchParams(location.search).get("level"); } catch (e) { /* old browser */ }
  if (param === "deeper") param = "phd";
  applyLevel(LEVELS.indexOf(param) >= 0 ? param : (stored() || "standard"), false);
  trackAnchor();
})();
