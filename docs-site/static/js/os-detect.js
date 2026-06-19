(() => {
  const ua = navigator.userAgent.toLowerCase();
  const platform = (navigator.platform || "").toLowerCase();
  let os = "linux";
  if (platform.includes("mac") || ua.includes("mac os")) os = "macos";
  else if (platform.includes("win") || ua.includes("windows")) os = "windows";
  document.documentElement.dataset.os = os;

  document.querySelectorAll("[data-os-section]").forEach((section) => {
    if (section.dataset.osSection === os) section.dataset.autoActive = "true";
  });
})();
