// Donut KOL Helper — service worker (stub).
//
// Content scripts fetch the Donut local axum server directly via
// `kolFetchDirect` (content.js), bypassing this service worker entirely.
// The SW is kept in manifest.json to satisfy MV3 requirements for some
// Chromium builds, but it has no active message handlers.
