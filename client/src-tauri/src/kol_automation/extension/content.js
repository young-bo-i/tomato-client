// Donut KOL Helper — content script.
//
// Injected into every douyin.com page for profiles tagged
// `kol_platform=douyin`. Provides two floating buttons:
//
//   📥 抓取 DOM   — one-off downloader for selector-design (bypasses
//                   Wayfern's paywalled CDP entirely).
//   ▶️ 开始采集   — toggles the gather pipeline. While active, observes
//                   the active video card via MutationObserver, simulates
//                   ArrowDown to advance through the slider feed, and
//                   batches extracted rows to the service worker which
//                   forwards them to the Donut Tauri local axum server.
//                   That server in turn POSTs to the tomato-server
//                   /api/douyin/videos/bulk.
//
// Idempotent — re-injection on history-driven re-execution is a no-op.

(function () {
  if (window.__kolHelperInstalled) return;
  window.__kolHelperInstalled = true;

  // ------------ profile_id baked in by Tauri ------------------------------

  let __profileId = null;
  fetch(chrome.runtime.getURL("profile.json"))
    .then((r) => r.json())
    .then((j) => {
      __profileId = j && j.profile_id ? String(j.profile_id) : null;
      console.log("[kol-helper] profile_id =", __profileId);
      // First state ping + server flag sync as soon as we know who we
      // are — eager so the batch UI sees this profile online quickly.
      void reportLoginState();
      void syncWithServerFlag();
    })
    .catch((e) => console.warn("[kol-helper] profile.json read failed", e));

  // ------------ direct fetch to Donut local axum -------------------------
  //
  // We used to dispatch via `chrome.runtime.sendMessage` to a service
  // worker, but Wayfern (or Chromium-via-Wayfern) seems to keep the
  // MV3 service worker dormant in a way our wakeup events don't break
  // out of — observed via SW DevTools where even `1+1` doesn't return.
  //
  // Content scripts in MV3 with host_permissions for the target host
  // can fetch directly, no SW needed. The server has CORS allow-origin
  // wildcard and uses no-Content-Type body to skip preflight.
  //
  // Probe ports lazily; the first fetch establishes the cached port,
  // subsequent calls reuse it. Cache is reset on any failure so a
  // restart of Donut (and consequent port shift) recovers automatically.

  const KOL_PORT_CANDIDATES = [10108, 10109, 10110, 10111, 10112];
  let __kolPort = null;

  async function kolFetchDirect(pathAndQuery, init) {
    if (__kolPort === null) {
      for (const p of KOL_PORT_CANDIDATES) {
        try {
          const r = await fetch(`http://127.0.0.1:${p}/kol-ext/health`);
          if (r.ok) {
            __kolPort = p;
            console.log("[kol-helper] axum probe hit port", p);
            break;
          }
        } catch (_) {
          /* try next */
        }
      }
      if (__kolPort === null) {
        throw new Error("no Donut local axum on 10108..10112");
      }
    }
    try {
      const r = await fetch(
        `http://127.0.0.1:${__kolPort}${pathAndQuery}`,
        init,
      );
      if (!r.ok) {
        let text = "";
        try {
          text = await r.text();
        } catch (_) {}
        throw new Error(`status ${r.status} ${text.slice(0, 200)}`);
      }
      return await r.json();
    } catch (e) {
      // Reset cached port on any failure so the next call re-probes.
      __kolPort = null;
      throw e;
    }
  }

  // ------------ login state detection ------------------------------------

  // Three states mirror the Rust side (kol_automation/ingest.rs).
  // The detector is a structural sniff — class names rotate per Douyin
  // build, so we only look for `data-e2e` markers + element ids.
  //
  // Avoid using nav icons (notice-entry / im-entry / something-button)
  // as auth signals — Douyin renders them for guests as well, just
  // routing them through the login flow on click.
  function detectLoginState() {
    // Strong positive: actual feed content is in the DOM. Both of
    // these only appear once Douyin's React tree has hydrated with
    // post-login data.
    if (
      document.querySelector('[data-e2e="feed-active-video"]') ||
      document.querySelector('[data-e2e="feed-item"]')
    ) {
      return "authenticated";
    }
    // Strong negative: login affordance is visible.
    //   - The QR-scan modal injects an element with this id.
    //   - The header "登录" button: Douyin uses an obfuscated class
    //     (e.g. `r2P1NdJa`) but always wraps the literal text "登录"
    //     in a <p> inside a <button>. Scan top buttons by text.
    if (document.getElementById("douyin_login_comp_scan_code")) {
      return "unauthenticated";
    }
    const buttons = document.querySelectorAll("button");
    const cap = Math.min(buttons.length, 100);
    for (let i = 0; i < cap; i++) {
      const txt = (buttons[i].textContent || "").trim();
      if (txt === "登录" || txt === "立即登录") {
        return "unauthenticated";
      }
    }
    // Page chrome ready but no feed and no clear login affordance —
    // SPA still hydrating. Caller treats `unknown` as "not yet
    // authenticated, don't auto-close, don't auto-gather".
    if (document.querySelector('[data-e2e="searchbar-input"]')) {
      return "unknown";
    }
    return "unknown";
  }

  let __lastReportedState = null;
  async function reportLoginState() {
    if (!__profileId) return;
    const state = detectLoginState();
    if (state === __lastReportedState) return;
    __lastReportedState = state;
    console.log("[kol-helper] login state →", state);
    try {
      await kolFetchDirect("/kol-ext/state", {
        method: "POST",
        body: JSON.stringify({
          profile_id: __profileId,
          state,
          url: location.href,
        }),
      });
    } catch (e) {
      // Reset dedupe so next poll retries the report.
      __lastReportedState = null;
      console.warn("[kol-helper] state report failed", e.message || e);
    }
  }
  // Poll independently of gather — Tauri panel wants visibility even
  // when the user isn't actively gathering. 15s is generous because
  // the report itself dedupes on state-change; most ticks are no-ops.
  setInterval(() => {
    void reportLoginState();
  }, 15000);

  // ------------ last-gasp report on page close --------------------------
  //
  // Use case: the user opens a browser, completes login on Douyin, then
  // closes the window before the next 15s polling tick fires. Without
  // this hook the server keeps thinking the profile is unauthenticated
  // (or unknown) and the notification dispatcher emails a false alarm.
  //
  // sendBeacon is the browser-recommended pattern for "fire one last
  // request as the page goes away":
  //   - Doesn't block the unload
  //   - Survives the page tear-down (browser keeps the request in
  //     flight even after the JS context dies)
  //   - Default Content-Type is text/plain; our Tauri /kol-ext/state
  //     handler reads raw bytes so it doesn't care
  //
  // Hook BOTH `pagehide` (modern, fires on BFCache navigation too)
  // AND `beforeunload` (older, more universally fires on tab close)
  // for maximum coverage. Server upsert is idempotent, two beacons
  // hitting it back-to-back just writes the same row twice.
  function lastGaspReport() {
    if (!__profileId || __kolPort === null) return;
    try {
      const state = detectLoginState();
      const payload = JSON.stringify({
        profile_id: __profileId,
        state,
        url: location.href,
      });
      navigator.sendBeacon(
        `http://127.0.0.1:${__kolPort}/kol-ext/state`,
        payload,
      );
    } catch (_) {
      // beforeunload has no time for retries; silent fail.
    }
  }
  window.addEventListener("pagehide", lastGaspReport);
  window.addEventListener("beforeunload", lastGaspReport);

  // ------------ server-driven gather flag --------------------------------

  // The "batch start/stop" button on the Tauri panel flips a per-profile
  // boolean on the server. We poll it every 10s and bring local gather
  // state into agreement: start when the flag goes true (and we're
  // logged in), stop when it goes false.
  //
  // The user's manual ▶️ button still works — it sets local state
  // directly. If they manually start while the server flag says false,
  // the next poll will stop them; that's intentional (server is the
  // authority during a batch session). The button effectively acts as
  // a per-tab override only between polls.
  async function syncWithServerFlag() {
    if (!__profileId) return;
    let resp;
    try {
      resp = await kolFetchDirect(
        `/kol-ext/gather/should?profile_id=${encodeURIComponent(__profileId)}`,
      );
    } catch (e) {
      console.warn("[kol-helper] sync fetch failed:", e.message || e);
      return;
    }
    if (!resp) return;
    const should = !!resp.should_gather;
    const isGathering = !!gatherState;
    if (should && !isGathering) {
      if (detectLoginState() === "authenticated") {
        console.log("[kol-helper] server flag=true → auto startGather");
        startGather();
      }
    } else if (!should && isGathering) {
      console.log("[kol-helper] server flag=false → auto stopGather");
      stopGather();
    }
  }
  // 10s — the batch start/stop button doesn't expect sub-second
  // response. Every browser polling at 4s caused 50×0.25Hz=12.5/s of
  // localhost traffic which adds up against 200+ profile scenarios.
  setInterval(() => {
    void syncWithServerFlag();
  }, 10000);
  // First sync as soon as profile.json is loaded (covered by the
  // earlier reportLoginState call chain in profile.json's then block).

  // ------------ helpers ----------------------------------------------------

  function attrs(node, max) {
    if (!node || !node.attributes) return [];
    return Array.from(node.attributes)
      .slice(0, max || 12)
      .map((a) => ({ name: a.name, value: String(a.value).slice(0, 240) }));
  }
  function dataAttrs(node) {
    if (!node || !node.dataset) return {};
    const out = {};
    for (const k in node.dataset) {
      out[k] = String(node.dataset[k]).slice(0, 240);
    }
    return out;
  }

  // ------------ DUMP probe (B1, unchanged) --------------------------------

  function collectProbe() {
    const counts = {
      videos: document.querySelectorAll("video").length,
      images: document.querySelectorAll("img").length,
      anchors: document.querySelectorAll("a[href]").length,
      lis: document.querySelectorAll("li").length,
      articles: document.querySelectorAll("article").length,
      dataE2e: document.querySelectorAll("[data-e2e]").length,
      dataId: document.querySelectorAll("[data-id]").length,
      dataKey: document.querySelectorAll("[data-key]").length,
    };
    const dataE2eSeen = {};
    document.querySelectorAll("[data-e2e]").forEach((el) => {
      const v = el.getAttribute("data-e2e");
      if (v) dataE2eSeen[v] = (dataE2eSeen[v] || 0) + 1;
    });
    const distinctDataE2e = Object.keys(dataE2eSeen)
      .sort()
      .map((k) => ({ value: k, count: dataE2eSeen[k] }));

    const seenSig = {};
    const candidates = [];
    const probes = document.querySelectorAll(
      "li, article, [data-e2e], [data-id], [data-key]",
    );
    for (const el of probes) {
      if (candidates.length >= 8) break;
      if (!el.querySelector("video, img")) continue;
      const hasLink = !!el.querySelector("a[href]");
      const text = (el.innerText || "").trim();
      if (!hasLink && text.length < 5) continue;
      const cls = typeof el.className === "string" ? el.className : "";
      const sig = el.tagName + "|" + cls.slice(0, 60);
      if (seenSig[sig]) continue;
      seenSig[sig] = 1;
      const anchor = el.querySelector("a[href]");
      const video = el.querySelector("video");
      const img = el.querySelector("img");
      candidates.push({
        tag: el.tagName,
        cls: cls.slice(0, 200),
        data: dataAttrs(el),
        attrs: attrs(el, 14),
        text: text.slice(0, 240),
        anchor: anchor
          ? {
              href: anchor.getAttribute("href"),
              attrs: attrs(anchor, 10),
              data: dataAttrs(anchor),
              text: (anchor.innerText || "").trim().slice(0, 200),
            }
          : null,
        img: img
          ? {
              src: img.getAttribute("src") || img.getAttribute("data-src"),
              attrs: attrs(img, 10),
              data: dataAttrs(img),
            }
          : null,
        video: video
          ? {
              src: video.getAttribute("src"),
              poster: video.getAttribute("poster"),
              attrs: attrs(video, 10),
              data: dataAttrs(video),
            }
          : null,
        outerHTMLPreview: (el.outerHTML || "").slice(0, 1200),
      });
    }
    return {
      url: location.href,
      title: document.title,
      timestamp: Date.now(),
      counts,
      distinctDataE2e,
      candidates,
    };
  }

  function downloadJson(filename, payload) {
    const blob = new Blob([JSON.stringify(payload, null, 2)], {
      type: "application/json",
    });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = filename;
    a.style.display = "none";
    document.documentElement.appendChild(a);
    a.click();
    a.remove();
    setTimeout(() => URL.revokeObjectURL(url), 1500);
  }

  // ------------ GATHER ----------------------------------------------------

  // Tunables.
  // Time between forced "next video" key dispatches. 3s is a fast skim
  // pace — videos are 15-60s, so we capture metadata then advance long
  // before the video would have ended naturally. Going below 2s starts
  // tripping Douyin's throttling, occasionally swallowing an aweme.
  const SLIDE_INTERVAL_MS = 3000;
  const POLL_MS = 5000; // belt-and-suspenders, paired with MutationObserver
  const MAX_VIDEOS = 200; // per browser session — prevents runaway
  // Cap on simultaneous in-flight POSTs from this single browser. The
  // gather pipeline is dominated by the 3s slide rate (≤1 row/3s/browser),
  // so 4 concurrent uploads is plenty of slack for transient server slow
  // responses without queueing rows behind a slow flush.
  const MAX_INFLIGHT_UPLOADS = 4;

  // Assembled at ▶️ click; nulled at ⏸ click.
  let gatherState = null;

  // Toggle the declarativeNetRequest video/image blocking ruleset. The
  // ruleset is OFF by default (see manifest.json) so manual browsing
  // doesn't see flicker from blocked thumbnails — we flip it ON when
  // gather starts, OFF when gather stops. Sent to background.js since
  // chrome.declarativeNetRequest is only callable from the SW context.
  // Best-effort: if Wayfern's SW is dormant the message no-ops silently
  // and the worst case is "rules stay off during gather" — bandwidth
  // not saved, but collection itself still works.
  function setBlockingEnabled(enable) {
    try {
      chrome.runtime.sendMessage(
        { kind: "set-block-enabled", enable: !!enable },
        (resp) => {
          if (chrome.runtime.lastError) {
            console.warn(
              "[kol-helper] block toggle (SW dormant?):",
              chrome.runtime.lastError.message,
            );
            return;
          }
          if (!resp || !resp.ok) {
            console.warn("[kol-helper] block toggle failed:", resp);
          }
        },
      );
    } catch (e) {
      console.warn("[kol-helper] block toggle exception:", e);
    }
  }

  function extractActiveVideo() {
    // The canonical "currently visible" hook on Douyin's follow page is
    // `[data-e2e="feed-active-video"]`. Its data-e2e VALUE rotates as
    // the user slides — the previously-active element loses the marker
    // and the new active one gains it. The data-e2e-vid attribute on
    // that same element is the aweme_id.
    //
    // We did try `[data-e2e="feed-item"].page-recommend-container` as
    // the primary selector earlier, but that turned out to be a stale
    // marker tied to the swiper's initial landing slide — it doesn't
    // re-target on slide transitions. So `feed-active-video` first,
    // and only fall back to the page-recommend-container path if the
    // page hasn't yet promoted any slide to active (e.g. very first
    // tick before the swiper has a chance to label the current).
    let active = document.querySelector(
      '[data-e2e="feed-active-video"][data-e2e-vid]',
    );
    if (!active) {
      const card = document.querySelector(
        '[data-e2e="feed-item"].page-recommend-container',
      );
      active = card
        ? card.querySelector('[data-e2e="feed-video"][data-e2e-vid]')
        : null;
    }
    if (!active) return null;

    const awemeId = active.getAttribute("data-e2e-vid");
    if (!awemeId) return null;

    // Locate the video-info bag matching this aweme. There can be 2-3
    // video-info elements rendered concurrently (prev/curr/next slides),
    // so always filter by id.
    const info = document.querySelector(
      `[data-e2e="video-info"][data-e2e-aweme-id="${awemeId}"]`,
    );

    let title = null;
    if (info) {
      const desc = info.querySelector('[data-e2e="video-desc"]');
      if (desc) {
        title = (desc.innerText || "")
          .trim()
          .replace(/\s+/g, " ")
          .slice(0, 1000);
      }
    }

    // suggest_word: Douyin's algorithmic per-video search suggestion
    // appears as a SIBLING div of the searchbar-input inside its flex
    // parent. Class names are obfuscated (`VYb0YFbL`, `LQnG3CYw`...)
    // and rotate per build, so we walk siblings structurally instead.
    //
    // The sibling div is conditionally rendered by Douyin's React tree —
    // it may or may not be present at any given moment depending on
    // input focus, animation state, and whether the active video has a
    // suggestion at all. When absent, we fall back to value/placeholder.
    //
    // We deliberately do NOT use the in-description #hashtag list as a
    // fallback: those are user-authored tags, not Douyin's algorithmic
    // suggestion, and conflating them muddies downstream targeting.
    const SEARCH_DEFAULT = "搜索你感兴趣的内容";
    let suggestWord = null;
    const searchInput = document.querySelector('[data-e2e="searchbar-input"]');
    if (searchInput && searchInput.parentElement) {
      // Pass 1: structural sibling — the actual suggest container.
      for (const sib of searchInput.parentElement.children) {
        if (sib === searchInput) continue;
        if (sib.tagName === "INPUT" || sib.tagName === "BUTTON") continue;
        if (sib.contains(searchInput)) continue;
        const text = (sib.innerText || "").trim();
        if (!text) continue;
        // Skip well-known generic labels Douyin sticks here.
        if (text === "搜索" || text === SEARCH_DEFAULT) continue;
        suggestWord = text.slice(0, 200);
        break;
      }
      // Pass 2 (fallback): some Douyin builds may dump the suggestion
      // directly into the input element itself.
      if (!suggestWord) {
        const v = (searchInput.value || "").trim();
        const ph = (searchInput.placeholder || "").trim();
        const cand = v || (ph && ph !== SEARCH_DEFAULT ? ph : "");
        if (cand) suggestWord = cand.slice(0, 200);
      }
    }

    // first_frame_url: cover thumbnail. Search the active element first,
    // then walk up to the enclosing feed-item if needed.
    let firstFrame = null;
    const searchRoots = [
      active,
      active.closest('[data-e2e="feed-item"]'),
      info,
    ].filter(Boolean);
    outer: for (const root of searchRoots) {
      for (const im of root.querySelectorAll("img")) {
        const s = im.getAttribute("src") || im.getAttribute("data-src") || "";
        if (s.includes("origin_cover") || s.includes("pcweb_cover")) {
          firstFrame = s.startsWith("//") ? "https:" + s : s;
          break outer;
        }
      }
    }

    const shareUrl = `https://www.douyin.com/video/${awemeId}`;

    return {
      profile_id: __profileId,
      aweme_id: awemeId,
      title: title || null,
      suggest_word: suggestWord,
      share_url: shareUrl,
      first_frame_url: firstFrame,
      captured_at: new Date().toISOString(),
    };
  }

  function dispatchNextSlide() {
    // Try clicking Douyin's own "next" arrow first — it's the path the
    // user would take, so it's the least risky to trigger Douyin's
    // anti-cheat heuristics.
    const nextBtn = document.querySelector(
      "[data-e2e=\"video-switch-next-arrow\"]",
    );
    if (nextBtn) {
      try {
        nextBtn.click();
        return "click";
      } catch (_) {}
    }
    // Fallback: synthesize ArrowDown on the document.
    document.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "ArrowDown",
        code: "ArrowDown",
        keyCode: 40,
        which: 40,
        bubbles: true,
        cancelable: true,
      }),
    );
    return "key";
  }

  // Single-row upload, fire-and-forget. The caller does NOT await — we
  // don't want a slow server response to stall the slide/observe loop
  // and reduce our scrape rate. `inflight` tracks concurrent POSTs so
  // we can backpressure if the server gets slow (rare in practice
  // because slide rate caps us at ~1 row/3s per browser).
  async function postRow(row) {
    const state = gatherState;
    if (!state) return;
    if (state.inflight >= MAX_INFLIGHT_UPLOADS) {
      // Back off: defer this row to the retry queue so we don't pile
      // up more than MAX_INFLIGHT concurrent fetches. Next observer
      // tick or pollTimer will retry from retryQueue.
      state.retryQueue.push(row);
      return;
    }
    state.inflight += 1;
    try {
      const r = await kolFetchDirect("/kol-ext/gather/bulk", {
        method: "POST",
        body: JSON.stringify([row]),
      });
      if (state === gatherState) {
        state.uploaded += Number(r.inserted || 0);
        state.duplicates += Number(r.duplicates || 0);
      }
    } catch (e) {
      console.warn("[kol-helper] post failed:", e.message || e);
      if (state === gatherState) {
        state.errors += 1;
        state.retryQueue.push(row);
      }
    } finally {
      if (state === gatherState) {
        state.inflight -= 1;
      }
      updateButton();
    }
  }

  // Drain whatever is sitting in the retry queue, subject to the
  // inflight cap. Called opportunistically after every enqueue and
  // from the periodic poll.
  function drainRetryQueue() {
    const state = gatherState;
    if (!state) return;
    while (
      state.retryQueue.length > 0 &&
      state.inflight < MAX_INFLIGHT_UPLOADS
    ) {
      const row = state.retryQueue.shift();
      void postRow(row);
    }
  }

  function tryEnqueue() {
    if (!gatherState) return;
    const row = extractActiveVideo();
    if (!row) return;
    if (!row.profile_id) {
      console.warn("[kol-helper] no profile_id — skipping");
      return;
    }
    if (gatherState.seen.has(row.aweme_id)) return;
    gatherState.seen.add(row.aweme_id);
    gatherState.captured += 1;
    // Upload immediately, do NOT await — keeps the slide loop free to
    // advance to the next video while the previous row is in flight.
    void postRow(row);
    // Opportunistically retry any rows that bounced off the inflight cap.
    drainRetryQueue();
    if (gatherState.captured >= MAX_VIDEOS) {
      console.log("[kol-helper] hit MAX_VIDEOS, stopping");
      stopGather();
    }
  }

  function startGather() {
    if (gatherState) return;
    const loginState = detectLoginState();
    if (loginState !== "authenticated") {
      console.warn(
        "[kol-helper] gather refused — login state is",
        loginState,
      );
      // Surface on the gather button so the user notices without
      // opening DevTools. Reset back to default after a moment.
      gatherBtn.textContent =
        loginState === "unknown" ? "页面加载中..." : "未登录,请先登录";
      gatherBtn.style.background = "#ef4444";
      void reportLoginState();
      setTimeout(() => updateButton(), 3000);
      return;
    }
    console.log("[kol-helper] gather start");
    setBlockingEnabled(true);
    gatherState = {
      seen: new Set(),
      // retryQueue holds rows that bounced off MAX_INFLIGHT_UPLOADS or
      // failed mid-fetch. Drained by drainRetryQueue() on every enqueue
      // tick + the 5s pollTimer, so we don't lose rows under load.
      retryQueue: [],
      inflight: 0,
      captured: 0,
      uploaded: 0,
      duplicates: 0,
      errors: 0,
      observer: null,
      slideTimer: null,
      pollTimer: null,
      debounceTimer: null,
    };

    // Capture whatever's currently on screen first.
    tryEnqueue();

    // Observe the slider container instead of document.body. Douyin's
    // React tree mutates aggressively (animations, hover state, etc.)
    // and observing body wakes the MutationObserver hundreds of times
    // a second; on 50 concurrent browsers that adds up. The slider
    // subtree mutates only on actual slide rotations, which is what
    // we care about.
    //
    // Fallback: if slideList isn't mounted yet, observe body for now
    // and re-narrow on the first poll that finds a tighter root.
    // Observe only the data-e2e* markers Douyin flips when the active
    // slide rotates. Watching `class` was triggering on every animation
    // frame — Douyin's React tree toggles transition classes constantly
    // and the 600ms debounce was barely keeping up on weaker hardware.
    const OBSERVER_OPTS = {
      subtree: true,
      childList: true,
      attributes: true,
      attributeFilter: [
        "data-e2e",
        "data-e2e-aweme-id",
        "data-e2e-vid",
      ],
    };
    function findObserverRoot() {
      // slideList wraps every feed-item that swiper renders.
      const sl = document.querySelector('[data-e2e="slideList"]');
      if (sl) return sl;
      // Last-resort up-walk from the active video.
      const active = document.querySelector('[data-e2e="feed-active-video"]');
      if (active) {
        const fi = active.closest('[data-e2e="feed-item"]');
        if (fi && fi.parentElement) return fi.parentElement;
      }
      return null;
    }
    gatherState.observerRoot = findObserverRoot() || document.body;
    gatherState.observer = new MutationObserver(() => {
      // Debounce 600ms — slider transitions update several DOM points
      // (the active marker, video-desc text, searchbar placeholder) in
      // close succession, and capturing on the first mutation reads a
      // half-updated state with stale suggest_word. Wait for the page
      // to settle, then extract once.
      if (gatherState && gatherState.debounceTimer) {
        clearTimeout(gatherState.debounceTimer);
      }
      gatherState.debounceTimer = setTimeout(() => {
        if (gatherState) tryEnqueue();
      }, 600);
    });
    gatherState.observer.observe(gatherState.observerRoot, OBSERVER_OPTS);
    if (gatherState.observerRoot === document.body) {
      console.log("[kol-helper] observer attached to body (slideList not yet mounted)");
    } else {
      console.log("[kol-helper] observer attached to", gatherState.observerRoot.getAttribute("data-e2e") || "narrowed root");
    }

    // Belt-and-suspenders: a periodic poll catches any rotation the
    // MutationObserver misses (e.g. if the swiper re-renders subtree
    // without mutating the watched attributes). Same poll also:
    //   - re-checks login state (kicked-out detection).
    //   - auto-narrows the observer root to slideList once it mounts,
    //     if we initially fell back to document.body.
    //   - drains the retry queue so rows that bounced off the inflight
    //     cap get a second chance even during quiet stretches.
    gatherState.pollTimer = setInterval(() => {
      if (!gatherState) return;
      // Self-narrow observer to a tighter root once the slider mounts.
      if (gatherState.observerRoot === document.body) {
        const better = findObserverRoot();
        if (better && better !== gatherState.observerRoot) {
          gatherState.observer.disconnect();
          gatherState.observerRoot = better;
          gatherState.observer.observe(better, OBSERVER_OPTS);
          console.log(
            "[kol-helper] observer narrowed to",
            better.getAttribute("data-e2e") || "tightroot",
          );
        }
      }
      const cur = detectLoginState();
      if (cur !== "authenticated") {
        console.warn(
          "[kol-helper] gather auto-stop — login state flipped to",
          cur,
        );
        void reportLoginState();
        stopGather();
        return;
      }
      tryEnqueue();
      drainRetryQueue();
    }, POLL_MS);

    // Drive the slider forward independent of MutationObserver — Douyin
    // auto-plays, but autoplay-to-next isn't guaranteed for every video.
    gatherState.slideTimer = setInterval(() => {
      if (!gatherState) return;
      try {
        const how = dispatchNextSlide();
        console.log("[kol-helper] next via", how);
      } catch (e) {
        console.warn("[kol-helper] next failed", e);
      }
    }, SLIDE_INTERVAL_MS);

    updateButton();
  }

  function stopGather() {
    if (!gatherState) return;
    console.log(
      "[kol-helper] gather stop — captured",
      gatherState.captured,
      "uploaded",
      gatherState.uploaded,
    );
    setBlockingEnabled(false);
    // Tear down timers/observer first so no new rows enter the pipeline.
    if (gatherState.observer) gatherState.observer.disconnect();
    if (gatherState.slideTimer) clearInterval(gatherState.slideTimer);
    if (gatherState.pollTimer) clearInterval(gatherState.pollTimer);
    if (gatherState.debounceTimer) clearTimeout(gatherState.debounceTimer);
    // Snapshot anything still in the retry queue, then null out state.
    // Each pending row is uploaded individually — same single-row policy
    // as the steady-state path, just no more session counters to update.
    const pending = gatherState.retryQueue.splice(0);
    gatherState = null;
    updateButton();
    for (const row of pending) {
      kolFetchDirect("/kol-ext/gather/bulk", {
        method: "POST",
        body: JSON.stringify([row]),
      }).catch((e) =>
        console.warn("[kol-helper] final post failed:", e.message || e),
      );
    }
  }

  // ------------ UI: floating buttons --------------------------------------

  const DUMP_BTN_ID = "kol-dump-btn";
  const GATHER_BTN_ID = "kol-gather-btn";

  function makeBtn(id, bottom, bg) {
    const b = document.createElement("div");
    b.id = id;
    b.style.cssText = [
      "position:fixed",
      "right:20px",
      `bottom:${bottom}px`,
      "z-index:2147483647",
      `background:${bg}`,
      "color:#fff",
      "padding:10px 18px",
      "border-radius:24px",
      "cursor:pointer",
      "font-size:14px",
      "font-weight:600",
      "box-shadow:0 6px 18px rgba(0,0,0,0.35)",
      "font-family:-apple-system,BlinkMacSystemFont,'Helvetica Neue',sans-serif",
      "user-select:none",
      "transition:background 120ms,opacity 120ms",
    ].join(";");
    return b;
  }

  const dumpBtn = makeBtn(DUMP_BTN_ID, 80, "#a855f7");
  dumpBtn.textContent = "📥 抓取 DOM";
  dumpBtn.addEventListener("click", function () {
    dumpBtn.style.opacity = "0.7";
    dumpBtn.textContent = "抓取中...";
    try {
      const probe = collectProbe();
      const html = document.documentElement.outerHTML;
      const filename = "douyin-dump-" + Date.now() + ".json";
      downloadJson(filename, { probe, html, htmlBytes: html.length });
      dumpBtn.textContent = "✓ " + filename;
      dumpBtn.style.background = "#10b981";
      setTimeout(() => {
        dumpBtn.textContent = "📥 抓取 DOM";
        dumpBtn.style.background = "#a855f7";
        dumpBtn.style.opacity = "1";
      }, 3000);
    } catch (e) {
      dumpBtn.textContent = "✗ " + String(e).slice(0, 30);
      dumpBtn.style.background = "#ef4444";
      console.error(e);
      setTimeout(() => {
        dumpBtn.textContent = "📥 抓取 DOM";
        dumpBtn.style.background = "#a855f7";
        dumpBtn.style.opacity = "1";
      }, 4000);
    }
  });

  const gatherBtn = makeBtn(GATHER_BTN_ID, 20, "#0ea5e9");
  gatherBtn.textContent = "▶️ 开始采集";
  gatherBtn.addEventListener("click", function () {
    if (gatherState) {
      stopGather();
    } else {
      startGather();
    }
  });

  function updateButton() {
    if (gatherState) {
      gatherBtn.textContent =
        "⏸ 停止 · " +
        gatherState.captured +
        "↑" +
        gatherState.uploaded +
        (gatherState.errors ? "↯" + gatherState.errors : "");
      gatherBtn.style.background = "#ef4444";
    } else {
      gatherBtn.textContent = "▶️ 开始采集";
      gatherBtn.style.background = "#0ea5e9";
    }
  }

  document.documentElement.appendChild(dumpBtn);
  document.documentElement.appendChild(gatherBtn);
  console.log("[kol-helper] installed on", location.href);
})();
