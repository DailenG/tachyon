// Tachyon site: progressive enhancement only. Every link, the benchmark
// numbers, and the block-swap demo already work with this file absent;
// this only adds motion and small conveniences.
(function () {
  "use strict";

  var reduceMotion = window.matchMedia(
    "(prefers-reduced-motion: reduce)"
  ).matches;

  document.documentElement.classList.add("js");

  // "This page painted in N ms" - only ever shows a number this browser
  // actually measured via the Paint Timing API. No API, no number: the
  // paragraph stays hidden, exactly as it started in the HTML.
  (function () {
    var el = document.getElementById("live-timing");
    if (!el) return;
    function report(ms) {
      if (!(ms > 0) || !isFinite(ms)) return;
      el.textContent =
        "This page painted in " +
        Math.round(ms) +
        " ms on your machine. Tachyon's window: 20\u201335 ms.";
      el.hidden = false;
    }
    function fromEntries(list) {
      for (var i = 0; i < list.length; i++) {
        if (list[i].name === "first-contentful-paint") return list[i].startTime;
      }
      return null;
    }
    try {
      var existing = fromEntries(performance.getEntriesByType("paint"));
      if (existing != null) {
        report(existing);
        return;
      }
      if ("PerformanceObserver" in window) {
        var po = new PerformanceObserver(function (list) {
          var ms = fromEntries(list.getEntries());
          if (ms != null) {
            report(ms);
            po.disconnect();
          }
        });
        po.observe({ type: "paint", buffered: true });
      }
    } catch (e) {
      /* Paint Timing unsupported: leave the paragraph hidden. */
    }
  })();

  // The hero screenshot starts on the dark capture when the visitor's system is dark, to match
  // the page. Without JS the light capture shows, and both tabs still work.
  if (window.matchMedia("(prefers-color-scheme: dark)").matches) {
    var darkShot = document.getElementById("shot-dark");
    if (darkShot) darkShot.checked = true;
  }

  // The launch race: two bars filling over their own real measured
  // duration (30 ms resident, 350 ms cold), starting together. The static
  // CSS default (no JS, or reduced motion) already shows both bars at full
  // length with the honest measured range in text, so this only adds the
  // in-view trigger and the Replay button.
  if (!reduceMotion && "IntersectionObserver" in window) {
    var race = document.querySelector(".race");
    var replayBtn = document.getElementById("race-replay");
    if (race) {
      var fills = Array.prototype.slice.call(
        race.querySelectorAll(".race-fill")
      );
      race.classList.add("js-armed");
      function runRace() {
        fills.forEach(function (f) {
          f.classList.remove("run");
        });
        // Force reflow so the removed class takes effect before re-adding it,
        // otherwise the browser can coalesce both into a no-op.
        void race.offsetWidth;
        requestAnimationFrame(function () {
          fills.forEach(function (f) {
            f.classList.add("run");
          });
        });
      }
      var io = new IntersectionObserver(
        function (entries) {
          entries.forEach(function (entry) {
            if (entry.isIntersecting) {
              runRace();
              io.disconnect();
            }
          });
        },
        { threshold: 0.4 }
      );
      io.observe(race);
      if (replayBtn) {
        replayBtn.hidden = false;
        replayBtn.addEventListener("click", runRace);
      }
    }
  }

  // Block-swap demo: the raw/rendered toggle is plain radio inputs and
  // works by click or arrow keys with no JS at all. This just lets a mouse
  // "hover like a caret" without requiring a click.
  document.querySelectorAll(".swap-line").forEach(function (label) {
    var input = document.getElementById(label.getAttribute("for"));
    if (!input) return;
    label.addEventListener("mouseenter", function () {
      input.checked = true;
    });
  });

  // Small "Copy" affordance on the build-from-source commands.
  document.querySelectorAll("pre.code-block").forEach(function (block) {
    var button = document.createElement("button");
    button.type = "button";
    button.className = "copy-btn";
    button.textContent = "Copy";
    button.setAttribute("aria-label", "Copy command to clipboard");
    button.addEventListener("click", function () {
      var text = block.innerText.replace(/\u00a0/g, " ");
      if (navigator.clipboard) {
        navigator.clipboard.writeText(text).then(function () {
          button.textContent = "Copied";
          setTimeout(function () {
            button.textContent = "Copy";
          }, 1500);
        });
      }
    });
    block.appendChild(button);
  });
})();

// Download buttons that name a release asset (`data-asset` is the end of its file name, whose
// version part changes every release) point straight at the latest release's file. Without
// JavaScript, or if GitHub's API is unreachable or rate-limited, they keep their static link to
// the latest release page.
(function () {
  var buttons = document.querySelectorAll("a[data-asset]");
  if (!buttons.length || !window.fetch) return;
  fetch("https://api.github.com/repos/DailenG/tachyon/releases/latest")
    .then(function (response) {
      return response.ok ? response.json() : null;
    })
    .then(function (release) {
      if (!release || !release.assets) return;
      buttons.forEach(function (button) {
        var suffix = button.getAttribute("data-asset");
        var asset = release.assets.find(function (a) {
          return a.name.endsWith(suffix);
        });
        if (asset) button.href = asset.browser_download_url;
      });
    })
    .catch(function () {});
})();
