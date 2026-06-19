(() => {
  // ---- Binary toggle (sss vs sss_code) ----
  const binToggle = document.querySelector("[data-binary-toggle]");
  const binLabel  = document.querySelector("[data-binary-label]");

  if (binToggle) {
    binToggle.addEventListener("click", (e) => {
      const btn = e.target.closest("button[data-binary]");
      if (!btn) return;
      const bin = btn.dataset.binary;

      binToggle.querySelectorAll("button").forEach((b) => {
        const on = b === btn;
        b.classList.toggle("is-active", on);
        b.setAttribute("aria-selected", on ? "true" : "false");
      });
      if (binLabel) binLabel.textContent = bin;

      document.querySelectorAll("[data-binary-pane]").forEach((p) => {
        p.hidden = p.dataset.binaryPane !== bin;
      });
    });
  }

  // ---- Per-OS tabs (Package managers / Direct download) ----
  document.querySelectorAll("[data-install-tabs]").forEach((root) => {
    const tablist = root.querySelector(".install-tabs-tablist");
    if (!tablist) return;
    tablist.addEventListener("click", (e) => {
      const btn = e.target.closest("button[data-tab]");
      if (!btn) return;
      const id = btn.dataset.tab;
      root.querySelectorAll("[data-tab]").forEach((b) => {
        const on = b.dataset.tab === id;
        b.classList.toggle("is-active", on);
        b.setAttribute("aria-selected", on ? "true" : "false");
      });
      root.querySelectorAll("[data-tab-panel]").forEach((p) => {
        const on = p.dataset.tabPanel === id;
        p.classList.toggle("is-active", on);
        p.hidden = !on;
      });
    });
  });

  // ---- "your OS" auto-detect badge ----
  const ua = navigator.userAgent || navigator.platform || "";
  let detected = null;
  if (/Mac|iPhone|iPad/i.test(ua))      detected = "macos";
  else if (/Linux|X11/i.test(ua))       detected = "linux";
  else if (/Win/i.test(ua))             detected = "windows";

  if (detected) {
    document.querySelectorAll(`[data-os-section="${detected}"] [data-os-detected]`)
      .forEach((el) => { el.hidden = false; });

    document.querySelectorAll(`[data-os-section="${detected}"]`)
      .forEach((el) => { el.classList.add("is-detected"); });
  }
})();
