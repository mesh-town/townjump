(function () {
  "use strict";

  var KEY = "townjump.variant.";
  var counter = 0;

  function load(name) {
    try { return window.localStorage.getItem(KEY + name); } catch (e) { return null; }
  }

  function save(name, key) {
    try { window.localStorage.setItem(KEY + name, key); } catch (e) { /* storage unavailable */ }
  }

  function groupsNamed(name) {
    var all = document.querySelectorAll("[data-var-group]");
    return Array.prototype.filter.call(all, function (g) { return g.getAttribute("data-var-group") === name; });
  }

  function apply(group, key, focus) {
    var tabs = group.querySelectorAll("[data-var-tab]");
    var panels = group.querySelectorAll("[data-var-panel]");
    Array.prototype.forEach.call(tabs, function (tab) {
      var on = tab.getAttribute("data-var-tab") === key;
      tab.setAttribute("aria-selected", on ? "true" : "false");
      tab.setAttribute("tabindex", on ? "0" : "-1");
      if (on && focus) { tab.focus(); }
    });
    Array.prototype.forEach.call(panels, function (panel) {
      panel.hidden = panel.getAttribute("data-var-panel") !== key;
    });
  }

  function select(name, key, source) {
    save(name, key);
    groupsNamed(name).forEach(function (g) { apply(g, key, g === source); });
  }

  function onKey(event, group, tabs) {
    var i = tabs.indexOf(event.currentTarget);
    var next = -1;
    if (event.key === "ArrowRight") { next = (i + 1) % tabs.length; }
    else if (event.key === "ArrowLeft") { next = (i + tabs.length - 1) % tabs.length; }
    else if (event.key === "Home") { next = 0; }
    else if (event.key === "End") { next = tabs.length - 1; }
    if (next < 0) { return; }
    event.preventDefault();
    select(group.getAttribute("data-var-group"), tabs[next].getAttribute("data-var-tab"), group);
  }

  function setup(group) {
    var id = "var" + (counter += 1);
    var bar = group.querySelector("[data-var-tabs]");
    var tabs = Array.prototype.slice.call(group.querySelectorAll("[data-var-tab]"));
    var name = group.getAttribute("data-var-group");
    bar.setAttribute("role", "tablist");
    bar.setAttribute("aria-label", "Installation method");
    bar.hidden = false;
    tabs.forEach(function (tab) {
      var key = tab.getAttribute("data-var-tab");
      tab.setAttribute("role", "tab");
      tab.setAttribute("id", id + "-tab-" + key);
      tab.setAttribute("aria-controls", id + "-panel-" + key);
      tab.addEventListener("click", function () { select(name, key, null); });
      tab.addEventListener("keydown", function (e) { onKey(e, group, tabs); });
    });
    Array.prototype.forEach.call(group.querySelectorAll("[data-var-panel]"), function (panel) {
      var key = panel.getAttribute("data-var-panel");
      var h = panel.querySelector("h3, h4");
      panel.setAttribute("role", "tabpanel");
      panel.setAttribute("id", id + "-panel-" + key);
      panel.setAttribute("aria-labelledby", id + "-tab-" + key);
      if (h) { h.classList.add("visually-hidden"); }
    });
    var keys = tabs.map(function (t) { return t.getAttribute("data-var-tab"); });
    var saved = load(name);
    apply(group, keys.indexOf(saved) >= 0 ? saved : keys[0], false);
  }

  function init() {
    Array.prototype.forEach.call(document.querySelectorAll("[data-var-group]"), setup);
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", init);
  } else {
    init();
  }
})();
