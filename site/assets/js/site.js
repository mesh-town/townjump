(function () {
  "use strict";

  var SLUGS = ["macos", "windows", "linux", "android", "router", "docker"];
  var NAMES = { macos: "macOS", windows: "Windows", linux: "Linux", android: "Android", router: "a router", docker: "Docker" };
  var tabs = [];
  var panels = [];

  function detectOS(env) {
    env = env || {};
    var ua = env.ua || "";
    var platform = env.platform || "";
    var uaPlatform = env.uaPlatform || "";
    if (/Android/.test(ua)) { return "android"; }
    if (/iPhone|iPad|iPod/.test(ua)) { return null; }
    if (uaPlatform === "macOS") { return "macos"; }
    if (uaPlatform === "Windows") { return "windows"; }
    if (uaPlatform === "Linux") { return "linux"; }
    if (/^Mac/.test(platform)) { return "macos"; }
    if (/^Win/.test(platform)) { return "windows"; }
    if (/Linux/.test(platform)) { return "linux"; }
    return null;
  }

  function setNote(text) {
    var note = document.querySelector("[data-os-note]");
    if (note) { note.textContent = text || ""; }
  }

  function select(slug, opts) {
    opts = opts || {};
    if (SLUGS.indexOf(slug) < 0) { return; }
    tabs.forEach(function (tab) {
      var on = tab.getAttribute("data-os-tab") === slug;
      tab.setAttribute("aria-selected", on ? "true" : "false");
      tab.setAttribute("tabindex", on ? "0" : "-1");
      if (on && opts.focus) { tab.focus(); }
    });
    panels.forEach(function (panel) {
      panel.hidden = panel.getAttribute("data-os-panel") !== slug;
    });
    if (opts.record && window.history && window.history.replaceState) {
      window.history.replaceState(null, "", "#os=" + slug);
    }
  }

  function onKey(event) {
    var i = tabs.indexOf(event.currentTarget);
    var next = -1;
    if (event.key === "ArrowRight") { next = (i + 1) % tabs.length; }
    else if (event.key === "ArrowLeft") { next = (i + tabs.length - 1) % tabs.length; }
    else if (event.key === "Home") { next = 0; }
    else if (event.key === "End") { next = tabs.length - 1; }
    if (next < 0) { return; }
    event.preventDefault();
    setNote("");
    select(tabs[next].getAttribute("data-os-tab"), { focus: true, record: true });
  }

  function hashSlug() {
    var m = /^#os=([a-z]+)$/.exec(window.location.hash || "");
    return m && SLUGS.indexOf(m[1]) >= 0 ? m[1] : null;
  }

  function setupTabs() {
    var bar = document.querySelector("[data-os-tabs]");
    if (!bar) { return false; }
    tabs = Array.prototype.slice.call(bar.querySelectorAll("[data-os-tab]"));
    panels = Array.prototype.slice.call(document.querySelectorAll("[data-os-panel]"));
    bar.setAttribute("role", "tablist");
    bar.setAttribute("aria-label", "Operating system");
    bar.hidden = false;
    tabs.forEach(function (tab) {
      var slug = tab.getAttribute("data-os-tab");
      tab.setAttribute("role", "tab");
      tab.setAttribute("id", "os-tab-" + slug);
      tab.setAttribute("aria-controls", "os-panel-" + slug);
      tab.addEventListener("click", function () { setNote(""); select(slug, { record: true }); });
      tab.addEventListener("keydown", onKey);
    });
    panels.forEach(function (panel) {
      var slug = panel.getAttribute("data-os-panel");
      var h3 = panel.querySelector("h3");
      panel.setAttribute("role", "tabpanel");
      panel.setAttribute("id", "os-panel-" + slug);
      panel.setAttribute("aria-labelledby", "os-tab-" + slug);
      if (h3) { h3.classList.add("visually-hidden"); }
    });
    return true;
  }

  function init() {
    if (!setupTabs()) { return; }
    var nav = window.navigator || {};
    var uad = nav.userAgentData;
    var os = hashSlug() || detectOS({
      ua: nav.userAgent || "", platform: nav.platform || "",
      uaPlatform: uad && uad.platform ? uad.platform : ""
    });
    if (os) {
      select(os);
      if (!hashSlug()) { setNote("Detected: " + NAMES[os] + "."); }
    } else {
      select("macos");
    }
    window.addEventListener("hashchange", function () {
      var slug = hashSlug();
      if (slug) { setNote(""); select(slug); }
    });
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", init);
  } else {
    init();
  }
})();
