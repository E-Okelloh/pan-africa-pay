/* Pan-Africa Pay — landing page interactions */

(function () {
  "use strict";

  // Sticky nav border when scrolled
  const nav = document.getElementById("nav");
  const onScroll = () => nav.classList.toggle("scrolled", window.scrollY > 8);
  window.addEventListener("scroll", onScroll, { passive: true });
  onScroll();

  // Mobile menu toggle
  const toggle = document.getElementById("navToggle");
  const menu = document.getElementById("mobileMenu");
  toggle.addEventListener("click", () => {
    const open = menu.hidden === false;
    menu.hidden = open;
    toggle.classList.toggle("open", !open);
    toggle.setAttribute("aria-expanded", String(!open));
  });
  menu.addEventListener("click", (event) => {
    if (event.target.closest("a")) {
      menu.hidden = true;
      toggle.classList.remove("open");
      toggle.setAttribute("aria-expanded", "false");
    }
  });

  // Scroll reveal
  const revealEls = document.querySelectorAll(".reveal");
  if ("IntersectionObserver" in window) {
    const observer = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          if (entry.isIntersecting) {
            const delay = entry.target.dataset.revealDelay;
            if (delay) entry.target.style.transitionDelay = delay + "ms";
            entry.target.classList.add("in");
            observer.unobserve(entry.target);
          }
        }
      },
      { threshold: 0.14, rootMargin: "0px 0px -40px 0px" }
    );
    revealEls.forEach((el) => observer.observe(el));
  } else {
    revealEls.forEach((el) => el.classList.add("in"));
  }

  // Animated counters
  const animateCount = (el) => {
    const target = Number(el.dataset.target) || 0;
    const duration = 1400;
    const start = performance.now();
    const tick = (now) => {
      const progress = Math.min((now - start) / duration, 1);
      const eased = 1 - Math.pow(1 - progress, 3);
      el.textContent = Math.round(target * eased).toLocaleString("en-US");
      if (progress < 1) requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
  };

  const counters = document.querySelectorAll(".counter");
  if ("IntersectionObserver" in window) {
    const observer = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          if (entry.isIntersecting) {
            animateCount(entry.target);
            observer.unobserve(entry.target);
          }
        }
      },
      { threshold: 0.5 }
    );
    counters.forEach((el) => observer.observe(el));
  } else {
    counters.forEach((el) => el.textContent = el.dataset.target);
  }
})();