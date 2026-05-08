//! Throttled global `sysinfo::System` snapshot.
//!
//! `check_browser_status` (and its `wayfern`/`camoufox` variants) used
//! to call `System::new_with_specifics(...everything())` on every
//! invocation. With ~100 profiles polling every 30s, that became ~100
//! full process-table enumerations per tick. This module caches one
//! refresh and serves all callers in the same window from a single
//! snapshot.
//!
//! Concurrency: a `tokio::Mutex` serializes access. Refresh is the
//! expensive bit (5–50ms depending on host process count); each
//! caller's lookup work is typically O(1) (PID hashmap probe), so the
//! serialization is cheap. Callers that iterate the full process list
//! are still cheaper than spinning up their own snapshot.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, RefreshKind, System};
use tokio::sync::Mutex;

/// Maximum staleness of the cached snapshot. The 30s
/// `check_browser_status` poll fans out to many profiles within a few
/// hundred ms, so a 1s window means each tick refreshes at most once.
const REFRESH_INTERVAL: Duration = Duration::from_secs(1);

struct Cell {
  sys: System,
  refreshed_at: Instant,
}

fn cell() -> &'static Mutex<Cell> {
  static CELL: OnceLock<Mutex<Cell>> = OnceLock::new();
  CELL.get_or_init(|| {
    let sys = System::new_with_specifics(
      RefreshKind::nothing().with_processes(ProcessRefreshKind::everything()),
    );
    Mutex::new(Cell {
      sys,
      refreshed_at: Instant::now(),
    })
  })
}

/// Run `f` against a recent process snapshot. Refreshes the underlying
/// `System` when the cached snapshot is older than `REFRESH_INTERVAL`,
/// then yields it to the closure. Multiple concurrent callers share a
/// single refresh per window.
pub async fn with_processes<R>(f: impl FnOnce(&System) -> R) -> R {
  let mut snap = cell().lock().await;
  if snap.refreshed_at.elapsed() >= REFRESH_INTERVAL {
    snap.sys.refresh_processes_specifics(
      ProcessesToUpdate::All,
      true,
      ProcessRefreshKind::everything(),
    );
    snap.refreshed_at = Instant::now();
  }
  f(&snap.sys)
}
