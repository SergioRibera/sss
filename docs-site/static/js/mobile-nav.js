(() => {
  const navToggle = document.querySelector("[data-mobile-nav]");
  const links = document.getElementById("nav-links");
  if (navToggle && links) {
    navToggle.addEventListener("click", () => {
      links.classList.toggle("is-open");
      const open = links.classList.contains("is-open");
      navToggle.setAttribute("aria-expanded", open ? "true" : "false");
    });
  }

  const sidebarToggle = document.querySelector("[data-sidebar-toggle]");
  const sidebar = document.getElementById("docs-sidebar");
  if (sidebarToggle && sidebar) {
    sidebarToggle.addEventListener("click", () => {
      sidebar.classList.toggle("is-open");
      const open = sidebar.classList.contains("is-open");
      sidebarToggle.setAttribute("aria-expanded", open ? "true" : "false");
    });
  }
})();
