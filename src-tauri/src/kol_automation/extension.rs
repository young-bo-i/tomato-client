//! Donut KOL helper extension materializer.
//!
//! Each `kol_platform=douyin` profile gets its OWN extension directory
//! at `<app-data>/kol-extension-{profile_uuid}/`. They share identical
//! `manifest.json` / `content.js` / `background.js`, but each carries a
//! distinct `profile.json` with the profile's UUID. The content script
//! reads that file via `chrome.runtime.getURL("profile.json")` so each
//! browser tab knows which profile is uploading rows.
//!
//! Why per-profile dirs: a single shared `--load-extension=` directory
//! has no way to bake a per-launch identifier into it (we'd need
//! cookies, native messaging, or query-string smuggling — all worse).
//! Disk overhead is ~6 KB per profile, trivial even at 50 profiles.
//!
//! All files are bundled into the binary via `include_str!` so updates
//! ship with the next Donut build. `write_if_changed` keeps the mtime
//! stable when the bundled version matches what's on disk, which avoids
//! Chromium's extension-reload heuristic firing every launch.

use std::path::{Path, PathBuf};

use uuid::Uuid;

const MANIFEST: &str = include_str!("extension/manifest.json");
const CONTENT_JS: &str = include_str!("extension/content.js");
const BACKGROUND_JS: &str = include_str!("extension/background.js");
const BLOCK_RULES: &str = include_str!("extension/block_rules.json");

/// Resolve the per-profile extension dir under app data. Stable across
/// launches so Chromium's extension cache stays warm.
fn extension_dir_for_profile(profile_id: &Uuid) -> PathBuf {
  crate::app_dirs::data_dir().join(format!("kol-extension-{profile_id}"))
}

/// Idempotently materialize the extension files for a single profile.
/// Writes the bundled manifest/content/background plus a per-profile
/// `profile.json` carrying the UUID. Returns the directory.
pub fn ensure_extension_dir_for_profile(profile_id: &Uuid) -> Result<PathBuf, String> {
  let dir = extension_dir_for_profile(profile_id);
  std::fs::create_dir_all(&dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
  write_if_changed(&dir.join("manifest.json"), MANIFEST)?;
  write_if_changed(&dir.join("content.js"), CONTENT_JS)?;
  write_if_changed(&dir.join("background.js"), BACKGROUND_JS)?;
  write_if_changed(&dir.join("block_rules.json"), BLOCK_RULES)?;
  let profile_json = format!(
    "{{\n  \"profile_id\": \"{profile_id}\"\n}}\n"
  );
  write_if_changed(&dir.join("profile.json"), &profile_json)?;
  Ok(dir)
}

fn write_if_changed(path: &Path, content: &str) -> Result<(), String> {
  if let Ok(existing) = std::fs::read_to_string(path) {
    if existing == content {
      return Ok(());
    }
  }
  std::fs::write(path, content).map_err(|e| format!("write {}: {e}", path.display()))
}
