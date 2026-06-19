(() => {
  const toggle = document.querySelector("[data-binary-toggle]");
  const label = document.querySelector("[data-binary-label]");
  if (!toggle) return;

  toggle.addEventListener("click", (e) => {
    const btn = e.target.closest("button[data-binary]");
    if (!btn) return;
    toggle.querySelectorAll("button").forEach((b) => b.classList.remove("is-active"));
    btn.classList.add("is-active");
    const bin = btn.dataset.binary;
    if (label) label.textContent = bin;
    document.querySelectorAll("[data-os-section]").forEach((sec) => {
      sec.dataset.binary = bin;
    });
  });
})();
