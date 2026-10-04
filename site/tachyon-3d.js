// The tachyonic antitelephone in 3D, for tachyon.html. Loaded on demand by tachyon.js.
// The scene is a spacetime "block": two space axes (x along the motion, y across it) and time
// rising upwards, in Alice's frame. Worldlines are drawn as tubes, light cones as translucent
// cones, and the two tachyon signals as arrows. Drag to orbit; the 2D diagram underneath
// stays the accessible equivalent and the source of the live text readout.
import {
  WebGLRenderer, Scene, PerspectiveCamera, Group, Mesh, Line, LineBasicMaterial,
  MeshBasicMaterial, SphereGeometry, CylinderGeometry, PlaneGeometry, BufferGeometry,
  Float32BufferAttribute, Vector3, Color, GridHelper, DoubleSide, OrbitControls,
} from "./vendor/three.min.js";

function css(name) {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim() || "#2563ff";
}

export function mount(stage, opts) {
  const canvas = stage.querySelector(".phone-canvas");
  const renderer = new WebGLRenderer({ canvas, antialias: true, alpha: true, powerPreference: "low-power" });
  renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 2));
  const scene = new Scene();
  const camera = new PerspectiveCamera(36, 1, 0.1, 200);
  // Mostly face-on, so the diagram reads like the 2D one, turned a little to show depth.
  camera.position.set(4.2, 3.2, 11.5);
  const controls = new OrbitControls(camera, canvas);
  controls.enableDamping = !opts.reduced;
  controls.dampingFactor = 0.08;
  controls.enablePan = false;
  controls.minDistance = 5;
  controls.maxDistance = 22;
  controls.minPolarAngle = 0.25;
  controls.maxPolarAngle = Math.PI - 0.25;

  const world = new Group();
  scene.add(world);
  const labels = [];

  function mat(color, opacity) {
    return new MeshBasicMaterial({ color: new Color(color), transparent: opacity < 1, opacity, depthWrite: opacity >= 1, side: DoubleSide });
  }
  // Space is x (motion) and z (across); time is y, up.
  function tube(a, b, color, r, opacity) {
    const dir = new Vector3().subVectors(b, a), len = dir.length();
    if (len < 1e-6) return null;
    const m = new Mesh(new CylinderGeometry(r, r, len, 12, 1, false), mat(color, opacity == null ? 1 : opacity));
    m.position.copy(a).addScaledVector(dir, 0.5);
    m.quaternion.setFromUnitVectors(new Vector3(0, 1, 0), dir.normalize());
    return m;
  }
  function arrow(a, b, color) {
    const g = new Group(), dir = new Vector3().subVectors(b, a), len = dir.length();
    if (len < 1e-6) return g;
    const head = Math.min(0.42, len * 0.35), shaft = tube(a, a.clone().addScaledVector(dir.clone().normalize(), len - head), color, 0.05);
    if (shaft) g.add(shaft);
    const cone = new Mesh(new CylinderGeometry(0, 0.16, head, 16), mat(color, 1));
    cone.position.copy(b).addScaledVector(dir.clone().normalize(), -head / 2);
    cone.quaternion.setFromUnitVectors(new Vector3(0, 1, 0), dir.normalize());
    g.add(cone);
    return g;
  }
  function dot(p, color, r) {
    const m = new Mesh(new SphereGeometry(r || 0.13, 20, 14), mat(color, 1));
    m.position.copy(p);
    return m;
  }
  function label(text, p, cls) {
    const el = document.createElement("span");
    el.className = "phone-label" + (cls ? " " + cls : "");
    el.textContent = text;
    stage.appendChild(el);
    labels.push({ el, p });
  }

  let state = null, framed = false;
  // World units: 1 light-second across = 1 second up, so light lines and cones really are at
  // 45° on screen, as in the 2D diagram.
  const SX = 1.8, ST = 1.8;
  function W(x, t, y) { return new Vector3(x * SX, t * ST, (y || 0) * SX); }

  function build() {
    if (!state) return;
    const { m, step, level, len } = state;
    // Every rebuild makes new meshes; free the old ones' GPU buffers first.
    world.traverse((o) => {
      if (o.geometry) o.geometry.dispose();
      if (o.material) o.material.dispose();
    });
    world.clear();
    labels.forEach((l) => l.el.remove());
    labels.length = 0;
    const ink = css("--text-strong"), accent = css("--accent"), violet = css("--violet"), cyan = css("--cyan-strong"),
      warn = css("--warn-text"), lightC = css("--viz-light"), sub = css("--border-default");
    const tTop = Math.max(1.6, m.t1 + 0.9), tBot = Math.min(-0.9, m.t2 - 0.7);
    const xMax = Math.max(len + m.b * tTop, m.x1) + 0.6;

    // The floor is "space" at Alice's question time; a faint grid marks it.
    const grid = new GridHelper(12, 12, new Color(sub), new Color(sub));
    grid.position.copy(W(xMax / 2 - 0.3, 0, 0));
    grid.material.transparent = true;
    grid.material.opacity = 0.4;
    world.add(grid);

    // Alice's light cone at the question, scaled so its slope is exactly light speed.
    const coneH = Math.min(tTop, 1.4);
    [1, -1].forEach((s) => {
      const c = new Mesh(new CylinderGeometry(s > 0 ? coneH * SX : 0, s > 0 ? 0 : coneH * SX, coneH * ST, 48, 1, true), mat(lightC, 0.12));
      c.position.copy(W(0, s * coneH / 2, 0));
      world.add(c);
    });

    // Worldlines: Alice at x = 0, Bob along x = len + βt.
    world.add(tube(W(0, tBot, 0), W(0, tTop, 0), ink, 0.05));
    world.add(tube(W(len + m.b * tBot, tBot, 0), W(len + m.b * tTop, tTop, 0), accent, 0.05));
    world.add(dot(W(0, 0, 0), ink, 0.16));

    const lang = {
      eli5: ["Alice", "Bob", "question", "answer", "answer arrives first!", "Alice’s now", "Bob’s now"],
      eli13: ["Alice", "Bob", "question", "reply", "reply lands before she asked", "Alice’s now", "Bob’s now"],
      standard: ["Alice", "Bob", "message", "reply", "reply received before the message was sent", "Alice’s now", "Bob’s now"],
      phd: ["A", "B", "A → B", "B → A", "t₂ < 0", "t = 0", "t′ = t′₁"],
    }[level] || [];
    label(lang[0], W(0, tTop + 0.15, 0));
    label(lang[1], W(len + m.b * tTop, tTop + 0.15, 0));
    label(lang[5], W(xMax + 0.1, 0, 1.4));

    // A plane of simultaneity: the set of events at one moment for an observer, drawn as a
    // translucent slab across the depth axis. slope = dt/dx (0 for Alice, β for Bob).
    function nowPlane(x, t, slope, color, width) {
      const pl = new Mesh(new PlaneGeometry(width * SX, 2.6 * SX), mat(color, 0.09));
      pl.rotation.x = -Math.PI / 2;
      pl.rotateOnWorldAxis(new Vector3(0, 0, 1), Math.atan(slope * ST / SX));
      pl.position.copy(W(x, t, 0));
      world.add(pl);
    }
    nowPlane(xMax / 2 - 0.2, 0, 0, ink, xMax + 1.4);

    if (step >= 1) {
      world.add(arrow(W(0, 0, 0), W(m.x1, m.t1, 0), violet));
      world.add(dot(W(m.x1, m.t1, 0), violet, 0.15));
      // Labels sit beside each arrow's far end, pushed away from the other arrow, which can run
      // above or below it depending on whether the reply lands in Alice's past or future.
      label(lang[2], W(m.x1 * 0.78, m.t1 + (m.t2 < m.t1 ? 0.32 : -0.32), 0));
    }
    if (step >= 2) {
      // Bob's line of simultaneity is what makes an instant reply run backwards in Alice's time.
      // It is only the reply's path when the signal is instantaneous in Bob's frame.
      if (m.u === Infinity) nowPlane(m.x1, m.t1, m.b, accent, Math.max(4, xMax + 1.5));
      world.add(arrow(W(m.x1, m.t1, 0), W(0, m.t2, 0), cyan));
      world.add(dot(W(0, m.t2, 0), cyan, 0.15));
      label(lang[3], W(m.x1 * 0.5 + (m.t2 < m.t1 ? 0.45 : 0), (m.t1 + m.t2) / 2 + (m.t2 < m.t1 ? -0.2 : 0.32), 0));
      if (m.u === Infinity) label(lang[6], W(m.x1 + 1.1, m.t1 + 1.1 * m.b, 1.4));
    }
    if (step >= 3 && m.early) {
      world.add(tube(W(0, m.t2, 0), W(0, 0, 0), warn, 0.1));
      label(lang[4], W(-0.25, m.t2 - 0.25, 0), "is-warn");
    }
    // Aim at the middle of the experiment; on first build, back off far enough to fit it.
    const centre = W(xMax / 2 - 0.2, (tTop + tBot) / 2, 0);
    if (!framed) {
      resize();
      const half = (camera.fov * Math.PI) / 360;
      const needH = (tTop - tBot + 0.8) * ST, needW = (xMax + 1.6) * SX;
      const dist = Math.max(needH / (2 * Math.tan(half)), needW / (2 * Math.tan(half) * camera.aspect)) * 1.12;
      const view = camera.position.clone().sub(controls.target).normalize();
      camera.position.copy(centre).addScaledVector(view, dist);
      framed = true;
    }
    controls.target.copy(centre);
    resize();
    frame();
  }

  const v = new Vector3();
  function place() {
    const r = canvas.getBoundingClientRect(), s = stage.getBoundingClientRect();
    labels.forEach((l) => {
      v.copy(l.p).applyMatrix4(world.matrixWorld).project(camera);
      l.el.style.left = (r.left - s.left + (v.x + 1) / 2 * r.width) + "px";
      l.el.style.top = (r.top - s.top + (1 - v.y) / 2 * r.height) + "px";
      l.el.style.display = v.z < 1 ? "" : "none";
    });
  }
  function resize() {
    const w = canvas.clientWidth, h = canvas.clientHeight;
    if (!w || !h) return;
    renderer.setSize(w, h, false);
    camera.aspect = w / h;
    camera.updateProjectionMatrix();
  }
  let raf = 0;
  function frame() {
    raf = 0;
    const moving = controls.update();
    renderer.render(scene, camera);
    place();
    if (moving) raf = requestAnimationFrame(frame);
  }
  function kick() { if (!raf) raf = requestAnimationFrame(frame); }
  controls.addEventListener("change", kick);
  new ResizeObserver(() => { resize(); kick(); }).observe(canvas);
  // theme changes recolour the scene
  window.matchMedia("(prefers-color-scheme: dark)").addEventListener("change", build);

  return {
    update(next) { state = next; build(); },
  };
}
