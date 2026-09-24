// Progressive enhancement only: tabs + theme toggle + hero motion. Without JS,
// all panels show and every animated surface renders its final state (the
// markup default); nothing below is required to read the page.
(function () {
  "use strict";
  var reduced = window.matchMedia && window.matchMedia("(prefers-reduced-motion: reduce)").matches;

  var KEY = "runwaybar:theme";
  var root = document.documentElement;
  var stored = null;
  try { stored = localStorage.getItem(KEY); } catch (e) { /* private mode */ }
  if (stored === "light" || stored === "dark") {
    root.setAttribute("data-theme", stored);
  }
  document.getElementById("theme-toggle").addEventListener("click", function () {
    var next = root.getAttribute("data-theme") === "light" ? "dark" : "light";
    root.setAttribute("data-theme", next);
    try { localStorage.setItem(KEY, next); } catch (e) { /* ignore */ }
  });

  var tabs = Array.prototype.slice.call(document.querySelectorAll('.tabs button[role="tab"]'));
  function select(btn) {
    tabs.forEach(function (b) {
      var on = b === btn;
      b.setAttribute("aria-selected", on ? "true" : "false");
      var panel = document.getElementById(b.getAttribute("aria-controls"));
      if (panel) {
        panel.hidden = !on;
        panel.classList.toggle("active", on);
      }
    });
  }
  tabs.forEach(function (b) {
    b.addEventListener("click", function () { select(b); });
  });
  if (tabs.length) select(tabs[0]);

  // Copy buttons: unhidden here so no-JS renders no dead buttons. Each button
  // copies its panel's full command-block text (both lines for the .deb tab),
  // prompt spans stripped, via navigator.clipboard with an execCommand fallback.
  var copies = Array.prototype.slice.call(document.querySelectorAll('.copy-btn[data-copy-for]'));
  copies.forEach(function (btn) {
    btn.hidden = false;
    btn.addEventListener("click", function () {
      var panel = document.getElementById(btn.getAttribute("data-copy-for"));
      var pre = panel ? panel.querySelector("pre") : null;
      if (!pre) return;
      var clone = pre.cloneNode(true);
      Array.prototype.slice.call(clone.querySelectorAll(".prompt")).forEach(function (el) {
        el.remove();
      });
      var text = clone.textContent.split("\n").map(function (l) { return l.trim(); }).filter(Boolean).join("\n");
      function done() {
        btn.textContent = "Copied";
        setTimeout(function () { btn.textContent = "Copy"; }, 1500);
      }
      function fallback() {
        var ta = document.createElement("textarea");
        ta.value = text;
        document.body.appendChild(ta);
        ta.select();
        try { document.execCommand("copy"); done(); } catch (e) { /* clipboard unavailable */ }
        document.body.removeChild(ta);
      }
      if (navigator.clipboard && navigator.clipboard.writeText) {
        navigator.clipboard.writeText(text).then(done, fallback);
      } else {
        fallback();
      }
    });
  });

  // Nav gains its solid wash once the page scrolls.
  var nav = document.querySelector(".nav");
  if (nav) {
    var onScroll = function () {
      nav.classList.toggle("scrolled", window.scrollY > 8);
    };
    window.addEventListener("scroll", onScroll, { passive: true });
    onScroll();
  }

  function countUp(el, target, dur) {
    var t0 = null;
    function frame(t) {
      if (t0 === null) t0 = t;
      var p = Math.min(1, (t - t0) / dur);
      var eased = 1 - Math.pow(1 - p, 3);
      el.textContent = Math.round(target * eased) + "%";
      if (p < 1) requestAnimationFrame(frame);
    }
    requestAnimationFrame(frame);
  }

  // Hero terminal: type the command, stagger the rows, grow the bars.
  // Markup already holds the final state; reduced-motion keeps it untouched.
  var demo = document.getElementById("demo");
  if (demo && !reduced) {
    var typed = demo.querySelector(".demo-typed");
    var rows = Array.prototype.slice.call(demo.querySelectorAll(".demo-row"));
    var end = demo.querySelector(".demo-end");
    var command = typed ? typed.textContent : "";
    var playing = false;
    function reset() {
      if (typed) typed.textContent = "";
      rows.forEach(function (r) {
        r.classList.remove("on");
        var fill = r.querySelector(".demo-fill");
        var pct = r.querySelector(".demo-pct");
        if (fill) fill.style.width = "0%";
        if (pct) pct.textContent = "0%";
      });
      if (end) end.classList.remove("on");
    }
    function play() {
      if (playing || !typed) return;
      playing = true;
      reset();
      var i = 0;
      var typeTimer = setInterval(function () {
        i += 1;
        typed.textContent = command.slice(0, i);
        if (i >= command.length) {
          clearInterval(typeTimer);
          setTimeout(revealRows, 280);
        }
      }, 42);
    }
    function revealRows() {
      rows.forEach(function (r, idx) {
        setTimeout(function () {
          var target = parseInt(r.getAttribute("data-pct"), 10) || 0;
          r.classList.add("on");
          var fill = r.querySelector(".demo-fill");
          var pct = r.querySelector(".demo-pct");
          // Width transition is CSS; force the 0% frame first so it animates.
          if (fill) requestAnimationFrame(function () { requestAnimationFrame(function () { fill.style.width = target + "%"; }); });
          if (pct) countUp(pct, target, 850);
          if (idx === rows.length - 1 && end) setTimeout(function () { end.classList.add("on"); }, 350);
          if (idx === rows.length - 1) setTimeout(function () { playing = false; }, 1200);
        }, idx * 200);
      });
      if (!rows.length) playing = false;
    }
    reset(); // clear to the 0-frame now so the final text never flashes pre-play
    setTimeout(play, 450);
    demo.addEventListener("click", play);
  } else if (demo) {
    // Reduced motion (or no matchMedia quirk): reveal final markup instantly.
    Array.prototype.slice.call(demo.querySelectorAll(".demo-row, .demo-end")).forEach(function (el) {
      el.classList.add("on");
    });
  }

  // Provider tiles: count the percent up and fill the ▓░ cells left to right,
  // once, when each tile scrolls into view. Static final glyphs stay for
  // reduced-motion, no-IntersectionObserver, and no-JS.
  var tiles = Array.prototype.slice.call(document.querySelectorAll(".hero-tiles .tile[data-pct]"));
  if (tiles.length && !reduced && "IntersectionObserver" in window) {
    var seen = new IntersectionObserver(function (entries) {
      entries.forEach(function (en) {
        if (!en.isIntersecting) return;
        var tile = en.target;
        seen.unobserve(tile);
        var target = parseInt(tile.getAttribute("data-pct"), 10) || 0;
        var pct = tile.querySelector(".tile-pct");
        var fill = tile.querySelector(".tile-fill");
        var empty = tile.querySelector(".tile-empty");
        if (pct) countUp(pct, target, 750);
        if (fill && empty) {
          var cells = Math.round(target / 10);
          var i = 0;
          fill.textContent = "";
          empty.textContent = "░░░░░░░░░░";
          var stepTimer = setInterval(function () {
            i += 1;
            var f = "";
            var e = "";
            for (var k = 0; k < i; k++) f += "▓";
            for (var j = 0; j < 10 - i; j++) e += "░";
            fill.textContent = f;
            empty.textContent = e;
            if (i >= cells) clearInterval(stepTimer);
          }, 70);
        }
      });
    }, { threshold: 0.4 });
    tiles.forEach(function (t) { seen.observe(t); });
  }
})();
