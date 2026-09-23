// Progressive enhancement only: tabs + theme toggle. Without JS, all panels show.
(function () {
  "use strict";
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
      if (panel) panel.hidden = !on;
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
})();
