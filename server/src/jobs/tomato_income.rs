//! 10-minute poller that pulls the per-tomato-account income snapshot
//! from 番茄达人's `/api/platform/user/income/stats` endpoint, upserts
//! into the `kol_income` table, and **emails the profile owner** when
//! a forward jump in `total_income` is detected.
//!
//! Full port of the legacy `KolScheduled.IncomeNoticeJob` semantics:
//!
//!   1. Concurrent fetch (8-parallel) for every active+online tomato
//!      cookie owner.
//!   2. **2-minute skew gate**: `if max(LUT) > NOW - 2min: return`.
//!      The platform's snapshot is still settling when our request
//!      lands within 2 minutes of its `latest_update_time` — reading
//!      then can produce values that flip back on the next poll.
//!   3. **Idempotency**: `if upstream_max_LUT <= db_max_LUT: return`.
//!      Means no account has anything newer than what's already
//!      persisted; skip the per-row diff + upsert step.
//!   4. Per-account diff: when `db.total_income < upstream.total_income`
//!      record `last_diff = upstream - db, last_diff_at = NOW`. Used
//!      by the admin UI for "🆙 +¥XX since last poll" markers.
//!   5. Bulk UPSERT all fetched rows (regardless of diff) so
//!      `fetched_at` is a heartbeat for "is this account still
//!      reachable" even when income hasn't moved.
//!   6. Email diffs: collect rows with `diff > 0`, group by recipient
//!      (owner.email → admin fallback), send one HTML email per
//!      recipient summarizing all their changed accounts. Update
//!      `last_emailed_at` on success; persist `last_email_error` on
//!      failure (admin sees both in the panel).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Local, TimeZone};
use futures_util::stream::StreamExt;
use serde_json::{json, Value as JsonValue};
use sqlx::Row;
use uuid::Uuid;

use crate::db::DbPool;
use crate::services::email_sender::{self, EmailSettings};
use crate::services::fanqie_income::{
    build_http_client, fetch_income, IncomeData, IncomeRecord, ENDPOINT_INCOME_STATS,
    SERVICE_NAME,
};
use crate::services::tomato_cookie;

/// Cron cadence — every 10 minutes, matching the legacy
/// `0 */10 * * * ?`. Income data updates upstream are roughly
/// hour-grained so anything finer just burns API calls.
const POLL_INTERVAL: Duration = Duration::from_secs(600);

/// Concurrent fetches per round. Legacy `MaxDegreeOfParallelism = 8`.
/// The upstream is happy with this since each cookie maps to a
/// different account on their side; this is not a single-IP burst.
const FETCH_CONCURRENCY: usize = 8;

/// 2-minute skew gate. Captured from the legacy comment
/// `var dateTime = DateTime.Now.AddMinutes(-2)` — a defensive buffer
/// against the platform's still-settling latest-update window.
const SKEW_WINDOW: Duration = Duration::from_secs(120);

pub async fn start_worker(pool: Arc<DbPool>, abogus_url: Arc<String>) {
    let p = pool.clone();
    crate::jobs::poller_loop("tomato_income", POLL_INTERVAL, p, move || {
        let pool = pool.clone();
        let abogus_url = abogus_url.clone();
        async move { run_round(&pool, &abogus_url).await }
    })
    .await;
}

/// One poller tick. Returns the count of profiles whose income row was
/// (re-)written this round; 0 means "skipped" (skew window or no
/// newer data) — `poller_loop` doesn't write a `job_runs` row for 0
/// so quiet rounds stay out of the audit feed.
async fn run_round(pool: &DbPool, abogus_url: &str) -> Result<usize, String> {
    // ── Step 1: gather all online tomato accounts to poll ──────────
    let profiles = list_online_tomato_profiles(pool).await?;
    if profiles.is_empty() {
        // Nothing to do. Don't log — `poller_loop` skips Ok(0).
        return Ok(0);
    }

    // ── Step 2: parallel fetch via buffer_unordered ────────────────
    // Caps in-flight at FETCH_CONCURRENCY. Each fetch gets its own
    // audit-log line via `.audit(...)` so failures land in
    // `external_api_responses` for triage.
    let http = build_http_client()?;
    let abogus = abogus_url.to_string();

    let fetched: Vec<(ProfileWithCookie, IncomeRecord)> =
        futures_util::stream::iter(profiles)
            .map(|p| {
                let http = http.clone();
                let abogus = abogus.clone();
                let pool = pool.clone();
                async move {
                    let outcome = fetch_income(&http, &abogus, &p.cookie_header).await;
                    let request_summary = json!({
                        "profile_id": p.profile_id,
                    });
                    match outcome
                        .audit(&pool, SERVICE_NAME, ENDPOINT_INCOME_STATS, request_summary)
                        .await
                    {
                        Ok(rec) => Some((p, rec)),
                        Err(err) if err.is_auth_failure() => {
                            // 401/403 → cookie is dead. Mark offline so
                            // alias/backfill workers also skip it.
                            tomato_cookie::mark_offline(
                                &pool,
                                p.profile_id,
                                &format!("income: {err}"),
                            )
                            .await
                            .ok();
                            tracing::warn!(
                                "tomato_income: cookie dead profile={} {err}",
                                p.profile_id
                            );
                            None
                        }
                        Err(err) => {
                            tracing::warn!(
                                "tomato_income: fetch failed profile={} {err}",
                                p.profile_id
                            );
                            None
                        }
                    }
                }
            })
            .buffer_unordered(FETCH_CONCURRENCY)
            .filter_map(|x| async move { x })
            .collect()
            .await;

    if fetched.is_empty() {
        return Ok(0);
    }

    // ── Step 3: 2-min skew gate ────────────────────────────────────
    // Compute upstream's max latest_update_time across ALL fetched
    // accounts (matches legacy behavior: ANY fresh account → whole
    // round skips). Use Unix seconds throughout to avoid TZ math.
    let now_unix = chrono::Local::now().timestamp();
    let upstream_max_lut: i64 = fetched
        .iter()
        .map(|(_, r)| r.data.latest_update_time)
        .filter(|&t| t > 0)
        .max()
        .unwrap_or(0);

    if upstream_max_lut > 0 && (now_unix - upstream_max_lut) < SKEW_WINDOW.as_secs() as i64 {
        tracing::info!(
            "tomato_income: skew skip — upstream max_lut={} is within {}s of now",
            upstream_max_lut,
            SKEW_WINDOW.as_secs()
        );
        return Ok(0);
    }

    // ── Step 4: idempotency gate against persisted MAX(LUT) ────────
    let db_max_lut: i64 = sqlx::query_scalar::<_, Option<DateTime<Local>>>(
        "SELECT MAX(latest_update_time) FROM kol_income",
    )
    .fetch_one(pool)
    .await
    .map_err(|e| format!("max db lut: {e}"))?
    .map(|t| t.timestamp())
    .unwrap_or(0);

    if upstream_max_lut > 0 && upstream_max_lut <= db_max_lut {
        tracing::info!(
            "tomato_income: idempotent skip — upstream max_lut={} ≤ db max_lut={}",
            upstream_max_lut,
            db_max_lut
        );
        return Ok(0);
    }

    // ── Step 5: load existing rows for diff computation ────────────
    let profile_ids: Vec<Uuid> = fetched.iter().map(|(p, _)| p.profile_id).collect();
    let existing = load_existing(pool, &profile_ids).await?;

    // ── Step 6: per-row diff + UPSERT ──────────────────────────────
    // While we're at it, build the email queue: rows with `diff > 0`
    // get queued for the post-UPSERT email step. We don't email
    // inside the diff loop because (a) we want all DB writes to
    // commit first so the email reflects committed state, and (b)
    // grouping by recipient lets one user with multiple changed
    // accounts get one digest email instead of N.
    let mut written = 0usize;
    let mut emailable: Vec<EmailableUpdate> = Vec::new();
    for (profile, record) in &fetched {
        let prev_total: i64 = existing
            .iter()
            .find(|(pid, _)| *pid == profile.profile_id)
            .map(|(_, total)| *total)
            .unwrap_or(0);
        let new_total = record.data.total_income;
        let diff = new_total - prev_total;
        // Only count strictly-positive forward jumps. Equal totals
        // (= no movement) and downward corrections (rare; would mean
        // upstream rolled back) leave last_diff/_at untouched on the
        // pre-existing row.
        let (record_diff, record_diff_at) = if diff > 0 {
            (Some(diff), Some(Local::now()))
        } else {
            (None, None)
        };

        upsert_income(pool, profile.profile_id, record, record_diff, record_diff_at)
            .await?;
        written += 1;

        if diff > 0 {
            emailable.push(EmailableUpdate {
                profile: profile.clone(),
                diff,
                snapshot: record.data.clone(),
            });
        }
    }

    // ── Step 7: send diff emails grouped by recipient ──────────────
    if !emailable.is_empty() {
        match email_sender::load(pool).await {
            Ok(settings) if settings.validation_error().is_none() => {
                send_diff_emails(pool, &settings, &emailable).await;
            }
            Ok(_) => {
                tracing::info!(
                    "tomato_income: SMTP not configured, {} diff(s) skipped (last_diff still recorded)",
                    emailable.len()
                );
            }
            Err(e) => {
                tracing::warn!("tomato_income: load email_settings: {e}");
            }
        }
    }

    tracing::info!(
        "tomato_income: round done — fetched {}, written {}, emailable {}, upstream_max_lut={}",
        fetched.len(),
        written,
        emailable.len(),
        upstream_max_lut
    );
    Ok(written)
}

/// Per-profile data the round needs: the cookie header (for fetching)
/// PLUS the metadata (profile_name, owner email + username) needed to
/// route + render diff emails. Loaded once at the top of each round
/// so the email step doesn't need a follow-up SELECT per profile.
#[derive(Debug, Clone)]
struct ProfileWithCookie {
    profile_id: Uuid,
    profile_name: String,
    cookie_header: Arc<str>,
    /// Owner's `users.email`. None / empty means "fall back to admin
    /// `email_settings.recipients[0]`" at email-send time.
    owner_email: Option<String>,
    owner_username: String,
}

/// Pull all online tomato cookies + the metadata used by the email
/// step. Same predicate as `tomato_cookie::pick_cookie` (any active
/// user, not just admin) but JOINs in `browser_profiles.name` +
/// `users.email`/`users.username`.
async fn list_online_tomato_profiles(pool: &DbPool) -> Result<Vec<ProfileWithCookie>, String> {
    let rows = sqlx::query(
        r#"SELECT pkc.profile_id, pkc.cookies,
                  bp.name        AS profile_name,
                  u.email        AS owner_email,
                  u.username     AS owner_username
           FROM platform_kol_cookies pkc
           JOIN browser_profiles bp ON bp.id = pkc.profile_id
           JOIN users u             ON u.id = bp.user_id
           WHERE pkc.platform = 'tomato'
             AND pkc.domain   = 'kol.fanqieopen.com'
             AND pkc.is_online = TRUE
             AND u.is_active   = TRUE"#,
    )
    .fetch_all(pool)
    .await
    .map_err(|e| format!("list online tomato profiles: {e}"))?;

    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        let pid: Uuid = r
            .try_get("profile_id")
            .map_err(|e| format!("profile_id col: {e}"))?;
        let cookies: JsonValue = r
            .try_get("cookies")
            .map_err(|e| format!("cookies col: {e}"))?;
        let header = match cookies_to_header(&cookies) {
            Some(h) => h,
            None => continue,
        };
        out.push(ProfileWithCookie {
            profile_id: pid,
            profile_name: r.try_get("profile_name").unwrap_or_default(),
            cookie_header: Arc::from(header),
            owner_email: r.try_get("owner_email").ok().flatten(),
            owner_username: r.try_get("owner_username").unwrap_or_default(),
        });
    }
    Ok(out)
}

/// Re-implementation of `tomato_cookie::serialize_cookie_header`'s
/// logic locally (the helper there is private). Joins `name=value`
/// pairs from the JSONB cookie array into a single header string.
fn cookies_to_header(cookies: &JsonValue) -> Option<String> {
    let arr = cookies.as_array()?;
    let pairs: Vec<String> = arr
        .iter()
        .filter_map(|c| {
            let name = c.get("name")?.as_str()?;
            let value = c.get("value")?.as_str()?;
            if name.is_empty() {
                return None;
            }
            Some(format!("{name}={value}"))
        })
        .collect();
    if pairs.is_empty() {
        return None;
    }
    Some(pairs.join("; "))
}

/// `(profile_id, total_income)` for the rows we're about to upsert,
/// so we can compute diffs in-memory rather than per-row SELECTs.
async fn load_existing(
    pool: &DbPool,
    profile_ids: &[Uuid],
) -> Result<Vec<(Uuid, i64)>, String> {
    if profile_ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows = sqlx::query(
        "SELECT profile_id, total_income FROM kol_income WHERE profile_id = ANY($1::uuid[])",
    )
    .bind(profile_ids)
    .fetch_all(pool)
    .await
    .map_err(|e| format!("load existing income: {e}"))?;

    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        let pid: Uuid = r.try_get("profile_id").map_err(|e| format!("pid: {e}"))?;
        let total: i64 = r
            .try_get("total_income")
            .map_err(|e| format!("total: {e}"))?;
        out.push((pid, total));
    }
    Ok(out)
}

/// One UPSERT per profile. `last_diff` / `last_diff_at` are CASE'd so
/// rounds with no positive jump don't overwrite the previously
/// recorded diff (operator can still see "🆙 +¥X" from the last real
/// movement until the next forward jump).
async fn upsert_income(
    pool: &DbPool,
    profile_id: Uuid,
    record: &IncomeRecord,
    diff: Option<i64>,
    diff_at: Option<DateTime<Local>>,
) -> Result<(), String> {
    let lut = unix_to_local(record.data.latest_update_time);

    sqlx::query(
        r#"INSERT INTO kol_income (
              profile_id,
              total_income, regular_income, bonus_income,
              current_week_income, current_month_income,
              latest_update_time,
              weekly_income_list, monthly_income_list, task_income_list,
              raw,
              last_diff, last_diff_at,
              fetched_at
           ) VALUES (
              $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11,
              COALESCE($12, 0), $13, NOW()
           )
           ON CONFLICT (profile_id) DO UPDATE SET
              total_income          = EXCLUDED.total_income,
              regular_income        = EXCLUDED.regular_income,
              bonus_income          = EXCLUDED.bonus_income,
              current_week_income   = EXCLUDED.current_week_income,
              current_month_income  = EXCLUDED.current_month_income,
              latest_update_time    = EXCLUDED.latest_update_time,
              weekly_income_list    = EXCLUDED.weekly_income_list,
              monthly_income_list   = EXCLUDED.monthly_income_list,
              task_income_list      = EXCLUDED.task_income_list,
              raw                   = EXCLUDED.raw,
              last_diff             = COALESCE($12, kol_income.last_diff),
              last_diff_at          = COALESCE($13, kol_income.last_diff_at),
              fetched_at            = NOW()"#,
    )
    .bind(profile_id)
    .bind(record.data.total_income)
    .bind(record.data.regular_income)
    .bind(record.data.bonus_income)
    .bind(record.data.current_week_income)
    .bind(record.data.current_month_income)
    .bind(lut)
    .bind(&record.data.weekly_income_list)
    .bind(&record.data.monthly_income_list)
    .bind(&record.data.task_income_list)
    .bind(&record.raw)
    .bind(diff)
    .bind(diff_at)
    .execute(pool)
    .await
    .map_err(|e| format!("upsert income {profile_id}: {e}"))?;
    Ok(())
}

/// `0` (upstream "no income computed yet") → None; positive seconds →
/// Some(local timestamp).
fn unix_to_local(secs: i64) -> Option<DateTime<Local>> {
    if secs <= 0 {
        return None;
    }
    Local.timestamp_opt(secs, 0).single()
}

// ─────────────────────────── email pipeline ───────────────────────────

/// One profile's worth of "新增收益" data, queued for emailing after
/// the diff loop commits.
#[derive(Debug, Clone)]
struct EmailableUpdate {
    profile: ProfileWithCookie,
    /// Strictly positive (we only queue diffs > 0).
    diff: i64,
    snapshot: IncomeData,
}

/// Resolve email destination + group emailable rows by recipient, then
/// send one HTML digest per recipient. After successful SMTP ack:
///   * `last_emailed_at = NOW()` for each profile in the group
///   * `last_email_error = NULL` (clear any prior error)
/// On failure: `last_email_error = $smtp_err` (kept verbatim so the
/// admin panel hover-tooltip shows the cause); `last_emailed_at` is
/// NOT advanced, so the operator can retry by clearing the row OR
/// the next genuine diff will re-attempt.
async fn send_diff_emails(
    pool: &DbPool,
    settings: &EmailSettings,
    emailable: &[EmailableUpdate],
) {
    // Group by resolved recipient. Profiles whose owner has no email
    // and no admin fallback get bucketed under None; we record an
    // error on those rows but don't try to send.
    let mut by_recipient: HashMap<Option<String>, Vec<&EmailableUpdate>> = HashMap::new();
    for u in emailable {
        let recipient = resolve_recipient(&u.profile, settings);
        by_recipient.entry(recipient).or_default().push(u);
    }

    for (recipient, items) in by_recipient {
        let profile_ids: Vec<Uuid> = items.iter().map(|u| u.profile.profile_id).collect();

        let Some(addr) = recipient else {
            // Bucket with no resolvable email — persist error so admin
            // panel surfaces it; nothing to try.
            mark_email_error(
                pool,
                &profile_ids,
                "no recipient (owner email empty + no admin fallback)",
            )
            .await;
            continue;
        };

        let total_diff: i64 = items.iter().map(|u| u.diff).sum();
        let subject = format!(
            "番茄达人收益更新 · {} 账号 · +{}",
            items.len(),
            fmt_yuan(total_diff)
        );
        let body = render_diff_email_body(&items);

        match email_sender::send_html(settings, &[addr.clone()], &subject, &body).await {
            Ok(()) => {
                tracing::info!(
                    "tomato_income: emailed {} → {} accounts, +{}",
                    addr,
                    items.len(),
                    fmt_yuan(total_diff)
                );
                mark_emailed_ok(pool, &profile_ids).await;
            }
            Err(e) => {
                tracing::warn!(
                    "tomato_income: email send → {} failed: {e}",
                    addr
                );
                mark_email_error(pool, &profile_ids, &format!("smtp: {e}")).await;
            }
        }
    }
}

/// Owner.email → `email_settings.recipients[0]` → None.
fn resolve_recipient(profile: &ProfileWithCookie, settings: &EmailSettings) -> Option<String> {
    if let Some(addr) = profile
        .owner_email
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        return Some(addr);
    }
    settings.recipients.first().cloned()
}

async fn mark_emailed_ok(pool: &DbPool, profile_ids: &[Uuid]) {
    if profile_ids.is_empty() {
        return;
    }
    if let Err(e) = sqlx::query(
        r#"UPDATE kol_income
           SET last_emailed_at = NOW(),
               last_email_error = NULL
           WHERE profile_id = ANY($1::uuid[])"#,
    )
    .bind(profile_ids)
    .execute(pool)
    .await
    {
        tracing::warn!("tomato_income: mark_emailed_ok: {e}");
    }
}

async fn mark_email_error(pool: &DbPool, profile_ids: &[Uuid], reason: &str) {
    if profile_ids.is_empty() {
        return;
    }
    let trimmed: String = reason.chars().take(500).collect();
    if let Err(e) = sqlx::query(
        r#"UPDATE kol_income
           SET last_email_error = $1
           WHERE profile_id = ANY($2::uuid[])"#,
    )
    .bind(&trimmed)
    .bind(profile_ids)
    .execute(pool)
    .await
    {
        tracing::warn!("tomato_income: mark_email_error: {e}");
    }
}

/// Render the digest email body. Inline-styled HTML so it looks OK in
/// most clients without a stylesheet. Single table per recipient
/// listing every changed account with its diff + key counters.
fn render_diff_email_body(items: &[&EmailableUpdate]) -> String {
    use std::fmt::Write as _;
    let now_str = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let total_diff: i64 = items.iter().map(|u| u.diff).sum();

    let mut html = String::new();
    let _ = writeln!(
        html,
        r#"<!doctype html>
<html><body style="font-family: -apple-system, BlinkMacSystemFont, 'Helvetica Neue', sans-serif; color:#222; line-height:1.5;">
<h2 style="margin:0 0 12px;color:#1a73e8;">番茄达人收益更新</h2>
<p style="margin:0 0 8px;">检测到 <strong>{n}</strong> 个账号有新的收益结算,本次新增合计 <strong style="color:#0a8754;">+{total}</strong>。</p>
<table style="border-collapse:collapse;width:100%;margin:16px 0;font-size:13px;">
<thead><tr style="background:#f5f6f8;text-align:left;">
<th style="padding:8px;border:1px solid #e5e6e8;">账号</th>
<th style="padding:8px;border:1px solid #e5e6e8;">所属</th>
<th style="padding:8px;border:1px solid #e5e6e8;text-align:right;">本次新增</th>
<th style="padding:8px;border:1px solid #e5e6e8;text-align:right;">总收益</th>
<th style="padding:8px;border:1px solid #e5e6e8;text-align:right;">常规</th>
<th style="padding:8px;border:1px solid #e5e6e8;text-align:right;">激励</th>
<th style="padding:8px;border:1px solid #e5e6e8;text-align:right;">本月</th>
<th style="padding:8px;border:1px solid #e5e6e8;text-align:right;">本周</th>
</tr></thead>
<tbody>"#,
        n = items.len(),
        total = html_escape(&fmt_yuan(total_diff))
    );

    for u in items {
        let _ = writeln!(
            html,
            r#"<tr>
<td style="padding:8px;border:1px solid #e5e6e8;">{name}</td>
<td style="padding:8px;border:1px solid #e5e6e8;color:#666;">@{owner}</td>
<td style="padding:8px;border:1px solid #e5e6e8;text-align:right;color:#0a8754;font-weight:600;">+{diff}</td>
<td style="padding:8px;border:1px solid #e5e6e8;text-align:right;font-weight:600;">{total}</td>
<td style="padding:8px;border:1px solid #e5e6e8;text-align:right;color:#666;">{regular}</td>
<td style="padding:8px;border:1px solid #e5e6e8;text-align:right;color:#666;">{bonus}</td>
<td style="padding:8px;border:1px solid #e5e6e8;text-align:right;">{month}</td>
<td style="padding:8px;border:1px solid #e5e6e8;text-align:right;">{week}</td>
</tr>"#,
            name = html_escape(&u.profile.profile_name),
            owner = html_escape(&u.profile.owner_username),
            diff = html_escape(&fmt_yuan(u.diff)),
            total = html_escape(&fmt_yuan(u.snapshot.total_income)),
            regular = html_escape(&fmt_yuan(u.snapshot.regular_income)),
            bonus = html_escape(&fmt_yuan(u.snapshot.bonus_income)),
            month = html_escape(&fmt_yuan(u.snapshot.current_month_income)),
            week = html_escape(&fmt_yuan(u.snapshot.current_week_income)),
        );
    }

    let _ = writeln!(
        html,
        r#"</tbody></table>
<p style="color:#888;font-size:12px;margin:16px 0 0;">采集时间:{ts} · 数据来源:番茄达人 user/income/stats</p>
</body></html>"#,
        ts = html_escape(&now_str)
    );
    html
}

/// 分 → 元, 2 decimal places, prefixed with ¥.
fn fmt_yuan(cents: i64) -> String {
    format!("¥{:.2}", (cents as f64) / 100.0)
}

/// Minimal HTML-entity escape for user-controlled strings going into
/// the email body. Profile/user names are admin-curated so injection
/// risk is low, but still defend against `<` / `>` / `&` / `"` to
/// avoid breaking the surrounding markup.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

