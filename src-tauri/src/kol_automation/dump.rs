//! One-shot Douyin DOM dump command.
//!
//! Triggered by the user after they've already launched a profile,
//! logged in to Douyin, and scrolled to a useful state. Walks the local
//! profile list to find a *running* `kol_platform="douyin"` Wayfern
//! profile, opens a CDP connection, fetches:
//!
//! - `document.documentElement.outerHTML` — the literal markup, written
//!   verbatim to a `.html` file so the host can read it like any other
//!   web page (or open it in a browser).
//! - `dump_probe.js` — a structural digest written to a `.json` file
//!   with normalized candidate cards, distinct `data-e2e` values, and
//!   summary counts. Faster to skim than the raw HTML.
//!
//! Both files land in `app_dirs::data_dir()/kol-dumps/`. The command
//! returns the absolute paths so the UI can show a copy-to-clipboard
//! affordance.

use std::path::PathBuf;

use chrono::Local;
use serde::Serialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::browser_runner::BrowserRunner;
use crate::profile::types::BrowserProfile;
use crate::wayfern_manager::WayfernManager;

use super::cdp::{fetch_first_page_ws, Cdp};
use super::ingest::ProfileLoginState;

pub const DOUYIN_URL: &str = "https://www.douyin.com/follow";

const PROBE_JS: &str = include_str!("dump_probe.js");

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DumpResult {
  pub profile_id: Uuid,
  pub profile_name: String,
  pub page_url: Option<String>,
  pub html_path: String,
  pub probe_path: String,
  pub html_bytes: usize,
  pub candidate_count: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DouyinProfileInfo {
  pub profile: BrowserProfile,
  /// `true` iff WayfernManager has an active instance with a CDP port
  /// for this profile's data directory.
  pub running: bool,
  /// Latest Douyin login state ping reported by the in-browser content
  /// script. `None` means the extension hasn't reported yet (just-
  /// launched profile, or page hasn't finished loading).
  pub login_state: Option<ProfileLoginState>,
}

/// List local profiles tagged for Douyin scrape (kol_platform=douyin +
/// browser=wayfern), each annotated with whether the Wayfern process is
/// currently up. The dump panel uses this to render Launch vs Dump per
/// row without an N+1 round trip.
#[tauri::command]
pub async fn kol_list_douyin_profiles() -> Result<Vec<DouyinProfileInfo>, String> {
  let runner = BrowserRunner::instance();
  let profiles = runner
    .profile_manager
    .list_profiles()
    .map_err(|e| format!("list_profiles: {e}"))?;
  let profiles_dir = runner.profile_manager.get_profiles_dir();
  let wayfern = WayfernManager::instance();

  let mut out = Vec::new();
  for p in profiles {
    if p.browser != "wayfern" {
      continue;
    }
    if p.kol_platform.as_deref() != Some("douyin") {
      continue;
    }
    let pp = p.get_profile_data_path(&profiles_dir);
    let running = wayfern
      .get_cdp_port(&pp.to_string_lossy())
      .await
      .is_some();
    let login_state = super::ingest::get_login_state(p.id);
    out.push(DouyinProfileInfo {
      profile: p,
      running,
      login_state,
    });
  }
  Ok(out)
}

/// Dump the DOM of a specific running douyin Wayfern profile (or the
/// first one found if `profile_id` is None).
#[tauri::command]
pub async fn kol_dump_douyin_dom(profile_id: Option<Uuid>) -> Result<DumpResult, String> {
  let runner = BrowserRunner::instance();
  let profiles = runner
    .profile_manager
    .list_profiles()
    .map_err(|e| format!("list_profiles: {e}"))?;
  let profiles_dir = runner.profile_manager.get_profiles_dir();

  let mut chosen: Option<(BrowserProfile, u16)> = None;
  for p in profiles {
    if p.browser != "wayfern" {
      continue;
    }
    if p.kol_platform.as_deref() != Some("douyin") {
      continue;
    }
    if let Some(want) = profile_id {
      if p.id != want {
        continue;
      }
    }
    let pp = p.get_profile_data_path(&profiles_dir);
    let pp_str = pp.to_string_lossy().to_string();
    if let Some(port) = WayfernManager::instance().get_cdp_port(&pp_str).await {
      chosen = Some((p, port));
      break;
    }
  }
  let (profile, port) = chosen.ok_or_else(|| match profile_id {
    Some(id) => format!("profile {id} not running or not a douyin Wayfern profile"),
    None => "no running douyin Wayfern profile found — launch one first".to_string(),
  })?;

  let ws_url = fetch_first_page_ws(port).await?;
  let (cdp, _events) = Cdp::connect(&ws_url).await?;

  let html = match eval_string(
    &cdp,
    "document.documentElement.outerHTML",
  )
  .await
  {
    Ok(s) => s,
    Err(e) => {
      cdp.close().await;
      return Err(format!("eval outerHTML: {e}"));
    }
  };

  let probe_value = match eval_value(&cdp, PROBE_JS).await {
    Ok(v) => v,
    Err(e) => {
      cdp.close().await;
      return Err(format!("eval probe: {e}"));
    }
  };

  cdp.close().await;

  let dumps_dir = crate::app_dirs::data_dir().join("kol-dumps");
  std::fs::create_dir_all(&dumps_dir)
    .map_err(|e| format!("mkdir kol-dumps: {e}"))?;

  let ts = Local::now().format("%Y%m%dT%H%M%S");
  let html_path: PathBuf = dumps_dir.join(format!("dump-{}-{ts}.html", profile.id));
  let probe_path: PathBuf = dumps_dir.join(format!("dump-{}-{ts}.json", profile.id));

  std::fs::write(&html_path, &html).map_err(|e| format!("write html: {e}"))?;
  std::fs::write(
    &probe_path,
    serde_json::to_string_pretty(&probe_value).unwrap_or_default(),
  )
  .map_err(|e| format!("write probe: {e}"))?;

  let candidate_count = probe_value
    .get("candidates")
    .and_then(|v| v.as_array())
    .map(|a| a.len())
    .unwrap_or(0);
  let page_url = probe_value
    .get("url")
    .and_then(|v| v.as_str())
    .map(|s| s.to_string());

  log::info!(
    "kol_dump_douyin_dom: profile={} html={}B probe_candidates={} -> {}",
    profile.id,
    html.len(),
    candidate_count,
    html_path.display()
  );

  Ok(DumpResult {
    profile_id: profile.id,
    profile_name: profile.name,
    page_url,
    html_path: html_path.to_string_lossy().into_owned(),
    probe_path: probe_path.to_string_lossy().into_owned(),
    html_bytes: html.len(),
    candidate_count,
  })
}

async fn eval_string(cdp: &Cdp, expression: &str) -> Result<String, String> {
  let result = cdp
    .call(
      "Runtime.evaluate",
      json!({
        "expression": expression,
        "returnByValue": true,
        "awaitPromise": false,
      }),
    )
    .await?;
  if let Some(exc) = result.get("exceptionDetails") {
    return Err(format!("page exception: {exc}"));
  }
  result
    .get("result")
    .and_then(|r| r.get("value"))
    .and_then(|v| v.as_str())
    .map(|s| s.to_string())
    .ok_or_else(|| format!("non-string evaluate result: {result}"))
}

async fn eval_value(cdp: &Cdp, expression: &str) -> Result<Value, String> {
  let result = cdp
    .call(
      "Runtime.evaluate",
      json!({
        "expression": expression,
        "returnByValue": true,
        "awaitPromise": false,
      }),
    )
    .await?;
  if let Some(exc) = result.get("exceptionDetails") {
    return Err(format!("page exception: {exc}"));
  }
  result
    .get("result")
    .and_then(|r| r.get("value"))
    .cloned()
    .ok_or_else(|| format!("missing evaluate value: {result}"))
}
