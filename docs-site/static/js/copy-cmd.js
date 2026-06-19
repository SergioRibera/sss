(() => {
  const COPIED_MS = 1400;

  document.addEventListener("click", async (event) => {
    const btn = event.target.closest("[data-copy-button]");
    if (!btn) return;

    const wrap = btn.closest(".command");
    const source = wrap?.querySelector("[data-copy-source]");
    if (!source) return;

    const text = source.textContent.trim();
    try {
      await navigator.clipboard.writeText(text);
    } catch {
      const ta = document.createElement("textarea");
      ta.value = text;
      ta.style.position = "fixed";
      ta.style.opacity = "0";
      document.body.appendChild(ta);
      ta.select();
      document.execCommand("copy");
      ta.remove();
    }

    btn.dataset.copied = "true";
    const icons = btn.querySelectorAll("svg");
    if (icons.length >= 2) {
      icons[0].style.display = "none";
      icons[1].style.display = "block";
    }
    setTimeout(() => {
      delete btn.dataset.copied;
      if (icons.length >= 2) {
        icons[0].style.display = "block";
        icons[1].style.display = "none";
      }
    }, COPIED_MS);
  });
})();
