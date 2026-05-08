// KOL CDP collector — injected via Runtime.addBinding + Page.addScriptToEvaluateOnNewDocument.
//
// Talks back to Rust through the `__kolPush` binding:
//   { type: "video", aweme_id, title?, suggest_word?, share_url?, first_frame_url? }
//     → enqueued by gather.rs for bulk POST to /api/douyin/videos/bulk
//   { type: "log",   level?, msg?, extra? } → tracing only
//
// Mirrors content.js extraction logic so both CDP and extension paths
// produce schema-equivalent rows.

(function () {
  if (window.__kolCollectorInstalled) return;
  window.__kolCollectorInstalled = true;

  function push(obj) {
    try {
      window.__kolPush(JSON.stringify(obj));
    } catch (_) {
      // Binding may not be ready on the very first tick; harmless.
    }
  }

  push({ type: "log", level: "info", msg: "collector installed",
         extra: { url: location.href, readyState: document.readyState } });

  // ---- extraction (mirrors content.js extractActiveVideo) ---------------

  const SEARCH_DEFAULT = "搜索你感兴趣的内容";

  function extractActiveVideo() {
    // Primary: feed-active-video carries the aweme id on data-e2e-vid.
    let active = document.querySelector('[data-e2e="feed-active-video"][data-e2e-vid]');
    if (!active) {
      const card = document.querySelector('[data-e2e="feed-item"].page-recommend-container');
      active = card ? card.querySelector('[data-e2e="feed-video"][data-e2e-vid]') : null;
    }
    if (!active) return null;

    const awemeId = active.getAttribute("data-e2e-vid");
    if (!awemeId) return null;

    // Title from the matching video-info block (filter by aweme id to avoid
    // picking up prev/next slides that are pre-rendered in the DOM).
    let title = null;
    const info = document.querySelector(
      `[data-e2e="video-info"][data-e2e-aweme-id="${awemeId}"]`
    );
    if (info) {
      const desc = info.querySelector('[data-e2e="video-desc"]');
      if (desc) {
        title = (desc.innerText || "").trim().replace(/\s+/g, " ").slice(0, 1000) || null;
      }
    }

    // suggest_word: sibling div of searchbar-input, NOT the input value/placeholder.
    let suggestWord = null;
    const searchInput = document.querySelector('[data-e2e="searchbar-input"]');
    if (searchInput && searchInput.parentElement) {
      for (const sib of searchInput.parentElement.children) {
        if (sib === searchInput) continue;
        if (sib.tagName === "INPUT" || sib.tagName === "BUTTON") continue;
        if (sib.contains(searchInput)) continue;
        const text = (sib.innerText || "").trim();
        if (!text || text === "搜索" || text === SEARCH_DEFAULT) continue;
        suggestWord = text.slice(0, 200);
        break;
      }
      if (!suggestWord) {
        const v = (searchInput.value || "").trim();
        const ph = (searchInput.placeholder || "").trim();
        const cand = v || (ph && ph !== SEARCH_DEFAULT ? ph : "");
        if (cand) suggestWord = cand.slice(0, 200);
      }
    }

    // first_frame_url: cover thumbnail from active element or enclosing feed-item.
    let firstFrame = null;
    const roots = [active, active.closest('[data-e2e="feed-item"]'), info].filter(Boolean);
    outer: for (const root of roots) {
      for (const im of root.querySelectorAll("img")) {
        const s = im.getAttribute("src") || im.getAttribute("data-src") || "";
        if (s.includes("origin_cover") || s.includes("pcweb_cover")) {
          firstFrame = s.startsWith("//") ? "https:" + s : s;
          break outer;
        }
      }
    }

    return {
      aweme_id: awemeId,
      title: title || null,
      suggest_word: suggestWord || null,
      share_url: "https://www.douyin.com/video/" + awemeId,
      first_frame_url: firstFrame || null,
    };
  }

  // ---- slide driver + dedup + push pipeline ------------------------------

  // Per-session seen set so one rotation doesn't emit the same aweme twice.
  // MAX_VIDEOS caps the seen Set so memory stays bounded after the Rust
  // side disconnects CDP (browsers are left open after stop()).
  const seen = new Set();
  const MAX_VIDEOS = 200;
  const SLIDE_INTERVAL_MS = 3000;
  const POLL_MS = 2000;

  // Timer handles so we can cancel on cap hit.
  let slideTimer = null;
  let pollTimer = null;
  let debounceTimer = null;

  function stopTimers() {
    if (slideTimer) { clearInterval(slideTimer); slideTimer = null; }
    if (pollTimer)  { clearInterval(pollTimer);  pollTimer  = null; }
    if (debounceTimer) { clearTimeout(debounceTimer); debounceTimer = null; }
    if (observer) { observer.disconnect(); observer = null; }
  }

  function tryPush() {
    if (seen.size >= MAX_VIDEOS) { stopTimers(); return; }
    const row = extractActiveVideo();
    if (!row || !row.aweme_id || seen.has(row.aweme_id)) return;
    seen.add(row.aweme_id);
    push({ type: "video", ...row });
    if (seen.size >= MAX_VIDEOS) {
      push({ type: "log", level: "info", msg: "collector: MAX_VIDEOS reached, stopping" });
      stopTimers();
    }
  }

  function dispatchNextSlide() {
    const nextBtn = document.querySelector('[data-e2e="video-switch-next-arrow"]');
    if (nextBtn) { try { nextBtn.click(); return; } catch (_) {} }
    document.dispatchEvent(new KeyboardEvent("keydown", {
      key: "ArrowDown", code: "ArrowDown", keyCode: 40, which: 40,
      bubbles: true, cancelable: true,
    }));
  }

  // MutationObserver on the slider subtree — narrowed to slideList when present
  // to avoid firing on every React micro-update in the wider DOM.
  const OBSERVER_OPTS = {
    subtree: true, childList: true, attributes: true,
    attributeFilter: ["data-e2e", "data-e2e-aweme-id", "data-e2e-vid", "class"],
  };
  let observerRoot = null;
  let observer = null;

  function findObserverRoot() {
    const sl = document.querySelector('[data-e2e="slideList"]');
    if (sl) return sl;
    const active = document.querySelector('[data-e2e="feed-active-video"]');
    if (active) {
      const fi = active.closest('[data-e2e="feed-item"]');
      if (fi && fi.parentElement) return fi.parentElement;
    }
    return null;
  }

  function attachObserver() {
    observerRoot = findObserverRoot() || document.body;
    observer = new MutationObserver(() => {
      clearTimeout(debounceTimer);
      // 600ms debounce: slider transitions update several DOM points in
      // quick succession; wait for the page to settle before extracting.
      debounceTimer = setTimeout(tryPush, 600);
    });
    observer.observe(observerRoot, OBSERVER_OPTS);
  }

  function start() {
    tryPush();
    attachObserver();

    // Periodic poll: catches rotations the MutationObserver misses and
    // narrows the observer root to slideList once it mounts.
    pollTimer = setInterval(() => {
      if (observerRoot === document.body) {
        const better = findObserverRoot();
        if (better) {
          observer.disconnect();
          observerRoot = better;
          observer.observe(better, OBSERVER_OPTS);
          push({ type: "log", level: "info", msg: "observer narrowed to slideList" });
        }
      }
      tryPush();
    }, POLL_MS);

    // Drive the slider forward at a fixed pace so the infinite feed
    // keeps advancing even when autoplay doesn't trigger next-slide.
    slideTimer = setInterval(dispatchNextSlide, SLIDE_INTERVAL_MS);
  }

  if (document.readyState === "complete") {
    setTimeout(start, 1500);
  } else {
    window.addEventListener("load", () => setTimeout(start, 1500));
  }
})();
