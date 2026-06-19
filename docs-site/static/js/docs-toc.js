(() => {
  const links = Array.from(document.querySelectorAll("[data-toc-link]"));
  if (!links.length) return;

  const map = new Map();
  links.forEach((a) => {
    const id = a.getAttribute("href").slice(1);
    const el = document.getElementById(id);
    if (el) map.set(el, a);
  });

  let current = null;
  const setActive = (link) => {
    if (current === link) return;
    if (current) current.classList.remove("is-active");
    if (link) link.classList.add("is-active");
    current = link;
  };

  const observer = new IntersectionObserver(
    (entries) => {
      const visible = entries
        .filter((e) => e.isIntersecting)
        .sort((a, b) => a.boundingClientRect.top - b.boundingClientRect.top);
      if (visible[0]) {
        const link = map.get(visible[0].target);
        if (link) setActive(link);
      }
    },
    { rootMargin: "-80px 0px -65% 0px", threshold: 0 }
  );

  map.forEach((_, el) => observer.observe(el));
})();
