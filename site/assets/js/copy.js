(function () {
  "use strict";

  var live = null;

  function ensureLive() {
    if (live) { return live; }
    live = document.createElement("div");
    live.className = "visually-hidden";
    live.setAttribute("aria-live", "polite");
    document.body.appendChild(live);
    return live;
  }

  function selectText(node) {
    var range = document.createRange();
    range.selectNodeContents(node);
    var sel = window.getSelection();
    sel.removeAllRanges();
    sel.addRange(range);
  }

  function done(button) {
    button.textContent = "Copied";
    ensureLive().textContent = "Copied";
    window.setTimeout(function () {
      button.textContent = "Copy";
      ensureLive().textContent = "";
    }, 2000);
  }

  function copy(pre, button) {
    var code = pre.querySelector("code") || pre;
    var text = code.textContent;
    if (navigator.clipboard && navigator.clipboard.writeText) {
      navigator.clipboard.writeText(text).then(function () { done(button); }, function () { selectText(code); });
    } else {
      selectText(code);
    }
  }

  function init() {
    var blocks = document.querySelectorAll("pre.cmd");
    Array.prototype.forEach.call(blocks, function (pre) {
      var button = document.createElement("button");
      button.type = "button";
      button.className = "copy-btn";
      button.textContent = "Copy";
      button.setAttribute("aria-label", "Copy command");
      button.addEventListener("click", function () { copy(pre, button); });
      pre.appendChild(button);
    });
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", init);
  } else {
    init();
  }
})();
