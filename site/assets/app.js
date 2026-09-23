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
})();
