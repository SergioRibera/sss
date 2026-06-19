(() => {
  const KEY = "sss-theme";
  const root = document.documentElement;
  const btn = document.querySelector("[data-theme-toggle]");
  if (!btn) return;

  const apply = (theme) => {
    root.dataset.theme = theme;
    try { localStorage.setItem(KEY, theme); } catch {}
  };

  btn.addEventListener("click", () => {
    const next = root.dataset.theme === "light" ? "dark" : "light";
    apply(next);
  });
})();
