// Tachyon site: the hero orbit ring's animations and the picker that chooses between them.
// Progressive enhancement only: without this file, or under reduced motion, the ring stays the
// static SVG in index.html. Plays the visitor's last choice, else one at random; ?orbit=<id>
// picks one for a shared link without remembering it.
(function () {
  "use strict";

  var svg = document.querySelector(".orbit");
  var visual = document.querySelector(".hero-visual");
  var frame = document.querySelector(".hero .shot-frame");
  if (!svg || !visual || !frame || !window.requestAnimationFrame) return;
  var reduced = window.matchMedia("(prefers-reduced-motion: reduce)");
  if (reduced.matches) return;

  var EFFECTS = [
    { id: "escape", name: "Escape velocity",
      note: "A particle drifts along the top of the ring, then breaks orbit along its tangent in a blur." },
    { id: "retro", name: "Arrives before it leaves",
      note: "A tachyon would be seen arriving before it sets off. The far node implodes first, the particle is already there, and a comet runs backwards, tail first, to the start, which only then bursts as it leaves." },
    { id: "jump", name: "Escape, arriving first",
      note: "The drift of 1, but its jump is seen in reverse: the spot ahead implodes, the particle appears there, a blur streaks back to where it was, and only then does the original vanish." },
    { id: "cherenkov", name: "Cherenkov lap",
      note: "Cherenkov light is the blue glow of a particle outrunning light in water or glass. The particle laps the ring in 0.7 s, shedding a blue shock cone." }
  ];
  var STORE = "tachyon-orbit";
  function find(id) {
    for (var i = 0; i < EFFECTS.length; i++) if (EFFECTS[i].id === id) return EFFECTS[i];
    return null;
  }
  function stored() {
    try { return window.localStorage.getItem(STORE); } catch (e) { return null; }
  }
  function remember(id) {
    try { window.localStorage.setItem(STORE, id); } catch (e) { /* private mode: just don't remember */ }
  }

  // Geometry: the outer stroke of the ring in index.html, an ellipse rotated -12 degrees.
  var NS = "http://www.w3.org/2000/svg";
  var CX = 420, CY = 260, RX = 400, RY = 232, A = -12 * Math.PI / 180, CA = Math.cos(A), SA = Math.sin(A);
  var PI = Math.PI;
  function P(t) {
    var x = RX * Math.cos(t), y = RY * Math.sin(t);
    return { x: CX + x * CA - y * SA, y: CY + x * SA + y * CA };
  }
  function tangent(t) {
    var dx = -RX * Math.sin(t), dy = RY * Math.cos(t);
    var x = dx * CA - dy * SA, y = dx * SA + dy * CA, l = Math.sqrt(x * x + y * y);
    return { x: x / l, y: y / l };
  }
  function angleOf(px, py) {
    var dx = px - CX, dy = py - CY;
    return Math.atan2((-dx * SA + dy * CA) / RY, (dx * CA + dy * SA) / RX);
  }
  function clamp(v) { return v < 0 ? 0 : v > 1 ? 1 : v; }
  function seg(a, b, x) { return clamp((x - a) / (b - a)); }
  function easeIn(x) { return x * x * x; }
  function easeOut(x) { return 1 - Math.pow(1 - x, 3); }
  function n1(v) { return v.toFixed(1); }

  // Scene: everything the four effects draw, appended above the ring and the nodes.
  function mk(name, attrs, parent, style) {
    var e = document.createElementNS(NS, name);
    for (var k in attrs) e.setAttribute(k, attrs[k]);
    if (style) e.setAttribute("style", style);
    parent.appendChild(e);
    return e;
  }
  var defs = svg.querySelector("defs") || mk("defs", {}, svg);
  var glow = mk("radialGradient", { id: "orbit-fx-glow" }, defs);
  mk("stop", { offset: "0" }, glow, "stop-color: var(--orbit-hot)");
  mk("stop", { offset: ".25", "stop-color": "#00D1FF", "stop-opacity": ".7" }, glow);
  mk("stop", { offset: "1", "stop-color": "#00D1FF", "stop-opacity": "0" }, glow);
  var streakGradient = mk("linearGradient", { id: "orbit-fx-streak", gradientUnits: "userSpaceOnUse" }, defs);
  mk("stop", { offset: "0", "stop-color": "#7C3AED", "stop-opacity": "0" }, streakGradient);
  mk("stop", { offset: ".6", "stop-color": "#2563FF", "stop-opacity": ".55" }, streakGradient);
  mk("stop", { offset: ".93", "stop-color": "#00D1FF" }, streakGradient);
  mk("stop", { offset: "1" }, streakGradient, "stop-color: var(--orbit-hot)");

  var layer = mk("g", { "class": "orbit-fx", "stroke-linecap": "round", fill: "none" }, svg);
  var all = [];
  function part(name, attrs, parent, style) {
    var e = mk(name, attrs, parent || layer, style);
    e.setAttribute("opacity", "0");
    if (!parent) all.push(e);
    return e;
  }
  var N = 36, trail = [], halo = [], i;
  for (i = 0; i < N; i++) halo.push(part("path", {}, null, "stroke: #00D1FF"));
  for (i = 0; i < N; i++) trail.push(part("path", {}));
  var track = part("path", { "stroke-width": "2" }, null, "stroke: #00D1FF");
  var ringFlash = part("ellipse", { cx: CX, cy: CY, rx: RX, ry: RY, "stroke-width": "3", transform: "rotate(-12 420 260)" }, null, "stroke: #00D1FF");
  var streakHalo = part("line", { stroke: "url(#orbit-fx-streak)", "stroke-width": "18" });
  var streak = part("line", { stroke: "url(#orbit-fx-streak)", "stroke-width": "6" });
  var cones = [part("line", { "stroke-width": "1.5" }, null, "stroke: #00D1FF"), part("line", { "stroke-width": "1.5" }, null, "stroke: #00D1FF")];
  var burstA = part("circle", { "stroke-width": "2", r: "0" }, null, "stroke: #00D1FF");
  var burstB = part("circle", { "stroke-width": "2", r: "0" }, null, "stroke: #7C3AED");
  var inRings = [], sparks = [];
  for (i = 0; i < 3; i++) inRings.push(part("circle", { "stroke-width": "1.5", r: "0" }, null, "stroke: " + (i === 1 ? "#7C3AED" : "#00D1FF")));
  for (i = 0; i < 12; i++) sparks.push(part("circle", { r: "1.8" }, null, "fill: var(--orbit-spark)"));
  function particle() {
    var g = part("g", {});
    mk("circle", { r: "28", fill: "url(#orbit-fx-glow)" }, g);
    mk("circle", { r: "4.5" }, g, "fill: var(--orbit-hot)");
    return g;
  }
  var head = particle(), arrived = particle();

  function hideAll() { all.forEach(function (e) { e.setAttribute("opacity", "0"); }); }
  function show(e, a) { e.setAttribute("opacity", a > 0 ? a.toFixed(3) : "0"); }
  function place(g, p, a) { g.setAttribute("transform", "translate(" + n1(p.x) + " " + n1(p.y) + ")"); show(g, a); }
  function colour(f) { return f < 0.12 ? "var(--orbit-hot)" : f < 0.5 ? "#00D1FF" : f < 0.8 ? "#2563FF" : "#7C3AED"; }
  function drawSegment(k, d, w, a, f) {
    trail[k].setAttribute("d", d); halo[k].setAttribute("d", d);
    trail[k].setAttribute("stroke-width", w.toFixed(2)); halo[k].setAttribute("stroke-width", (w * 3.4).toFixed(2));
    trail[k].style.stroke = colour(f);
    show(trail[k], a); show(halo[k], a * 0.28);
  }
  // A fading trail along the ring, from the particle at angle t back by `span` (dir +1 travels toward larger t).
  function setTrail(t, span, dir, alpha, width) {
    for (var k = 0; k < N; k++) {
      var f0 = k / N, f1 = (k + 1) / N, fade = 1 - f0;
      var p0 = P(t - dir * span * f0), p1 = P(t - dir * span * f1), pm = P(t - dir * span * (f0 + f1) / 2);
      drawSegment(k, "M" + n1(p0.x) + " " + n1(p0.y) + "Q" + n1(2 * pm.x - (p0.x + p1.x) / 2) + " " + n1(2 * pm.y - (p0.y + p1.y) / 2) + " " + n1(p1.x) + " " + n1(p1.y),
        width * (0.35 + 0.65 * fade), alpha * Math.pow(fade, 1.6), f0);
    }
  }
  function hideTrail() { for (var k = 0; k < N; k++) { show(trail[k], 0); show(halo[k], 0); } }
  function setStreak(tail, tip, a) {
    [streak, streakHalo].forEach(function (e) {
      e.setAttribute("x1", n1(tail.x)); e.setAttribute("y1", n1(tail.y)); e.setAttribute("x2", n1(tip.x)); e.setAttribute("y2", n1(tip.y));
    });
    streakGradient.setAttribute("x1", n1(tail.x)); streakGradient.setAttribute("y1", n1(tail.y));
    streakGradient.setAttribute("x2", n1(tip.x)); streakGradient.setAttribute("y2", n1(tip.y));
    show(streak, a); show(streakHalo, a * 0.3);
  }
  function burst(c, p, f, rmax) {
    c.setAttribute("cx", n1(p.x)); c.setAttribute("cy", n1(p.y)); c.setAttribute("r", n1(7 + rmax * easeOut(f)));
    show(c, f > 0 && f < 1 ? 0.9 * (1 - f) : 0);
  }
  // A burst run backwards: rings and sparks collapse onto p, accelerating into contact at f = 1.
  function implode(p, f) {
    inRings.forEach(function (e, k) {
      var g = seg(k * 0.14, 0.72 + k * 0.14, f);
      e.setAttribute("cx", n1(p.x)); e.setAttribute("cy", n1(p.y)); e.setAttribute("r", n1(6 + 78 * (1 - easeIn(g))));
      show(e, g > 0 && g < 1 ? 0.85 * Math.sqrt(g) : 0);
    });
    var g2 = seg(0.12, 1, f);
    sparks.forEach(function (e, k) {
      var ang = k * PI * 2 / sparks.length + 0.4 + (k % 2) * 0.15, rad = (64 + (k % 3) * 22) * (1 - easeIn(g2));
      e.setAttribute("cx", n1(p.x + Math.cos(ang) * rad)); e.setAttribute("cy", n1(p.y + Math.sin(ang) * rad));
      show(e, g2 > 0 && g2 < 1 ? 0.95 * seg(0, 0.4, g2) : 0);
    });
  }
  function setTrack(ta, tb, a) {
    if (!(a > 0)) { show(track, 0); return; }
    var d = "";
    for (var k = 0; k <= 48; k++) { var p = P(ta + (tb - ta) * k / 48); d += (k ? "L" : "M") + n1(p.x) + " " + n1(p.y); }
    track.setAttribute("d", d); show(track, a);
  }
  // The reversed jump of effects 2 and 3. x is seconds since it began: the destination implodes for
  // `pull` s, the particle is there, a tail-first comet runs back to the origin in `dur` s, then the
  // origin bursts as the particle "leaves".
  function reversedJump(x, tFrom, tTo, dur, pull) {
    var pFrom = P(tFrom), pTo = P(tTo), back = pull + dur;
    implode(pTo, seg(0, pull, x));
    burst(burstA, pTo, seg(pull, pull + 0.5, x), 30);
    place(arrived, pTo, x < pull ? 0 : 0.6 + 0.4 * (1 - seg(pull, pull + 1.2, x)));
    if (x >= pull && x < back + 0.05) {
      var g = easeOut(seg(pull, back, x)), th = tTo + (tFrom - tTo) * g;
      // Tail first: the tail points ahead of the comet, toward where the particle still is.
      setTrail(th, Math.min(Math.abs(tFrom - th), 0.45), tFrom > tTo ? -1 : 1, 1, 4.5);
      setTrack(tTo, th, 0.55);
    } else {
      hideTrail();
      setTrack(tTo, tFrom, x >= back ? 0.55 * (1 - seg(back, back + 1.4, x)) : 0);
    }
    burst(burstB, pFrom, seg(back, back + 0.7, x), 44);
    return back;
  }

  var nodeTop = { x: 574, y: 22 }, nodeBottom = { x: 177, y: 479 };
  var tTop = angleOf(nodeTop.x, nodeTop.y), tBottom = angleOf(nodeBottom.x, nodeBottom.y);
  if (tBottom < tTop) tBottom += 2 * PI;

  // Each effect draws the frame s seconds into its loop. The visible parts of the ring (outside the
  // screenshot) are its top arc and right side, about t = 1.42 pi round to 0.33 pi, so all of them play there.
  var draw = {
    escape: function (s) {
      var x = s % 11, t0 = PI * 1.42, t1 = PI * 1.78;
      if (x < 9.6) {
        var t = t0 + (t1 - t0) * x / 9.6, a = 0.55 * Math.min(1, x / 1.2);
        place(head, P(t), a); setTrail(t, 0.28, 1, a * 0.8, 3); show(streak, 0); show(streakHalo, 0);
      } else if (x < 10.05) {
        var p = P(t1), d = tangent(t1), dist = 1500 * easeIn(seg(9.6, 10.05, x)), len = Math.min(dist, 520);
        var tip = { x: p.x + d.x * dist, y: p.y + d.y * dist };
        place(head, tip, 1); hideTrail(); setStreak({ x: tip.x - d.x * len, y: tip.y - d.y * len }, tip, 1);
      } else {
        show(head, 0); show(streak, 0); show(streakHalo, 0);
      }
      burst(burstA, P(t1), seg(9.6, 10.3, x), 34);
    },
    retro: function (s) {
      var x = s % 10, start = 5.2;
      if (x < start) {
        hideAll();
        place(head, nodeBottom, (0.45 + 0.15 * Math.sin(x * 2.2)) * Math.min(1, x / 0.8));
        return;
      }
      var back = reversedJump(x - start, tBottom, tTop, 0.5, 0.7);
      place(head, nodeBottom, x - start < back ? 0.6 : 0);
      if (x - start > 1.9) place(arrived, nodeTop, 0.6 * (1 - seg(8.6, 9.4, x)));
    },
    jump: function (s) {
      var x = s % 11, t0 = PI * 1.42, t1 = PI * 1.62, tTo = PI * 2.24, start = 8.4;
      if (x < start) {
        hideAll();
        var t = t0 + (t1 - t0) * x / start, a = 0.6 * Math.min(1, x / 1.2);
        place(head, P(t), a); setTrail(t, 0.24, 1, a * 0.8, 3);
        return;
      }
      var back = reversedJump(x - start, t1, tTo, 0.28, 0.5);
      place(head, P(t1), x - start < back ? 0.6 : 0);
      if (x - start > 1.7) place(arrived, P(tTo), 0.6 * (1 - seg(10, 10.8, x)));
    },
    cherenkov: function (s) {
      var x = s % 8, lap = x > 6.2 && x < 6.9, b = seg(6.2, 6.9, x), smooth = b * b * (3 - 2 * b);
      var t = PI * 1.42 + 0.12 * x + 2 * PI * smooth, p = P(t), d = tangent(t), v = lap ? Math.sin(PI * b) : 0;
      var fade = Math.min(1, x / 0.8, (8 - x) / 0.6);
      place(head, p, (0.5 + 0.5 * v) * fade);
      setTrail(t, 0.25 + 1.6 * v, 1, (0.45 + 0.5 * v) * fade, 2.5 + 1.5 * v);
      cones.forEach(function (c, k) {
        var ang = k ? -0.55 : 0.55, ca = Math.cos(ang), sa = Math.sin(ang);
        var bx = -(d.x * ca - d.y * sa), by = -(d.x * sa + d.y * ca);
        c.setAttribute("x1", n1(p.x)); c.setAttribute("y1", n1(p.y));
        c.setAttribute("x2", n1(p.x + bx * 70 * v)); c.setAttribute("y2", n1(p.y + by * 70 * v));
        show(c, 0.8 * v);
      });
      show(ringFlash, x > 6.2 && x < 7.8 ? 0.35 * (lap ? b : 1 - seg(6.9, 7.8, x)) : 0);
    }
  };

  // Loop: runs only while the ring is on screen and the tab is visible.
  var current = null, start = null, raf = 0, onScreen = true;
  function frameStep(now) {
    raf = 0;
    if (!current) return;
    if (start === null) start = now;
    draw[current.id]((now - start) / 1000);
    if (onScreen && !document.hidden) raf = window.requestAnimationFrame(frameStep);
  }
  function resume() {
    if (!raf && current && onScreen && !document.hidden) {
      start = null;
      raf = window.requestAnimationFrame(frameStep);
    }
  }
  if ("IntersectionObserver" in window) {
    new IntersectionObserver(function (entries) {
      onScreen = entries[0].isIntersecting;
      resume();
    }).observe(svg);
  }
  document.addEventListener("visibilitychange", resume);

  // Picker: a card under the screenshot on wide screens; on narrow ones a sheet opened by tapping
  // the screenshot or the "Orbit animation" button under it.
  var narrow = window.matchMedia("(max-width: 980px)");
  var picker = document.createElement("section");
  picker.className = "orbit-picker";
  picker.id = "orbit-picker";
  picker.setAttribute("aria-labelledby", "orbit-picker-title");
  var options = EFFECTS.map(function (e, k) {
    return '<input type="radio" name="orbit-fx" id="orbit-fx-' + e.id + '" value="' + e.id + '">' +
      '<label for="orbit-fx-' + e.id + '"><span class="orbit-num">' + (k + 1) + "</span>" + e.name + "</label>";
  }).join("");
  picker.innerHTML =
    '<div class="orbit-picker-head"><h2 id="orbit-picker-title">Orbit animation</h2>' +
    '<button type="button" class="orbit-picker-close" aria-label="Close">&times;</button></div>' +
    '<p class="orbit-picker-sub">Choose what the ring around the screenshot does.</p>' +
    '<div class="orbit-options" role="radiogroup" aria-labelledby="orbit-picker-title">' + options + "</div>" +
    '<p class="orbit-picker-note" aria-live="polite"></p>' +
    '<a class="orbit-picker-link" href="tachyon.html">What is a tachyon?<svg class="icon" aria-hidden="true"><use href="#i-arrow-right"/></svg></a>';
  var note = picker.querySelector(".orbit-picker-note");

  var toggle = document.createElement("button");
  toggle.type = "button";
  toggle.className = "orbit-toggle";
  toggle.setAttribute("aria-controls", "orbit-picker");
  toggle.setAttribute("aria-expanded", "false");
  toggle.innerHTML = '<svg viewBox="0 0 24 24" aria-hidden="true"><ellipse cx="12" cy="12" rx="10" ry="6" transform="rotate(-12 12 12)"/><circle cx="17.5" cy="7.2" r="2"/></svg>' +
    '<span>Orbit animation: <strong></strong></span>';
  var toggleName = toggle.querySelector("strong");

  var backdrop = document.createElement("div");
  backdrop.className = "orbit-backdrop";

  function select(id, keep) {
    current = find(id) || EFFECTS[0];
    if (keep) remember(current.id);
    picker.querySelector("#orbit-fx-" + current.id).checked = true;
    note.textContent = current.note;
    toggleName.textContent = current.name;
    hideAll();
    if (raf) { window.cancelAnimationFrame(raf); raf = 0; }
    resume();
  }
  function open() {
    if (!narrow.matches) return;
    picker.classList.add("is-open");
    backdrop.classList.add("is-open");
    toggle.setAttribute("aria-expanded", "true");
    picker.querySelector("input:checked").focus();
  }
  function close() {
    if (!picker.classList.contains("is-open")) return;
    picker.classList.remove("is-open");
    backdrop.classList.remove("is-open");
    toggle.setAttribute("aria-expanded", "false");
    if (narrow.matches) toggle.focus();
  }
  function layout() {
    if (narrow.matches) {
      picker.setAttribute("role", "dialog");
      document.body.appendChild(backdrop);
      document.body.appendChild(picker);
    } else {
      close();
      picker.removeAttribute("role");
      visual.appendChild(picker);
    }
  }
  picker.addEventListener("change", function (e) { if (e.target.name === "orbit-fx") select(e.target.value, true); });
  picker.querySelector(".orbit-picker-close").addEventListener("click", close);
  backdrop.addEventListener("click", close);
  toggle.addEventListener("click", open);
  frame.addEventListener("click", open);
  document.addEventListener("keydown", function (e) { if (e.key === "Escape") close(); });

  visual.appendChild(toggle);
  layout();
  if (narrow.addEventListener) narrow.addEventListener("change", layout); else narrow.addListener(layout);
  if (reduced.addEventListener) {
    reduced.addEventListener("change", function () {
      if (!reduced.matches) return;
      current = null; hideAll(); close();
      picker.remove(); toggle.remove(); backdrop.remove();
    });
  }

  var param = (window.location.search.match(/[?&]orbit=([a-z]+)/) || [])[1];
  var initial = find(param) || find(stored()) || EFFECTS[Math.floor(Math.random() * EFFECTS.length)];
  select(initial.id, false);
})();
