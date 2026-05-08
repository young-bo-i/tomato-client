//! Batch orchestration: rolling Worker Pool + auto-loop + 4h full-restart
//! supervisor.
//!
//! Two layers of orchestration:
//!
//! ## Inner: rolling Worker Pool (within one session)
//!
//! Up to ~50 douyin profiles can't all run at once (50 × ~700 MB ≈ 35 GB).
//! We keep `POOL_SIZE` profiles active and rotate the rest through.
//!
//! Slot lifecycle (one worker task per slot):
//!   1. take next profile from queue (or wait for refiller if empty)
//!   2. flip its `should_gather` flag → true
//!   3. launch_browser_profile (douyin/follow URL)
//!   4. wait until either MAX_PROFILE_DURATION cap or batch cancel
//!   5. flip `should_gather` → false (extension flushes buffer)
//!   6. wait STOP_FLUSH_GRACE_PROFILE
//!   7. kill_browser_profile
//!   8. random INTER_PROFILE_SLEEP_MIN..MAX cooldown (anti-pattern)
//!   9. goto 1
//!
//! When the queue drains AND every slot is idle, ONE worker becomes the
//! refiller (single-flight via AtomicBool), waits a short ROUND_END_GAP,
//! reloads the douyin profile list, refills the queue, increments the
//! round counter, and notifies the others. Workers that lost the race
//! wait on `refill_notify` so they don't busy-loop.
//!
//! ## Outer: 4h full-restart supervisor
//!
//! A separate task created by `kol_batch_start` watches session age. At
//! FULL_RESTART_INTERVAL (4h, hardcoded) it cancels the current Pool —
//! which kills all active browsers via the existing teardown path —
//! clears in-process state (SHOULD_GATHER, UNAUTH_SINCE), and builds a
//! fresh Pool. Dedup cache is intentionally NOT cleared (its 24h TTL
//! is part of the design — clearing would re-upload videos).
//!
//! `kol_batch_stop` cancels the supervisor; the supervisor cancels the
//! Pool; workers and browsers tear down.
//!
//! The Donut local axum endpoint `/kol-ext/gather/should` is still the
//! source of truth that content.js polls; the worker just flips the
//! flag at the right moments.
//!
//! ## Event log
//!
//! Every state transition (session/round boundaries, profile start/end,
//! full-restart, errors) is recorded into a 500-entry ring buffer
//! exposed via `kol_batch_events`. The panel renders this as a
//! monitorable timeline so the operator sees what's happening without
//! tailing logs.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::time::{Duration, Instant};

use chrono::{DateTime, Local};
use once_cell::sync::Lazy;
use serde::Serialize;
use tokio::sync::{Notify, Semaphore};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::browser_runner::BrowserRunner;
use crate::profile::types::BrowserProfile;
use crate::wayfern_manager::WayfernManager;

// ---- tunables ----------------------------------------------------------

/// Concurrent browsers in the pool. With each Wayfern profile costing
/// ~700 MB, 10 caps RAM at ~7 GB which fits a typical Win iGPU box.
const POOL_SIZE: usize = 10;

/// How many launches we allow to run concurrently. The pool holds 10
/// active profiles, but at startup all 10 workers grab their first
/// profile at the same instant — without this throttle, the host
/// briefly goes 10×Chromium-startup, which is CPU/IO spike that can
/// stall fingerprint negotiation. Once a profile is past the launch,
/// it doesn't hold this semaphore (only the actual `launch_browser_profile`
/// call is gated).
const LAUNCH_CONCURRENCY: usize = 3;

/// Hard ceiling on time spent on a single profile per round. With the
/// content.js MAX_VIDEOS=200 cap and 3s slide rate, a profile naturally
/// finishes in 10 minutes; this is the safety net for cases where
/// dedupe slows progress.
const MAX_PROFILE_DURATION: Duration = Duration::from_secs(10 * 60);

/// Time between flipping `should_gather=false` and killing the browser.
/// Lets content.js flush whatever's still in its in-memory buffer
/// (4s flush interval inside content.js).
const STOP_FLUSH_GRACE_PROFILE: Duration = Duration::from_secs(4);

/// Random pause between finishing one profile and picking up the next.
/// Real users don't have profiles popping in at synchronized intervals;
/// this jitter makes per-IP traffic patterns less obviously automated.
const INTER_PROFILE_SLEEP_MIN: Duration = Duration::from_secs(30);
const INTER_PROFILE_SLEEP_MAX: Duration = Duration::from_secs(90);

/// Pause between a round draining (queue + active both empty) and the
/// next round's refill. Kept short — the per-profile 30~90s cooldowns
/// already provide anti-pattern spacing, no need to add minutes here.
const ROUND_END_GAP_MIN: Duration = Duration::from_secs(5);
const ROUND_END_GAP_MAX: Duration = Duration::from_secs(20);

/// Hardcoded full-restart cadence. After a session has run this long,
/// the supervisor tears the Pool down (kills every browser, clears
/// in-process state) and builds a fresh one. Goal: flush whatever
/// junk has accumulated in long-running Chromium processes.
const FULL_RESTART_INTERVAL: Duration = Duration::from_secs(4 * 60 * 60);

/// Brief gap between a full-restart's teardown and the next session's
/// build. Lets the OS reclaim browser process resources before we
/// start spawning the next 10.
const POST_FULL_RESTART_GAP: Duration = Duration::from_secs(15);

/// Supervisor's poll cadence for "is it time for the 4h restart yet".
const SUPERVISOR_TICK: Duration = Duration::from_secs(60);

/// Stop-everything grace from the legacy global stop path.
const STOP_FLUSH_GRACE_BATCH: Duration = Duration::from_secs(5);

/// In-memory event log size. Bounded ring buffer — older events are
/// dropped as new ones arrive. Sized so a busy 4h session (50 profiles
/// × ~5 rounds × ~3 events/profile + session/round events) fits with
/// headroom.
const EVENT_LOG_CAP: usize = 500;

/// First moment we noticed `state=unauthenticated` for a profile that
/// the batch wants to be gathering. Cleared whenever the extension
/// reports a non-unauth state, so the timer only fires for sustained
/// unauthenticated runs (kicked out, or first-launch never logged in).
static UNAUTH_SINCE: Lazy<StdMutex<HashMap<Uuid, Instant>>> =
  Lazy::new(|| StdMutex::new(HashMap::new()));

/// Grace window before auto-closing a sustained-unauth browser. Tuned
/// to be longer than a typical scan-QR-and-confirm flow (~30s) so that
/// users actively logging in aren't booted mid-process.
const UNAUTH_AUTO_CLOSE_GRACE: Duration = Duration::from_secs(90);

const DOUYIN_URL: &str = "https://www.douyin.com/follow";

// ---- event log ---------------------------------------------------------
//
// Structured timeline of what the batch supervisor / workers are doing.
// Surfaced to the UI via `kol_batch_events` for the monitoring panel.

/// One entry in the timeline. `kind` is the discrete state-transition
/// the operator cares about; the optional fields scope it to a profile
/// or round when applicable.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchEvent {
  /// Auto-incrementing id so the UI can dedupe / track latest seen.
  pub id: u64,
  pub at: DateTime<Local>,
  pub kind: BatchEventKind,
  /// Round the event belongs to (None for session-level events that
  /// don't map cleanly to a single round).
  #[serde(skip_serializing_if = "Option::is_none")]
  pub round: Option<usize>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub profile_id: Option<Uuid>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub profile_name: Option<String>,
  /// Free-form context (cap reason, error text, count, etc.).
  #[serde(skip_serializing_if = "Option::is_none")]
  pub detail: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BatchEventKind {
  /// Supervisor started a new session (initial start OR after a 4h
  /// restart). The session is the unit that gets torn down at 4h.
  SessionStart,
  /// Supervisor exited (kol_batch_stop or terminal error). Distinct
  /// from FullRestart (which immediately starts a new session).
  SessionStop,
  /// A round began — queue refilled with N profiles, workers picking up.
  RoundStart,
  /// All N profiles in this round finished. Refiller will start the
  /// next round shortly.
  RoundComplete,
  /// 4h elapsed; supervisor is tearing down the current Pool.
  FullRestartTriggered,
  /// Teardown done; a fresh Pool has been built and is starting.
  FullRestartComplete,
  /// Worker picked a profile, launching browser + flipping should_gather.
  ProfileStart,
  /// Profile slot finished (cap hit, cancel, or kill). `detail` carries
  /// the reason ("cap" / "cancel" / "unauth-auto-close").
  ProfileEnd,
  /// Profile failed during launch/setup. `detail` carries the error.
  ProfileError,
}

static EVENT_NEXT_ID: AtomicU64 = AtomicU64::new(1);
static EVENTS: Lazy<StdMutex<VecDeque<BatchEvent>>> =
  Lazy::new(|| StdMutex::new(VecDeque::with_capacity(EVENT_LOG_CAP)));

/// Builder used by every emit-event call site so we don't sprinkle
/// `BatchEvent { ..None, ..None, ..None }` everywhere.
struct EventBuilder {
  kind: BatchEventKind,
  round: Option<usize>,
  profile_id: Option<Uuid>,
  profile_name: Option<String>,
  detail: Option<String>,
}

impl EventBuilder {
  fn new(kind: BatchEventKind) -> Self {
    Self { kind, round: None, profile_id: None, profile_name: None, detail: None }
  }
  fn round(mut self, r: usize) -> Self { self.round = Some(r); self }
  fn profile(mut self, p: &BrowserProfile) -> Self {
    self.profile_id = Some(p.id);
    self.profile_name = Some(p.name.clone());
    self
  }
  fn detail(mut self, d: impl Into<String>) -> Self { self.detail = Some(d.into()); self }
  fn emit(self) {
    let id = EVENT_NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let ev = BatchEvent {
      id,
      at: Local::now(),
      kind: self.kind,
      round: self.round,
      profile_id: self.profile_id,
      profile_name: self.profile_name,
      detail: self.detail,
    };
    if let Ok(mut q) = EVENTS.lock() {
      if q.len() >= EVENT_LOG_CAP {
        q.pop_front();
      }
      q.push_back(ev);
    }
  }
}

/// Reads the current snapshot of the ring buffer, newest-first.
/// Returned to the UI verbatim; if the panel wants to filter by kind
/// or paginate it does so client-side (cheap with ≤500 entries).
fn events_snapshot() -> Vec<BatchEvent> {
  let q = match EVENTS.lock() {
    Ok(q) => q,
    Err(_) => return vec![],
  };
  let mut out: Vec<BatchEvent> = q.iter().cloned().collect();
  out.reverse();
  out
}

// ---- shared state ------------------------------------------------------

pub static SHOULD_GATHER: Lazy<StdMutex<HashMap<Uuid, bool>>> =
  Lazy::new(|| StdMutex::new(HashMap::new()));

/// The currently-running pool, if any. `None` between batches.
static POOL: Lazy<StdMutex<Option<Arc<Pool>>>> = Lazy::new(|| StdMutex::new(None));

/// The supervisor's cancellation token, if a batch is active. The
/// supervisor outlives any individual Pool (it's what builds Pools at
/// 4h boundaries). `kol_batch_stop` cancels this; the supervisor
/// cancels the Pool inside it.
static SUPERVISOR_CANCEL: Lazy<StdMutex<Option<CancellationToken>>> =
  Lazy::new(|| StdMutex::new(None));

struct PoolInner {
  /// Profiles waiting to be picked up THIS round.
  queue: VecDeque<BrowserProfile>,
  /// Profiles currently held by a worker slot.
  active: HashMap<Uuid, Instant>,
  /// Profiles finished this round. Cleared on refill.
  completed_in_round: HashSet<Uuid>,
  /// Total profiles in the round (= initial queue.len()). Cleared on
  /// refill.
  total_in_round: usize,
  /// 1-based round counter. Increments when the queue is refilled.
  round: usize,
  /// Wall-clock when the **session** started — surfaced to UI for
  /// "已运行 HH:mm:ss". Persists across rounds; reset only on a 4h
  /// full-restart (which builds a fresh Pool).
  session_started_at: DateTime<Local>,
  /// Wall-clock when the current round started. Refreshed on refill.
  round_started_at: DateTime<Local>,
}

struct Pool {
  inner: StdMutex<PoolInner>,
  /// Cancels every worker + the per-Pool supervisor cohort. The outer
  /// `SUPERVISOR_CANCEL` (which can outlive a Pool) is separate.
  cancel: CancellationToken,
  /// Throttles the cost-spike of `launch_browser_profile` so workers
  /// can't all spawn Chromium simultaneously (initial burst kills
  /// CPU + can race fingerprint injection).
  launch_gate: Arc<Semaphore>,
  /// Single-flight gate for the round-end refill. Whichever worker
  /// flips this from `false → true` becomes the refiller; the others
  /// wait on `refill_notify`. Reset to `false` after the refill is
  /// done.
  refilling: AtomicBool,
  /// Notify-all signal that the queue has been refilled (or the Pool
  /// is being cancelled). All non-refiller workers `notified().await`
  /// on this when they hit an empty queue.
  refill_notify: Notify,
  /// `Instant` form of `session_started_at`, used by the supervisor
  /// to test the 4h budget without re-doing chrono arithmetic each
  /// tick.
  session_started_inst: Instant,
}

fn current_pool() -> Option<Arc<Pool>> {
  POOL.lock().ok().and_then(|g| g.clone())
}

// ---- public types ------------------------------------------------------

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BatchStatus {
  /// "idle" when no supervisor is running, "running" otherwise.
  /// (Auto-loop means there's no longer a meaningful "drained" state
  /// from the user's perspective — the supervisor will refill.)
  pub state: String,
  /// Profiles in the **current round** (queue + active + completed).
  pub total_profiles: usize,
  /// Profiles waiting in the round's queue.
  pub queued: usize,
  /// Profiles currently held by a slot.
  pub active: usize,
  /// Profiles finished THIS round (cap, cancel, or kill).
  pub completed_in_round: usize,
  /// Actually-running browser processes (regardless of pool ownership).
  pub running_browsers: usize,
  /// Profiles where the gather flag is currently true (subset of active).
  pub active_gathers: usize,
  /// Wall-clock when the current session started. Reset on every
  /// 4h full-restart (which builds a fresh Pool); persists across
  /// auto-loop round boundaries within the session.
  pub session_started_at: Option<DateTime<Local>>,
  /// Wall-clock when the current round started.
  pub round_started_at: Option<DateTime<Local>>,
  /// 1-based round counter. Increments each time the queue is
  /// refilled. Auto-loop is unbounded — there's no `total_rounds`.
  pub current_round: Option<usize>,
  /// Wall-clock for the next planned 4h full-restart. UI shows this
  /// as a countdown so the operator can predict when browsers will
  /// be killed for cache flushing.
  pub next_full_restart_at: Option<DateTime<Local>>,
}

// ---- flag access -------------------------------------------------------

pub fn read_should_gather(profile_id: Uuid) -> bool {
  SHOULD_GATHER
    .lock()
    .ok()
    .and_then(|m| m.get(&profile_id).copied())
    .unwrap_or(false)
}

fn set_should_gather(profile_id: Uuid, value: bool) {
  if let Ok(mut m) = SHOULD_GATHER.lock() {
    m.insert(profile_id, value);
  }
}

pub fn clear_unauth_marker(profile_id: Uuid) {
  if let Ok(mut m) = UNAUTH_SINCE.lock() {
    m.remove(&profile_id);
  }
}

pub fn note_unauth_state(profile_id: Uuid) -> bool {
  if !read_should_gather(profile_id) {
    if let Ok(mut m) = UNAUTH_SINCE.lock() {
      m.remove(&profile_id);
    }
    return false;
  }
  let now = Instant::now();
  let mut map = match UNAUTH_SINCE.lock() {
    Ok(m) => m,
    Err(_) => return false,
  };
  let first = *map.entry(profile_id).or_insert(now);
  let aged_out = now.duration_since(first) >= UNAUTH_AUTO_CLOSE_GRACE;
  if aged_out {
    map.remove(&profile_id);
  }
  aged_out
}

pub async fn start_unauth_watchdog(app: tauri::AppHandle) {
  let mut tick = tokio::time::interval(Duration::from_secs(15));
  tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
  loop {
    tick.tick().await;
    let now = Instant::now();
    let to_kill: Vec<Uuid> = {
      let mut map = match UNAUTH_SINCE.lock() {
        Ok(m) => m,
        Err(_) => continue,
      };
      let expired: Vec<Uuid> = map
        .iter()
        .filter(|(id, t)| {
          now.duration_since(**t) >= UNAUTH_AUTO_CLOSE_GRACE
            && read_should_gather(**id)
        })
        .map(|(id, _)| *id)
        .collect();
      for id in &expired {
        map.remove(id);
      }
      expired
    };
    if to_kill.is_empty() {
      continue;
    }
    log::info!(
      "kol_batch unauth watchdog: closing {} profile(s)",
      to_kill.len()
    );
    for id in to_kill {
      let app = app.clone();
      tokio::spawn(async move { kill_unauth_profile(app, id).await });
    }
  }
}

pub async fn kill_unauth_profile(app: tauri::AppHandle, profile_id: Uuid) {
  set_should_gather(profile_id, false);
  let runner = BrowserRunner::instance();
  let profiles = match runner.profile_manager.list_profiles() {
    Ok(p) => p,
    Err(e) => {
      log::warn!("kol_batch unauth kill list_profiles: {e}");
      return;
    }
  };
  let Some(profile) = profiles.into_iter().find(|p| p.id == profile_id) else {
    log::warn!("kol_batch unauth kill: profile {profile_id} not found");
    return;
  };
  log::warn!(
    "kol_batch: profile {} ({}) sustained unauth for {}s — auto-closing",
    profile.name,
    profile.id,
    UNAUTH_AUTO_CLOSE_GRACE.as_secs(),
  );
  if let Err(e) =
    crate::browser_runner::kill_browser_profile(app, profile.clone()).await
  {
    log::warn!("kol_batch unauth kill failed for {profile_id}: {e}");
  }
}

// ---- profile resolution -----------------------------------------------

/// Cache for list_douyin_profiles. The underlying `profile_manager.list_profiles()`
/// is **not** a local disk read — it round-trips to the remote
/// tomato-server (`KOL_CLIENT.list_profiles_blocking()` → HTTP
/// `GET /api/profiles`). The status panel polls every 3-5s and
/// snapshot_status() is its primary consumer; without this cache that
/// poll alone burns ~80 KB/s of background traffic.
///
/// 5s TTL aligns with UI poll interval — same observable freshness,
/// roughly 1/5 the HTTP load.
static PROFILE_LIST_CACHE: Lazy<StdMutex<Option<(Instant, Vec<BrowserProfile>)>>> =
  Lazy::new(|| StdMutex::new(None));

const PROFILE_LIST_TTL: Duration = Duration::from_secs(5);

fn fetch_douyin_profiles_uncached() -> Result<Vec<BrowserProfile>, String> {
  let runner = BrowserRunner::instance();
  let all = runner
    .profile_manager
    .list_profiles()
    .map_err(|e| format!("list_profiles: {e}"))?;
  Ok(
    all
      .into_iter()
      .filter(|p| {
        p.browser == "wayfern" && p.kol_platform.as_deref() == Some("douyin")
      })
      .collect(),
  )
}

fn list_douyin_profiles() -> Result<Vec<BrowserProfile>, String> {
  // Fast path: cache hit within TTL.
  if let Ok(g) = PROFILE_LIST_CACHE.lock() {
    if let Some((t, v)) = g.as_ref() {
      if t.elapsed() < PROFILE_LIST_TTL {
        return Ok(v.clone());
      }
    }
  }
  // Miss / expired: fetch and store.
  let fresh = fetch_douyin_profiles_uncached()?;
  if let Ok(mut g) = PROFILE_LIST_CACHE.lock() {
    *g = Some((Instant::now(), fresh.clone()));
  }
  Ok(fresh)
}

/// Force-invalidate the profile list cache. Called on lifecycle events
/// (batch_start/stop) where the next read needs to be authoritative.
fn invalidate_profile_cache() {
  if let Ok(mut g) = PROFILE_LIST_CACHE.lock() {
    *g = None;
  }
}

async fn count_running_browsers(profiles: &[BrowserProfile]) -> usize {
  let runner = BrowserRunner::instance();
  let profiles_dir = runner.profile_manager.get_profiles_dir();
  let wayfern = WayfernManager::instance();
  // Parallel port checks — with 50 profiles, serial awaits add up.
  let checks: Vec<_> = profiles
    .iter()
    .map(|p| {
      let pp = p.get_profile_data_path(&profiles_dir);
      let pp_str = pp.to_string_lossy().to_string();
      async move { wayfern.get_cdp_port(&pp_str).await.is_some() }
    })
    .collect();
  futures_util::future::join_all(checks)
    .await
    .into_iter()
    .filter(|&up| up)
    .count()
}

fn count_active_gathers(profiles: &[BrowserProfile]) -> usize {
  let map = match SHOULD_GATHER.lock() {
    Ok(m) => m,
    Err(_) => return 0,
  };
  profiles
    .iter()
    .filter(|p| map.get(&p.id).copied().unwrap_or(false))
    .count()
}

// ---- status snapshot ---------------------------------------------------

pub async fn snapshot_status() -> Result<BatchStatus, String> {
  if let Some(pool) = current_pool() {
    let (
      queued,
      active,
      completed_in_round,
      total_in_round,
      session_started_at,
      round_started_at,
      round,
      session_started_inst,
    ) = {
      let inner = pool
        .inner
        .lock()
        .map_err(|_| "pool inner poisoned")?;
      (
        inner.queue.len(),
        inner.active.len(),
        inner.completed_in_round.len(),
        inner.total_in_round,
        inner.session_started_at,
        inner.round_started_at,
        inner.round,
        pool.session_started_inst,
      )
    };

    let profiles = list_douyin_profiles().unwrap_or_default();
    let running_browsers = count_running_browsers(&profiles).await;
    let active_gathers = count_active_gathers(&profiles);

    // Auto-loop means "running" as long as the supervisor is alive —
    // round boundaries (queue empty + active empty for a few seconds)
    // are still part of the session and shouldn't flash the UI to
    // idle. Supervisor presence is reflected in current_pool().is_some.
    let state = "running".to_string();

    // Compute "next full restart" as session_start + 4h, projected
    // onto wall-clock by applying the same elapsed-since-session-start
    // delta to session_started_at.
    let elapsed = session_started_inst.elapsed();
    let next_full_restart_at = if elapsed < FULL_RESTART_INTERVAL {
      let remaining = FULL_RESTART_INTERVAL - elapsed;
      let chrono_remaining = chrono::Duration::from_std(remaining).ok();
      chrono_remaining.map(|d| Local::now() + d)
    } else {
      // Already past the 4h budget — supervisor is about to (or already
      // is) tearing down. Show "now-ish" so countdown reads 00:00.
      Some(Local::now())
    };

    Ok(BatchStatus {
      state,
      total_profiles: total_in_round,
      queued,
      active,
      completed_in_round,
      running_browsers,
      active_gathers,
      session_started_at: Some(session_started_at),
      round_started_at: Some(round_started_at),
      current_round: Some(round),
      next_full_restart_at,
    })
  } else {
    let profiles = list_douyin_profiles()?;
    let running_browsers = count_running_browsers(&profiles).await;
    let active_gathers = count_active_gathers(&profiles);
    Ok(BatchStatus {
      state: "idle".to_string(),
      total_profiles: profiles.len(),
      queued: 0,
      active: 0,
      completed_in_round: 0,
      running_browsers,
      active_gathers,
      session_started_at: None,
      round_started_at: None,
      current_round: None,
      next_full_restart_at: None,
    })
  }
}

// ---- worker loop -------------------------------------------------------

async fn worker_loop(slot: usize, app: tauri::AppHandle, pool: Arc<Pool>) {
  loop {
    if pool.cancel.is_cancelled() {
      break;
    }

    let profile = match take_or_wait(slot, &pool).await {
      Some(p) => p,
      None => break, // cancelled while waiting for a refill
    };

    log::info!(
      "[pool slot {}] picking up {} ({})",
      slot,
      profile.name,
      profile.id
    );

    run_one_profile(slot, &app, &profile, &pool).await;

    // Move from active -> completed_in_round.
    {
      if let Ok(mut inner) = pool.inner.lock() {
        inner.active.remove(&profile.id);
        inner.completed_in_round.insert(profile.id);
      }
    }

    // Inter-profile cooldown — randomized to avoid lockstep traffic.
    let sleep = random_inter_profile_sleep();
    log::info!(
      "[pool slot {}] cooldown {}s before next profile",
      slot,
      sleep.as_secs()
    );
    tokio::select! {
      _ = pool.cancel.cancelled() => break,
      _ = tokio::time::sleep(sleep) => {}
    }
  }
  log::info!("[pool slot {}] worker exit", slot);
}

/// Pop the next profile, blocking until one is available. When the
/// queue is empty AND every slot is idle (= round drained), one of the
/// waiting workers becomes the refiller; the rest park on
/// `pool.refill_notify`.
///
/// Returns `None` when the Pool is cancelled while waiting.
async fn take_or_wait(slot: usize, pool: &Arc<Pool>) -> Option<BrowserProfile> {
  loop {
    if pool.cancel.is_cancelled() {
      return None;
    }

    // 1. Try the fast path — there's still work in the current round.
    let popped = {
      let mut inner = pool.inner.lock().ok()?;
      inner.queue.pop_front().map(|p| {
        inner.active.insert(p.id, Instant::now());
        p
      })
    };
    if let Some(p) = popped {
      return Some(p);
    }

    // 2. Queue empty. Decide if this slot is the LAST one to finish:
    //    if active is also empty, the round is fully drained and one
    //    slot needs to refill.
    let round_done = {
      let inner = pool.inner.lock().ok()?;
      inner.queue.is_empty() && inner.active.is_empty()
    };

    if round_done
      && pool
        .refilling
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
    {
      // We won the refiller race. Do the refill.
      let cur_round = pool.inner.lock().ok().map(|i| i.round).unwrap_or(0);
      EventBuilder::new(BatchEventKind::RoundComplete)
        .round(cur_round)
        .detail("queue + active drained")
        .emit();

      // Brief gap before the next round so per-IP traffic doesn't
      // exactly mirror the previous round's start. Per-profile 30~90s
      // cooldowns already provide most of the spacing — this just adds
      // a tiny round-boundary jitter.
      let gap = random_round_end_gap();
      log::info!(
        "[pool slot {}] round-end gap {}s before refill",
        slot,
        gap.as_secs()
      );
      let cancelled = tokio::select! {
        _ = pool.cancel.cancelled() => true,
        _ = tokio::time::sleep(gap) => false,
      };
      if cancelled {
        pool.refilling.store(false, Ordering::Release);
        pool.refill_notify.notify_waiters();
        return None;
      }

      // Reload the douyin profile list — picks up newly-created /
      // newly-deleted profiles between rounds without restarting the
      // whole batch.
      let profiles = match list_douyin_profiles() {
        Ok(p) => p,
        Err(e) => {
          log::warn!("[pool slot {}] refill list_douyin_profiles: {e}", slot);
          // Don't get stuck — release the refill flag, wake everyone,
          // and retry next iteration.
          pool.refilling.store(false, Ordering::Release);
          pool.refill_notify.notify_waiters();
          tokio::time::sleep(Duration::from_secs(5)).await;
          continue;
        }
      };
      if profiles.is_empty() {
        log::warn!(
          "[pool slot {}] refill: no douyin profiles available, idling 30s",
          slot
        );
        pool.refilling.store(false, Ordering::Release);
        pool.refill_notify.notify_waiters();
        tokio::select! {
          _ = pool.cancel.cancelled() => return None,
          _ = tokio::time::sleep(Duration::from_secs(30)) => continue,
        }
      }

      let next_round = cur_round + 1;
      let total = profiles.len();
      refill_pool_queue(pool, profiles, next_round);

      EventBuilder::new(BatchEventKind::RoundStart)
        .round(next_round)
        .detail(format!("{total} profile(s) queued"))
        .emit();
      log::info!(
        "[pool slot {}] refilled queue: round {} with {total} profile(s)",
        slot,
        next_round
      );

      pool.refilling.store(false, Ordering::Release);
      pool.refill_notify.notify_waiters();
      // Loop back: this slot pops the first profile of the new round.
      continue;
    }

    // 3. Either someone else is the refiller, or we're not the last
    //    finishing slot — wait for the refill notification.
    tokio::select! {
      _ = pool.cancel.cancelled() => return None,
      _ = pool.refill_notify.notified() => {},
    }
  }
}

/// Replace the queue + reset round-scoped state in one critical section.
fn refill_pool_queue(pool: &Pool, profiles: Vec<BrowserProfile>, round: usize) {
  if let Ok(mut inner) = pool.inner.lock() {
    inner.queue = profiles.into_iter().collect();
    inner.active.clear();
    inner.completed_in_round.clear();
    inner.total_in_round = inner.queue.len();
    inner.round = round;
    inner.round_started_at = Local::now();
  }
}

async fn run_one_profile(
  slot: usize,
  app: &tauri::AppHandle,
  profile: &BrowserProfile,
  pool: &Pool,
) {
  let round = pool.inner.lock().ok().map(|i| i.round).unwrap_or(0);

  set_should_gather(profile.id, true);
  clear_unauth_marker(profile.id);

  EventBuilder::new(BatchEventKind::ProfileStart)
    .round(round)
    .profile(profile)
    .detail(format!("slot {slot}"))
    .emit();

  // Throttle simultaneous launches. Holding the permit just for the
  // duration of `launch_browser_profile` smooths the startup CPU
  // spike when 10 workers grab profiles in the same millisecond.
  // Once `launch_browser_profile` returns (CDP ready), we release —
  // the pool's normal POOL_SIZE concurrency takes over for the
  // steady-state gather phase.
  let permit = pool.launch_gate.clone().acquire_owned().await;
  if let Err(e) = crate::browser_runner::launch_browser_profile(
    app.clone(),
    profile.clone(),
    Some(DOUYIN_URL.into()),
  )
  .await
  {
    drop(permit);
    log::error!("[pool slot {}] launch failed for {}: {e}", slot, profile.id);
    set_should_gather(profile.id, false);
    EventBuilder::new(BatchEventKind::ProfileError)
      .round(round)
      .profile(profile)
      .detail(format!("launch: {e}"))
      .emit();
    return;
  }
  drop(permit);

  // Wait for either the per-profile time cap OR a global cancel.
  let end_reason = tokio::select! {
    _ = pool.cancel.cancelled() => {
      log::info!("[pool slot {}] cancelled mid-profile", slot);
      "cancel"
    }
    _ = tokio::time::sleep(MAX_PROFILE_DURATION) => {
      log::info!(
        "[pool slot {}] {} hit {}min cap, rotating",
        slot,
        profile.id,
        MAX_PROFILE_DURATION.as_secs() / 60
      );
      "cap"
    }
  };

  // Stop signal first so the extension flushes its buffer.
  set_should_gather(profile.id, false);
  tokio::time::sleep(STOP_FLUSH_GRACE_PROFILE).await;
  if let Err(e) =
    crate::browser_runner::kill_browser_profile(app.clone(), profile.clone()).await
  {
    log::warn!("[pool slot {}] kill failed for {}: {e}", slot, profile.id);
  }

  EventBuilder::new(BatchEventKind::ProfileEnd)
    .round(round)
    .profile(profile)
    .detail(end_reason)
    .emit();
}

fn random_inter_profile_sleep() -> Duration {
  use rand::RngExt;
  let secs = rand::rng().random_range(
    INTER_PROFILE_SLEEP_MIN.as_secs()..=INTER_PROFILE_SLEEP_MAX.as_secs(),
  );
  Duration::from_secs(secs)
}

/// Round-end gap between draining the queue and refilling for the next
/// round. Intentionally small (5~20s) — per-profile cooldowns already
/// space out per-IP traffic, this just adds a tiny boundary jitter.
fn random_round_end_gap() -> Duration {
  use rand::RngExt;
  let secs = rand::rng().random_range(
    ROUND_END_GAP_MIN.as_secs()..=ROUND_END_GAP_MAX.as_secs(),
  );
  Duration::from_secs(secs)
}

// ---- supervisor (4h full-restart loop) --------------------------------

/// Top-level orchestrator spawned by `kol_batch_start`. Owns the
/// session lifecycle: builds a fresh Pool, spawns workers, waits for
/// either user-stop or 4h elapsed, tears down the Pool, then loops
/// back to build the next session.
///
/// The supervisor's `cancel` is the user-stop signal; the Pool's
/// `cancel` is the within-session signal that the supervisor itself
/// triggers at 4h. They're distinct so the supervisor can survive a
/// pool teardown.
async fn supervisor_loop(app: tauri::AppHandle, supervisor_cancel: CancellationToken) {
  let mut session_idx = 0usize;
  loop {
    if supervisor_cancel.is_cancelled() {
      break;
    }
    session_idx += 1;

    // Build a fresh session (kills leftover browsers, builds Pool,
    // spawns workers). On error, log + idle 30s + retry.
    let pool = match start_session(&app, session_idx).await {
      Ok(p) => p,
      Err(e) => {
        log::error!("supervisor: session {session_idx} start failed: {e}");
        EventBuilder::new(BatchEventKind::SessionStop)
          .detail(format!("start failed: {e}"))
          .emit();
        tokio::select! {
          _ = supervisor_cancel.cancelled() => break,
          _ = tokio::time::sleep(Duration::from_secs(30)) => continue,
        }
      }
    };

    // Watch for either the user-stop or the 4h budget expiring.
    let mut tick = tokio::time::interval(SUPERVISOR_TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let four_hours_up = loop {
      tokio::select! {
        _ = supervisor_cancel.cancelled() => break false,
        _ = tick.tick() => {
          if pool.session_started_inst.elapsed() >= FULL_RESTART_INTERVAL {
            break true;
          }
        }
      }
    };

    if four_hours_up {
      log::info!(
        "supervisor: session {session_idx} hit 4h cap, full-restarting"
      );
      EventBuilder::new(BatchEventKind::FullRestartTriggered)
        .detail(format!(
          "session {session_idx} reached {}h budget",
          FULL_RESTART_INTERVAL.as_secs() / 3600
        ))
        .emit();
    } else {
      log::info!("supervisor: session {session_idx} stop requested");
    }

    // Teardown: cancel pool (workers exit at their next check),
    // flush gather flags, kill browsers.
    teardown_session(&app, &pool).await;

    if four_hours_up {
      // Drop the pool reference + global so the next session starts
      // from a clean slate.
      *POOL.lock().ok().as_deref_mut().unwrap_or(&mut None) = None;
      EventBuilder::new(BatchEventKind::FullRestartComplete)
        .detail("ready for next session")
        .emit();
      // Brief gap so OS reclaims browser process resources.
      tokio::select! {
        _ = supervisor_cancel.cancelled() => break,
        _ = tokio::time::sleep(POST_FULL_RESTART_GAP) => {}
      }
      // Loop back to build session N+1.
    } else {
      // User-stop path — clear the global pool and exit the supervisor.
      if let Ok(mut g) = POOL.lock() {
        *g = None;
      }
      EventBuilder::new(BatchEventKind::SessionStop)
        .detail(format!("session {session_idx} stopped by user"))
        .emit();
      break;
    }
  }

  // Final cleanup. Clear the supervisor cancel handle so the next
  // batch_start treats us as fully done.
  if let Ok(mut g) = SUPERVISOR_CANCEL.lock() {
    *g = None;
  }
  log::info!("supervisor: exited");
}

/// Build a brand-new session: pre-kill stale browsers, allocate Pool,
/// fill round 1, spawn workers, publish to global. Returns the Pool
/// handle so the supervisor can wait on its session_started_inst.
async fn start_session(
  app: &tauri::AppHandle,
  session_idx: usize,
) -> Result<Arc<Pool>, String> {
  let profiles = list_douyin_profiles()?;
  if profiles.is_empty() {
    return Err("no douyin Wayfern profiles to launch".into());
  }
  log::info!(
    "supervisor: starting session {session_idx} with {} profile(s), pool size {}",
    profiles.len(),
    POOL_SIZE
  );

  // Pre-flight: kill any already-running douyin browsers so they
  // restart with this build's bundled extension and a clean SW state.
  pre_kill_running(app, &profiles).await;
  invalidate_profile_cache();

  // Reset flags so any prior session's stale state doesn't leak in.
  for p in &profiles {
    set_should_gather(p.id, false);
    clear_unauth_marker(p.id);
  }

  let total = profiles.len();
  let now = Local::now();
  let pool = Arc::new(Pool {
    inner: StdMutex::new(PoolInner {
      queue: profiles.into_iter().collect(),
      active: HashMap::new(),
      completed_in_round: HashSet::new(),
      total_in_round: total,
      round: 1,
      session_started_at: now,
      round_started_at: now,
    }),
    cancel: CancellationToken::new(),
    launch_gate: Arc::new(Semaphore::new(LAUNCH_CONCURRENCY)),
    refilling: AtomicBool::new(false),
    refill_notify: Notify::new(),
    session_started_inst: Instant::now(),
  });

  // Publish before spawning workers so status() can already see the
  // pool the moment workers start ticking.
  *POOL.lock().map_err(|_| "POOL poisoned")? = Some(pool.clone());

  EventBuilder::new(BatchEventKind::SessionStart)
    .detail(format!("session {session_idx} · {total} profile(s)"))
    .emit();
  EventBuilder::new(BatchEventKind::RoundStart)
    .round(1)
    .detail(format!("{total} profile(s) queued"))
    .emit();

  let n_workers = POOL_SIZE.min(total);
  for slot in 0..n_workers {
    let app = app.clone();
    let pool = pool.clone();
    tokio::spawn(async move { worker_loop(slot, app, pool).await });
  }

  Ok(pool)
}

/// Cancel the Pool, flush gather flags, kill any running browsers.
/// Used by both the 4h full-restart path and the user-stop path.
async fn teardown_session(app: &tauri::AppHandle, pool: &Arc<Pool>) {
  pool.cancel.cancel();

  // Snapshot active so we know what to definitely-kill even if their
  // CDP port check races.
  let active_ids: Vec<Uuid> = pool
    .inner
    .lock()
    .ok()
    .map(|i| i.active.keys().copied().collect())
    .unwrap_or_default();
  for id in &active_ids {
    set_should_gather(*id, false);
  }

  // Let extensions flush.
  tokio::time::sleep(STOP_FLUSH_GRACE_BATCH).await;

  let profiles = list_douyin_profiles().unwrap_or_default();
  let runner = BrowserRunner::instance();
  let profiles_dir = runner.profile_manager.get_profiles_dir();
  let wayfern = WayfernManager::instance();
  let port_checks: Vec<_> = profiles
    .iter()
    .map(|p| {
      let pp = p.get_profile_data_path(&profiles_dir);
      let pp_str = pp.to_string_lossy().to_string();
      let pid = p.id;
      let in_active = active_ids.contains(&pid);
      async move {
        let up = in_active || wayfern.get_cdp_port(&pp_str).await.is_some();
        (pid, up)
      }
    })
    .collect();
  let up_set: HashSet<Uuid> = futures_util::future::join_all(port_checks)
    .await
    .into_iter()
    .filter_map(|(pid, up)| if up { Some(pid) } else { None })
    .collect();
  let mut handles = Vec::new();
  for p in profiles {
    if !up_set.contains(&p.id) {
      continue;
    }
    let app = app.clone();
    handles.push(tokio::spawn(async move {
      let _ = crate::browser_runner::kill_browser_profile(app, p).await;
    }));
  }
  for h in handles {
    let _ = h.await;
  }

  // Free the bits we don't need to keep around between sessions.
  // SHOULD_GATHER stays (per-profile flag, naturally bounded), but
  // UNAUTH_SINCE / cache can be tossed. Dedup is intentionally NOT
  // cleared — its 24h TTL is part of the design.
  if let Ok(mut m) = UNAUTH_SINCE.lock() {
    m.clear();
  }
  invalidate_profile_cache();

  // Wake any worker still parked on refill_notify so they observe
  // cancel and exit.
  pool.refill_notify.notify_waiters();
}

/// Pre-kill helper used at session start. Identifies running douyin
/// profiles via CDP-port presence and kills them in parallel.
async fn pre_kill_running(app: &tauri::AppHandle, profiles: &[BrowserProfile]) {
  let runner = BrowserRunner::instance();
  let profiles_dir = runner.profile_manager.get_profiles_dir();
  let wayfern = WayfernManager::instance();
  let port_checks: Vec<_> = profiles
    .iter()
    .map(|p| {
      let pp = p.get_profile_data_path(&profiles_dir);
      let pp_str = pp.to_string_lossy().to_string();
      let pid = p.id;
      async move { (pid, wayfern.get_cdp_port(&pp_str).await.is_some()) }
    })
    .collect();
  let were_running: Vec<Uuid> = futures_util::future::join_all(port_checks)
    .await
    .into_iter()
    .filter_map(|(pid, up)| if up { Some(pid) } else { None })
    .collect();
  if were_running.is_empty() {
    return;
  }
  log::info!(
    "supervisor: killing {} already-running profile(s) for clean restart",
    were_running.len()
  );
  let kill_handles: Vec<_> = profiles
    .iter()
    .cloned()
    .filter(|p| were_running.contains(&p.id))
    .map(|p| {
      let app = app.clone();
      tokio::spawn(async move {
        if let Err(e) =
          crate::browser_runner::kill_browser_profile(app, p.clone()).await
        {
          log::warn!("batch pre-kill failed for {}: {e}", p.id);
        }
      })
    })
    .collect();
  for h in kill_handles {
    let _ = h.await;
  }
}

// ---- commands ----------------------------------------------------------

/// Start the supervisor — auto-loops rounds, full-restarts every 4h.
/// Idempotent only after a previous batch has fully torn down.
#[tauri::command]
pub async fn kol_batch_start(app: tauri::AppHandle) -> Result<BatchStatus, String> {
  // Refuse if a supervisor is already running. The previous one must
  // be cancelled (and have observed the cancel) before we start a
  // new one — otherwise both would race for POOL.
  {
    let mut g = SUPERVISOR_CANCEL
      .lock()
      .map_err(|_| "supervisor cancel lock poisoned".to_string())?;
    if g.is_some() {
      return Err("batch already running — stop first".into());
    }
    let token = CancellationToken::new();
    *g = Some(token);
  }

  let supervisor_cancel = SUPERVISOR_CANCEL
    .lock()
    .ok()
    .and_then(|g| g.clone())
    .ok_or_else(|| "supervisor cancel disappeared".to_string())?;

  // Sanity check that there's something to do BEFORE we let the
  // supervisor loop forever waiting on an empty profile list.
  let profiles = list_douyin_profiles()?;
  if profiles.is_empty() {
    if let Ok(mut g) = SUPERVISOR_CANCEL.lock() {
      *g = None;
    }
    return Err("no douyin Wayfern profiles to launch".into());
  }
  drop(profiles);

  let app_for_supervisor = app.clone();
  tokio::spawn(async move {
    supervisor_loop(app_for_supervisor, supervisor_cancel).await;
  });

  // Give the supervisor a moment to publish the pool so the immediate
  // status snapshot reflects the running state. Without this the
  // first poll returns idle even though we just started.
  for _ in 0..20 {
    if current_pool().is_some() {
      break;
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
  }

  snapshot_status().await
}

/// Stop the supervisor + tear down whatever Pool is currently active.
/// Returns the post-teardown status snapshot.
#[tauri::command]
pub async fn kol_batch_stop(app: tauri::AppHandle) -> Result<BatchStatus, String> {
  // Cancel the supervisor first; it will tear down its current Pool
  // and exit naturally. If there is no supervisor (i.e. user clicks
  // stop on an already-stopped batch) fall back to the legacy
  // "kill anything that's running" path.
  let supervisor = {
    let mut g = SUPERVISOR_CANCEL
      .lock()
      .map_err(|_| "supervisor cancel lock poisoned".to_string())?;
    g.take()
  };

  match supervisor {
    Some(token) => {
      log::info!("kol_batch_stop: cancelling supervisor");
      token.cancel();
      // The supervisor will run teardown_session itself, but we want
      // the response to reflect a fully-torn-down state — so wait for
      // POOL to clear with a bounded timeout.
      let deadline = Instant::now() + Duration::from_secs(20);
      while current_pool().is_some() && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;
      }
    }
    None => {
      // No supervisor active — defensive fallback. Mirrors the old
      // legacy behavior so an operator can recover from a stuck
      // half-started state.
      let profiles = list_douyin_profiles().unwrap_or_default();
      for p in &profiles {
        set_should_gather(p.id, false);
      }
      tokio::time::sleep(STOP_FLUSH_GRACE_BATCH).await;
      let mut handles = Vec::new();
      for p in profiles {
        let app = app.clone();
        handles.push(tokio::spawn(async move {
          let _ = crate::browser_runner::kill_browser_profile(app, p).await;
        }));
      }
      for h in handles {
        let _ = h.await;
      }
      if let Ok(mut g) = POOL.lock() {
        *g = None;
      }
    }
  }

  snapshot_status().await
}

#[tauri::command]
pub async fn kol_batch_status() -> Result<BatchStatus, String> {
  snapshot_status().await
}

/// Recent batch events, newest-first. Powers the monitoring panel —
/// the UI may filter / page client-side. Bounded at EVENT_LOG_CAP.
#[tauri::command]
pub fn kol_batch_events() -> Vec<BatchEvent> {
  events_snapshot()
}

/// Start a SINGLE douyin profile with the same semantics as a batch
/// worker would: flip its `should_gather` flag so the extension
/// auto-starts gathering once the page loads, then call the canonical
/// launch path. Does NOT touch the rolling pool — useful when the user
/// wants to bring up just one profile (or test an individual one)
/// without committing to a full batch round.
#[tauri::command]
pub async fn kol_start_single_profile(
  app: tauri::AppHandle,
  profile_id: Uuid,
) -> Result<(), String> {
  let profiles = list_douyin_profiles()?;
  let Some(profile) = profiles.into_iter().find(|p| p.id == profile_id) else {
    return Err(format!(
      "profile {profile_id} not found or not a douyin Wayfern profile"
    ));
  };
  set_should_gather(profile_id, true);
  clear_unauth_marker(profile_id);
  invalidate_profile_cache();
  crate::browser_runner::launch_browser_profile(
    app,
    profile.clone(),
    Some(DOUYIN_URL.into()),
  )
  .await?;
  Ok(())
}

/// Stop a SINGLE douyin profile: clear its `should_gather` flag (so
/// the extension flushes), brief grace, then kill the browser. Mirror
/// of kol_start_single_profile.
#[tauri::command]
pub async fn kol_stop_single_profile(
  app: tauri::AppHandle,
  profile_id: Uuid,
) -> Result<(), String> {
  let profiles = list_douyin_profiles()?;
  let Some(profile) = profiles.into_iter().find(|p| p.id == profile_id) else {
    return Err(format!("profile {profile_id} not found"));
  };
  set_should_gather(profile_id, false);
  clear_unauth_marker(profile_id);
  // 3s flush window — matches BATCH_FLUSH inside content.js plus a bit
  // of slack so its in-flight POST has a chance to complete.
  tokio::time::sleep(Duration::from_secs(3)).await;
  if let Err(e) =
    crate::browser_runner::kill_browser_profile(app, profile).await
  {
    log::warn!("kol_stop_single_profile {profile_id}: {e}");
  }
  invalidate_profile_cache();
  Ok(())
}
