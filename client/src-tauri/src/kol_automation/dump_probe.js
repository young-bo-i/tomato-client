// Structured DOM probe — runs as a CDP `Runtime.evaluate` expression
// and must EVALUATE TO an object (the IIFE wrapping does that). Output
// gets serialized back to the host with `returnByValue: true`, so
// every leaf must be JSON-safe (strings, numbers, plain objects/arrays).
//
// Goal: collect just enough structural information for the host to write
// concrete CSS selectors against the live page without needing to read
// the full HTML. The full HTML is dumped separately, this is the
// "executive summary".

(function () {
  function attrs(node, max) {
    if (!node || !node.attributes) return [];
    return Array.from(node.attributes)
      .slice(0, max || 12)
      .map((a) => ({ name: a.name, value: String(a.value).slice(0, 240) }));
  }

  function dataAttrs(node) {
    if (!node || !node.dataset) return {};
    var out = {};
    for (var k in node.dataset) out[k] = String(node.dataset[k]).slice(0, 240);
    return out;
  }

  function summarize(el) {
    if (!el) return null;
    var cls = typeof el.className === "string" ? el.className : "";
    return {
      tag: el.tagName,
      cls: cls.slice(0, 200),
      attrs: attrs(el, 12),
      data: dataAttrs(el),
      text: (el.innerText || "").trim().slice(0, 240),
    };
  }

  var result = {
    url: location.href,
    title: document.title,
    timestamp: Date.now(),
    counts: {
      videos: document.querySelectorAll("video").length,
      images: document.querySelectorAll("img").length,
      anchors: document.querySelectorAll("a[href]").length,
      lis: document.querySelectorAll("li").length,
      articles: document.querySelectorAll("article").length,
      dataE2e: document.querySelectorAll("[data-e2e]").length,
      dataId: document.querySelectorAll("[data-id]").length,
      dataKey: document.querySelectorAll("[data-key]").length,
    },
    distinctDataE2e: [],
    candidates: [],
  };

  // Roll up unique data-e2e values — those are usually stable hooks on
  // Douyin's React tree, the most reliable selector source.
  var dataE2eSeen = {};
  Array.from(document.querySelectorAll("[data-e2e]")).forEach(function (el) {
    var v = el.getAttribute("data-e2e");
    if (!v) return;
    dataE2eSeen[v] = (dataE2eSeen[v] || 0) + 1;
  });
  result.distinctDataE2e = Object.keys(dataE2eSeen)
    .sort()
    .map(function (k) {
      return { value: k, count: dataE2eSeen[k] };
    });

  // Collect candidate "video card" elements. Heuristic: must contain a
  // <video> or <img>, must have either an <a> link or non-trivial text.
  // We keep a sample (max 8) covering distinct shapes via class signature.
  var seenSig = {};
  var probe = document.querySelectorAll(
    "li, article, [data-e2e], [data-id], [data-key]"
  );
  for (var i = 0; i < probe.length && result.candidates.length < 8; i++) {
    var el = probe[i];
    if (!el.querySelector("video, img")) continue;
    var hasLink = !!el.querySelector("a[href]");
    var text = (el.innerText || "").trim();
    if (!hasLink && text.length < 5) continue;

    var cls = typeof el.className === "string" ? el.className : "";
    var sig = el.tagName + "|" + cls.slice(0, 60);
    if (seenSig[sig]) continue;
    seenSig[sig] = 1;

    var anchor = el.querySelector("a[href]");
    var video = el.querySelector("video");
    var img = el.querySelector("img");
    var poster = video && video.getAttribute("poster");
    var imgSrc =
      (img && (img.getAttribute("src") || img.getAttribute("data-src"))) ||
      null;

    result.candidates.push({
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
            src: imgSrc,
            attrs: attrs(img, 10),
            data: dataAttrs(img),
          }
        : null,
      video: video
        ? {
            src: video.getAttribute("src"),
            poster: poster,
            attrs: attrs(video, 10),
            data: dataAttrs(video),
          }
        : null,
      // Truncated outerHTML so we can eyeball the literal markup.
      outerHTMLPreview: (el.outerHTML || "").slice(0, 1200),
    });
  }

  return result;
})();
