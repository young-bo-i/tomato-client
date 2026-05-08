//! Routes each (word, platform, alias_type) to the profile that should
//! submit it, respecting per-profile enabled/daily_limit config plus
//! the 2-level tier hierarchy (admin / tier-1 / tier-2 → tier-1).
//!
//! ## Tier model
//!
//! - **admin**: top tier. Has tomato/qimao accounts as a global pool.
//! - **tier-1** (`users.parent_user_id IS NULL`, role='user'): default
//!   non-admin user. Has their own pool, plus admin pool as fallback.
//! - **tier-2** (`users.parent_user_id IS NOT NULL`): subordinate of
//!   exactly one tier-1. Has their own pool, plus the parent's pool
//!   AND admin pool as fallbacks.
//!
//! ## Per-word decision (sequential cascade)
//!
//! For each word from a non-admin caller:
//!
//! ```text
//! admin_acc += admin_contribution_pct       (global setting)
//! if admin_acc >= 100:
//!     decision = Admin; admin_acc -= 100; return
//!
//! if caller is tier-2 AND parent has tier2_contribution_pct > 0:
//!     parent_acc += parent.tier2_contribution_pct
//!     if parent_acc >= 100:
//!         decision = Parent; parent_acc -= 100; return
//!
//! decision = Self
//! ```
//!
//! Crucially: `parent_acc` is only incremented on words that admin
//! DIDN'T claim. So if admin_pct=20, parent_pct=30, the effective
//! distribution over 100 words is 20 admin + 0.30×80 = 24 parent +
//! 56 self. No constraint that admin_pct + parent_pct ≤ 100.
//!
//! Per-(user_id, platform) state lives in-memory in `CONTRIB_STATE`:
//! resets on server restart but the steady-state ratio is preserved.
//!
//! Admin callers bypass the cascade entirely (their pool == admin pool).
//!
//! ## Three-tier fallback per decision
//!
//! Each decision tries its preferred tier first, then admin, then
//! parent (if any), then self. Avoids dropping a word just because the
//! preferred tier is at capacity:
//!
//! | Decision | 1st  | 2nd    | 3rd    | 4th |
//! |----------|------|--------|--------|-----|
//! | Self     | self | admin  | parent | —   |
//! | Parent   | parent | admin | self   | —   |
//! | Admin    | admin | parent | self   | —   |
//!
//! ## Default priority (when contribution is disabled)
//!
//! Equivalent to "Self decision" above: Self → Admin → Parent (if any)
//! → discard. Profile with no config row is treated as enabled=true,
//! limit=0 (unlimited), so new profiles work out-of-the-box.
//!
//! ## Usage cache
//!
//! The "today's usage count per (profile, alias_type)" GROUP BY scan
//! grows linearly with the day's volume — by evening it scans tens of
//! thousands of rows. With bulk_create at 5–10 Hz and two platforms,
//! that scan was running 10–20 times per second.
//!
//! We cache the usage map per (user_id, platform) for 5 seconds. Within
//! the window, concurrent batches share the same baseline. This means
//! daily_limit becomes "soft" — bursts of concurrent enqueues may exceed
//! the limit by ~window_size × per-second-rate, capped to ~10–20%.
//! That's acceptable since daily_limit is admin-configured throttling,
//! not a hard contractual cap.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use once_cell::sync::Lazy;
use uuid::Uuid;

use crate::db::DbPool;

/// Today's usage map: (profile_id, alias_type) → count.
type UsageMap = HashMap<(Uuid, i32), i32>;

const USAGE_TTL: Duration = Duration::from_secs(5);

/// (user_id, platform) → (loaded_at, usage_map).
static USAGE_CACHE: Lazy<RwLock<HashMap<(i32, String), (Instant, Arc<UsageMap>)>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));

fn read_usage_cache(key: &(i32, String)) -> Option<Arc<UsageMap>> {
    let g = USAGE_CACHE.read().ok()?;
    let (ts, map) = g.get(key)?;
    if ts.elapsed() < USAGE_TTL {
        Some(Arc::clone(map))
    } else {
        None
    }
}

fn write_usage_cache(key: (i32, String), map: Arc<UsageMap>) {
    if let Ok(mut g) = USAGE_CACHE.write() {
        g.insert(key, (Instant::now(), map));
    }
}

/// One candidate profile's effective config for a single (platform, alias_type).
#[derive(Debug, Clone)]
struct Candidate {
    profile_id: Uuid,
    is_admin: bool,
    enabled: bool,
    daily_limit: i32, // 0 = unlimited
    used_today: i32,
}

impl Candidate {
    fn has_capacity(&self) -> bool {
        self.enabled && (self.daily_limit == 0 || self.used_today < self.daily_limit)
    }
}

/// Per-(user_id, platform) accumulator state for the sequential
/// cascade. Two independent residues, each modeling a Bresenham-like
/// stream with its own pct.
///
/// * `admin_acc`: every word adds `admin_contribution_pct` (the global
///   setting). When it crosses 100, a slot for admin is "due" and the
///   decision returns Admin.
/// * `parent_acc`: only incremented on words that admin DIDN'T claim,
///   and only when the caller is tier-2 with a non-zero parent rate.
///   When it crosses 100, decision is Parent.
///
/// Locked behind a `Mutex` per entry so the read-modify-write of both
/// fields is atomic. Hot path on the inner mutex; outer `RwLock` is
/// touched only to seed new entries.
struct ContribState {
    admin_acc: i32,
    parent_acc: i32,
}

static CONTRIB_STATE: Lazy<RwLock<HashMap<(i32, String), Arc<std::sync::Mutex<ContribState>>>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));

/// Get-or-create the contribution-state arc for `(user_id, platform)`.
/// Returns an Arc so the caller can drop the outer RwLock guard before
/// taking the inner Mutex.
fn contrib_state_for(user_id: i32, platform: &str) -> Arc<std::sync::Mutex<ContribState>> {
    let key = (user_id, platform.to_string());

    // Fast path: read-lock the outer map.
    if let Ok(g) = CONTRIB_STATE.read() {
        if let Some(s) = g.get(&key) {
            return Arc::clone(s);
        }
    }
    // Slow path: write-lock + double-check.
    let mut g = CONTRIB_STATE.write().expect("CONTRIB_STATE poisoned");
    g.entry(key)
        .or_insert_with(|| Arc::new(std::sync::Mutex::new(ContribState { admin_acc: 0, parent_acc: 0 })))
        .clone()
}

/// One word's tier decision after running the sequential cascade.
#[derive(Debug, Clone, Copy)]
pub enum Tier {
    /// Caller's own pool.
    Self_,
    /// Caller's parent's pool (tier-2 only).
    Parent,
    /// Admin pool.
    Admin,
}

/// Router state for one enqueue batch. Built once per enqueue call and
/// updated as assignments are made — so two words in the same batch correctly
/// share the same daily counter and don't both see "0 used today".
pub struct Router {
    /// Calling user's id. Scopes the contribution accumulator.
    user_id: i32,
    /// Platform this router covers (`tomato` / `qimao`).
    platform: String,
    /// True when the calling user is an admin. Admin callers bypass
    /// the cascade entirely (their pool already IS the admin pool).
    caller_is_admin: bool,
    /// Snapshot of `admin_settings.admin_contribution_pct` at build.
    /// Step 1 of the cascade.
    admin_contribution_pct: i32,
    /// Snapshot of the caller's PARENT's `tier2_contribution_pct` at
    /// build. 0 when caller is tier-1 (no parent) or admin. Step 2 of
    /// the cascade.
    parent_contribution_pct: i32,
    /// profile_id → (platform, alias_type) → Candidate
    candidates: HashMap<Uuid, HashMap<(String, i32), Candidate>>,
    /// Caller's own profiles (matches user_id == self.user_id).
    self_profiles: Vec<Uuid>,
    /// Caller's parent's profiles (tier-2 only). Empty for tier-1 / admin.
    parent_profiles: Vec<Uuid>,
    /// All admin-owned profiles. Includes admins other than the caller's
    /// parent (parent is never admin in this hierarchy).
    admin_profiles: Vec<Uuid>,
}

impl Router {
    /// Load all relevant profiles + their config + today's usage,
    /// plus the parent's `tier2_contribution_pct` if the caller is
    /// tier-2.
    pub async fn load(
        pool: &DbPool,
        user_id: i32,
        user_role: &str,
        platform: &str,
        alias_types: &[i32],
    ) -> Result<Self, String> {
        let cache_key = (user_id, platform.to_string());

        // Acquire profile_rows + usage. Cache hit → 1 query. Miss → 2
        // queries run in parallel via tokio::join.
        let (profile_rows, usage): (Vec<ProfileRow>, Arc<UsageMap>) =
            match read_usage_cache(&cache_key) {
                Some(u) => {
                    let profile_rows = load_profiles(pool, user_id, platform).await?;
                    (profile_rows, u)
                }
                None => {
                    let (profiles_res, usage_res) = tokio::join!(
                        load_profiles(pool, user_id, platform),
                        load_usage(pool, platform),
                    );
                    let profile_rows = profiles_res?;
                    let usage_arc = Arc::new(usage_res?);
                    write_usage_cache(cache_key, Arc::clone(&usage_arc));
                    (profile_rows, usage_arc)
                }
            };

        // Look up caller's tier hierarchy in one query so we know:
        //   * is caller tier-2? (parent_user_id IS NOT NULL)
        //   * if yes, what is parent's tier2_contribution_pct?
        // Returns (parent_user_id, parent_tier2_pct). For tier-1 / admin
        // callers, parent_user_id is None and parent_tier2_pct = 0.
        let (parent_user_id, parent_contribution_pct) =
            load_caller_tier(pool, user_id).await.unwrap_or((None, 0));

        // Config: from cache (hot path — Arc clone is O(1))
        let config_cache = crate::services::cache::get_submission_config(pool).await?;

        // Admin contribution snapshot — captured at build, not re-read
        // per call, so a slider change mid-batch doesn't disturb words
        // already being processed.
        let settings = crate::services::admin_settings::get(pool).await?;
        let admin_contribution_pct = settings.admin_contribution_pct;

        let mut candidates: HashMap<Uuid, HashMap<(String, i32), Candidate>> = HashMap::new();

        for row in &profile_rows {
            for &atype in alias_types {
                let used = usage.get(&(row.profile_id, atype)).copied().unwrap_or(0);
                let (enabled, daily_limit) = config_cache
                    .get(&row.profile_id)
                    .and_then(|m| m.get(&(platform.to_string(), atype)))
                    .copied()
                    .unwrap_or((true, 0));
                candidates
                    .entry(row.profile_id)
                    .or_default()
                    .entry((platform.to_string(), atype))
                    .or_insert(Candidate {
                        profile_id: row.profile_id,
                        is_admin: row.is_admin,
                        enabled,
                        daily_limit,
                        used_today: used,
                    });
            }
        }

        // Partition profiles into the three tiers. Order:
        //   1. self_profiles  — user_id == caller
        //   2. parent_profiles — user_id == caller's parent (only for tier-2)
        //   3. admin_profiles  — anything else (must be admin given the
        //      load_profiles WHERE clause)
        // A profile can match exactly one tier; the load_profiles query
        // is structured so this is unambiguous (caller's own / caller's
        // parent's / admin's).
        let mut self_profiles = Vec::new();
        let mut parent_profiles = Vec::new();
        let mut admin_profiles = Vec::new();
        for row in &profile_rows {
            if row.owner_user_id == user_id {
                self_profiles.push(row.profile_id);
            } else if Some(row.owner_user_id) == parent_user_id {
                parent_profiles.push(row.profile_id);
            } else if row.is_admin {
                admin_profiles.push(row.profile_id);
            }
        }

        Ok(Self {
            user_id,
            platform: platform.to_string(),
            caller_is_admin: user_role == "admin",
            admin_contribution_pct,
            parent_contribution_pct,
            candidates,
            self_profiles,
            parent_profiles,
            admin_profiles,
        })
    }

    /// Run the sequential cascade for ONE word and return its tier
    /// decision. Mutates the per-(user, platform) accumulator state.
    /// Call once per word (NOT per alias_type) so all alias_types of
    /// the same word share the same tier decision — otherwise tomato's
    /// 3 rows could split across tiers, which the operator's mental
    /// model doesn't expect.
    pub fn decide_tier(&self) -> Tier {
        // Admin callers always route through admin pool (their pool IS
        // the admin pool). Skip the cascade.
        if self.caller_is_admin {
            return Tier::Admin;
        }

        // Parent cascade is meaningful only when parent_pct > 0 AND
        // the parent actually has a pool for this platform. If parent
        // configured a non-zero rate but holds no profile here, every
        // "Parent" decision would still fall back to admin/self via
        // pick_for_tier — but the parent_acc would be consumed for
        // nothing, drifting the long-run distribution away from the
        // configured rate. Treating "no parent pool" as "parent step
        // disabled" keeps the configured rate honest.
        let parent_step_active =
            self.parent_contribution_pct > 0 && !self.parent_profiles.is_empty();

        // No contribution configured anywhere → always Self.
        if self.admin_contribution_pct <= 0 && !parent_step_active {
            return Tier::Self_;
        }

        let state_arc = contrib_state_for(self.user_id, &self.platform);
        let mut s = state_arc.lock().expect("CONTRIB_STATE inner mutex poisoned");

        // Step 1: admin cascade (always runs when pct > 0).
        if self.admin_contribution_pct > 0 {
            s.admin_acc += self.admin_contribution_pct;
            if s.admin_acc >= 100 {
                s.admin_acc -= 100;
                return Tier::Admin;
            }
        }

        // Step 2: parent cascade (only when parent_step_active). Note:
        // parent_acc is only incremented on words that DIDN'T go to
        // admin — so the parent rate is a share of the user's
        // "remaining" words, not the total. Yields the sequential
        // semantics the operator described:
        //   admin=20%, parent=30%
        //     → 20 admin + (0.30 × 80) = 20 + 24 + 56 self per 100 words.
        if parent_step_active {
            s.parent_acc += self.parent_contribution_pct;
            if s.parent_acc >= 100 {
                s.parent_acc -= 100;
                return Tier::Parent;
            }
        }

        Tier::Self_
    }

    /// Pick a profile for one (platform, alias_type) given the tier
    /// decision from `decide_tier`. Tries the preferred tier first,
    /// then admin, then parent (if any), then self — so a "decision =
    /// Parent" word still lands SOMEWHERE if parent pool is full.
    /// Returns None iff every tier is at capacity for this key →
    /// caller should discard.
    ///
    /// Updates the chosen profile's `used_today` so subsequent picks in
    /// the same batch reflect the assignment.
    pub fn pick_for_tier(
        &mut self,
        platform: &str,
        alias_type: i32,
        decision: Tier,
    ) -> Option<Uuid> {
        let key = (platform.to_string(), alias_type);

        // Order tiers by decision-driven preference.
        //
        //   * Self decision → keep with caller's circle if at all
        //     possible: self → parent (still "my people") → admin
        //     (system overflow). The decision said "don't contribute
        //     this word out", so admin is last resort.
        //   * Parent decision → parent first, then admin (system),
        //     then self.
        //   * Admin decision → admin first, then parent (still
        //     "outside" the caller), then self.
        let order: [&[Uuid]; 3] = match decision {
            Tier::Self_ => [&self.self_profiles, &self.parent_profiles, &self.admin_profiles],
            Tier::Parent => [&self.parent_profiles, &self.admin_profiles, &self.self_profiles],
            Tier::Admin => [&self.admin_profiles, &self.parent_profiles, &self.self_profiles],
        };

        let mut chosen: Option<Uuid> = None;
        for tier in order {
            if let Some(pid) = best_in_tier(&self.candidates, &key, tier) {
                chosen = Some(pid);
                break;
            }
        }

        if let Some(pid) = chosen {
            if let Some(slot) = self.candidates.get_mut(&pid).and_then(|m| m.get_mut(&key)) {
                slot.used_today += 1;
            }
            Some(pid)
        } else {
            None
        }
    }
}

/// Helper: among `pids`, find the (platform, alias_type) candidate
/// with capacity and the lowest `used_today`. Returns its profile_id
/// or None if no candidate has capacity.
fn best_in_tier(
    candidates: &HashMap<Uuid, HashMap<(String, i32), Candidate>>,
    key: &(String, i32),
    pids: &[Uuid],
) -> Option<Uuid> {
    let mut best: Option<(Uuid, i32)> = None;
    for &pid in pids {
        if let Some(slot) = candidates.get(&pid).and_then(|m| m.get(key)) {
            if slot.has_capacity() {
                match best {
                    None => best = Some((pid, slot.used_today)),
                    Some((_, b_used)) if slot.used_today < b_used => {
                        best = Some((pid, slot.used_today));
                    }
                    _ => {}
                }
            }
        }
    }
    best.map(|(pid, _)| pid)
}

#[derive(sqlx::FromRow)]
struct ProfileRow {
    profile_id: Uuid,
    /// Profile owner (browser_profiles.user_id). Used to partition
    /// candidates into self / parent / admin tiers.
    owner_user_id: i32,
    /// True when the owner's role is 'admin'.
    is_admin: bool,
}

/// Load all profiles relevant to the caller's routing decisions:
///
///   * caller's own profiles
///   * caller's parent's profiles (tier-2 case; via `users.parent_user_id`)
///   * any active admin's profiles
///
/// `is_active = TRUE` filtering keeps deactivated owners out of the
/// pool — otherwise the router could pick a deactivated owner's
/// profile and the downstream cookie/token picker (which DOES filter
/// on is_active) would fail to surface a credential, wedging the row
/// in `pending`.
async fn load_profiles(
    pool: &DbPool,
    user_id: i32,
    platform: &str,
) -> Result<Vec<ProfileRow>, String> {
    sqlx::query_as::<_, ProfileRow>(
        r#"SELECT bp.id            AS profile_id,
                  bp.user_id       AS owner_user_id,
                  (u.role = 'admin') AS is_admin
           FROM browser_profiles bp
           JOIN users u ON u.id = bp.user_id
           WHERE bp.kol_platform = $1
             AND u.is_active = TRUE
             AND (
                 bp.user_id = $2                                       -- self
              OR bp.user_id = (SELECT parent_user_id FROM users WHERE id = $2)  -- parent
              OR u.role = 'admin'                                      -- admin pool
             )"#,
    )
    .bind(platform)
    .bind(user_id)
    .fetch_all(pool)
    .await
    .map_err(|e| format!("load profiles: {e}"))
}

/// Look up the caller's tier in one query. Returns
/// `(parent_user_id, parent_tier2_pct)`:
///   * tier-1 / admin caller → `(None, 0)`
///   * tier-2 caller         → `(Some(parent_id), parent's tier2_contribution_pct)`
async fn load_caller_tier(pool: &DbPool, user_id: i32) -> Result<(Option<i32>, i32), String> {
    let row = sqlx::query(
        r#"SELECT u.parent_user_id,
                  COALESCE(p.tier2_contribution_pct, 0) AS parent_tier2_pct
           FROM users u
           LEFT JOIN users p ON p.id = u.parent_user_id AND p.is_active = TRUE
           WHERE u.id = $1"#,
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("load caller tier: {e}"))?;

    use sqlx::Row;
    match row {
        Some(r) => Ok((
            r.try_get("parent_user_id").ok().flatten(),
            r.try_get("parent_tier2_pct").unwrap_or(0),
        )),
        None => Ok((None, 0)),
    }
}

#[derive(sqlx::FromRow)]
struct UsageRow {
    profile_id: Uuid,
    alias_type: i32,
    count: i32,
}

/// `daily_limit` is a per-account cap on **successful submissions per
/// day**. A row that ends up `failed` (rejected by the platform,
/// transport error, etc.) does NOT consume the cap.
///
/// What we count is `pending + submitted today`, NOT just `submitted`:
///
/// - **submitted**: filtered by `submitted_at >= CURRENT_DATE` so
///   yesterday's submissions don't carry over, and renewals (which
///   bump `submitted_at` to NOW()) correctly count toward today's
///   budget.
/// - **pending today**: rows enqueued today that are in flight.
///   Counted because otherwise the 5s usage cache would let multiple
///   batches over-allocate while workers haven't drained them yet —
///   strict success-only counting + async worker lag → systematic
///   overshoot of `daily_limit`.
/// - **failed**: never counted. When a row fails, the cap slot is
///   freed and the router will route a replacement to the same
///   account on the next batch.
///
/// Net effect: an account configured for daily_limit=100 will land
/// **at most 100 successful submissions per day** (instead of "as
/// many as possible until 100 succeed", which the strict reading
/// would imply but the cache makes infeasible). If many submissions
/// fail in a row the daily total may UNDERSHOOT 100; the router
/// chooses undershoot over overshoot since the user-described intent
/// is "don't exceed the limit".
async fn load_usage(pool: &DbPool, platform: &str) -> Result<UsageMap, String> {
    let rows: Vec<UsageRow> = if platform == "tomato" {
        sqlx::query_as(
            r#"SELECT target_profile_id AS profile_id, alias_type, COUNT(*)::int AS count
               FROM tomato_aliases
               WHERE target_profile_id IS NOT NULL
                 AND (
                   (status = 'submitted' AND submitted_at >= CURRENT_DATE)
                   OR (status = 'pending' AND created_at >= CURRENT_DATE)
                 )
               GROUP BY target_profile_id, alias_type"#,
        )
        .fetch_all(pool)
        .await
        .map_err(|e| format!("load tomato usage: {e}"))?
    } else {
        sqlx::query_as(
            r#"SELECT target_profile_id AS profile_id, 1::int AS alias_type, COUNT(*)::int AS count
               FROM qimao_aliases
               WHERE target_profile_id IS NOT NULL
                 AND (
                   (status = 'submitted' AND submitted_at >= CURRENT_DATE)
                   OR (status = 'pending' AND created_at >= CURRENT_DATE)
                 )
               GROUP BY target_profile_id"#,
        )
        .fetch_all(pool)
        .await
        .map_err(|e| format!("load qimao usage: {e}"))?
    };
    Ok(rows
        .into_iter()
        .map(|r| ((r.profile_id, r.alias_type), r.count))
        .collect())
}

