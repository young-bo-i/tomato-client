// Donut KOL Helper — service worker.
//
// Mostly idle. Two responsibilities:
//
//   1. Receive `set-block-enabled` messages from content.js and toggle
//      the `video_block_ruleset` declarativeNetRequest ruleset. The
//      ruleset blocks Douyin video CDN + image CDN to save bandwidth
//      during automated batch collection. We default it to OFF (in
//      manifest) so manual browsing doesn't get visible-flicker from
//      blocked thumbnails; content.js flips it ON when gather starts
//      and OFF when gather stops.
//
//   2. Stay registered as MV3 SW for declarativeNetRequest API access
//      (the chrome.declarativeNetRequest.* surface is only callable
//      from the extension's SW context).
//
// Note: content.js otherwise bypasses this SW and fetches the Donut
// local axum server directly via `kolFetchDirect` because the Wayfern
// embedded chromium has been observed to keep the SW dormant in a way
// our wakeup events don't break out of. The set-block-enabled message
// IS subject to that risk — if the SW is dormant, the toggle silently
// no-ops. That's acceptable: the worst case is "rules stay off during
// gather" which means more bandwidth use, not broken collection.

const RULESET_ID = "video_block_ruleset";

chrome.runtime.onMessage.addListener((msg, _sender, sendResponse) => {
  if (msg && msg.kind === "set-block-enabled") {
    const enable = !!msg.enable;
    const opts = enable
      ? { enableRulesetIds: [RULESET_ID] }
      : { disableRulesetIds: [RULESET_ID] };
    chrome.declarativeNetRequest
      .updateEnabledRulesets(opts)
      .then(() => sendResponse({ ok: true, enabled: enable }))
      .catch((e) => sendResponse({ ok: false, error: String(e) }));
    return true; // keep the message channel open for async sendResponse
  }
  return false;
});
