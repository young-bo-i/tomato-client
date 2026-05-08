//! Long-running worker that emails admins when one of their KOL
//! accounts goes offline.
//!
//! Per-admin trigger semantics (per spec):
//!   * One admin's email goes out **only** when the admin has at
//!     least one *newly-offline* profile (i.e. offline + notified_at
//!     IS NULL).
//!   * The email body lists **every currently-offline** profile owned
//!     by the admin — both freshly-offline and previously-notified-
//!     but-not-yet-recovered. So the admin always sees the full
//!     "needs re-login" picture, not just deltas.
//!   * After sending, every freshly-offline row is stamped
//!     `notified_at=NOW`. Re-sending only happens after the row
//!     recovers (which clears the flag) and goes offline again.
//!
//! Two offline sources, one dispatcher:
//!   * **番茄**: `platform_kol_cookies` — `is_online=FALSE` plus the
//!     `offline_notified_at` flag. Recovery hook is in
//!     `api::profile_state` (re-pushed cookies clear the flag).
//!   * **抖音**: `browser_profiles.douyin_login_state =
//!     'unauthenticated'` plus the `douyin_offline_notified_at` flag.
//!     Recovery hook is in `api::profiles::set_douyin_state` (state
//!     transitioning back to `authenticated` clears the flag).
//!
//! Recipients come from the `users.email` of each profile owner.
//! Admins without an email are skipped with a debug log — the rest
//! still get notified.
//!
//! SMTP credentials come from `email_settings`; if the SMTP host
//! isn't configured, the worker logs once and idles until config is
//! set. (No retries — the next round will retry naturally.)

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Local};
use sqlx::Row;
use uuid::Uuid;

use crate::db::DbPool;
use crate::services::email_sender;

const POLL_INTERVAL: Duration = Duration::from_secs(60);

pub async fn start_worker(pool: Arc<DbPool>) {
    tracing::info!("notification_dispatcher: worker starting");
    let mut tick = tokio::time::interval(POLL_INTERVAL);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    tick.tick().await;
    loop {
        tick.tick().await;
        match dispatch_round(&pool).await {
            Ok(0) => {}
            Ok(n) => tracing::info!("notification_dispatcher: sent {n} email(s)"),
            Err(e) => tracing::warn!("notification_dispatcher: round failed: {e}"),
        }
    }
}

async fn dispatch_round(pool: &DbPool) -> Result<usize, String> {
    // Single batch fetch: all currently-offline rows for all active
    // admins, joined with username + email. Group by user_id in Rust.
    // Replaces the previous 1 + 4N queries (scan + per-user) with 2.
    let mut groups = collect_offline_groups(pool).await?;
    if groups.is_empty() {
        return Ok(0);
    }

    // Only users with at least one *new* offline trigger an email.
    // (Already-notified rows still ride along in the body.)
    let users_with_fresh: Vec<i32> = groups
        .iter()
        .filter(|(_, g)| g.rows.iter().any(|r| r.is_new))
        .map(|(uid, _)| *uid)
        .collect();
    if users_with_fresh.is_empty() {
        return Ok(0);
    }

    let settings = email_sender::load(pool).await?;
    if settings.validation_error().is_some() {
        tracing::debug!(
            "notification_dispatcher: skipping ({} admin(s) waiting), SMTP not configured",
            users_with_fresh.len()
        );
        return Ok(0);
    }

    let mut sent = 0usize;
    for user_id in users_with_fresh {
        let Some(group) = groups.remove(&user_id) else { continue };
        match handle_user_with_data(pool, &settings, user_id, group).await {
            Ok(true) => sent += 1,
            Ok(false) => {}
            Err(e) => tracing::warn!(
                "notification_dispatcher: user {user_id} failed: {e}"
            ),
        }
    }
    Ok(sent)
}

/// Collected offline rows for one user, plus their notification address.
struct UserOfflineGroup {
    username: String,
    email: Option<String>,
    rows: Vec<OfflineRow>,
}

/// Fetch all (user, offline_row) pairs in two queries (cookies + douyin),
/// joined with users so username/email come along for free. Group in
/// Rust by user_id. Inactive users are filtered server-side.
async fn collect_offline_groups(
    pool: &DbPool,
) -> Result<HashMap<i32, UserOfflineGroup>, String> {
    let mut groups: HashMap<i32, UserOfflineGroup> = HashMap::new();

    // Cookie offlines (covers both tomato and qimao platform rows).
    let cookie_rows = sqlx::query(
        r#"SELECT bp.user_id, u.username, u.email,
                  pkc.profile_id, bp.name AS profile_name,
                  pkc.platform, pkc.domain,
                  pkc.last_offline_at, pkc.offline_reason,
                  pkc.offline_notified_at
           FROM platform_kol_cookies pkc
           JOIN browser_profiles bp ON bp.id = pkc.profile_id
           JOIN users u             ON u.id = bp.user_id
           WHERE pkc.is_online = FALSE
             AND u.is_active = TRUE
           ORDER BY bp.user_id, pkc.last_offline_at DESC NULLS LAST"#,
    )
    .fetch_all(pool)
    .await
    .map_err(|e| format!("batch cookie offlines: {e}"))?;

    for r in cookie_rows {
        let uid: i32 = r.try_get("user_id").map_err(|e| format!("user_id col: {e}"))?;
        let username: String = r.try_get("username").unwrap_or_default();
        let email: Option<String> = r.try_get("email").unwrap_or(None);
        let platform: String = r.try_get("platform").unwrap_or_default();
        let domain: String = r.try_get("domain").unwrap_or_default();
        let kind = match platform.as_str() {
            "tomato" => OfflineKind::TomatoCookie { platform: "tomato" },
            "qimao" => OfflineKind::QimaoCookie,
            _ => continue,
        };
        let notified_at: Option<DateTime<Local>> =
            r.try_get("offline_notified_at").unwrap_or(None);
        let entry = groups.entry(uid).or_insert(UserOfflineGroup {
            username,
            email,
            rows: Vec::new(),
        });
        entry.rows.push(OfflineRow {
            profile_id: r.try_get("profile_id").unwrap_or_default(),
            profile_name: r.try_get("profile_name").unwrap_or_default(),
            platform: human_label(&platform).to_string(),
            detail_label: Some(domain),
            last_offline_at: r.try_get("last_offline_at").unwrap_or(None),
            reason: r.try_get("offline_reason").unwrap_or(None),
            is_new: notified_at.is_none(),
            kind,
        });
    }

    // Douyin profile offlines.
    let douyin_rows = sqlx::query(
        r#"SELECT bp.user_id, u.username, u.email,
                  bp.id AS profile_id, bp.name AS profile_name,
                  bp.douyin_login_state_updated_at,
                  bp.douyin_login_state_url,
                  bp.douyin_offline_notified_at
           FROM browser_profiles bp
           JOIN users u ON u.id = bp.user_id
           WHERE bp.kol_platform = 'douyin'
             AND bp.douyin_login_state = 'unauthenticated'
             AND u.is_active = TRUE
           ORDER BY bp.user_id, bp.douyin_login_state_updated_at DESC NULLS LAST"#,
    )
    .fetch_all(pool)
    .await
    .map_err(|e| format!("batch douyin offlines: {e}"))?;

    for r in douyin_rows {
        let uid: i32 = r.try_get("user_id").map_err(|e| format!("user_id col: {e}"))?;
        let username: String = r.try_get("username").unwrap_or_default();
        let email: Option<String> = r.try_get("email").unwrap_or(None);
        let notified_at: Option<DateTime<Local>> =
            r.try_get("douyin_offline_notified_at").unwrap_or(None);
        let entry = groups.entry(uid).or_insert(UserOfflineGroup {
            username,
            email,
            rows: Vec::new(),
        });
        entry.rows.push(OfflineRow {
            profile_id: r.try_get("profile_id").unwrap_or_default(),
            profile_name: r.try_get("profile_name").unwrap_or_default(),
            platform: "抖音".to_string(),
            detail_label: r.try_get("douyin_login_state_url").unwrap_or(None),
            last_offline_at: r.try_get("douyin_login_state_updated_at").unwrap_or(None),
            reason: None,
            is_new: notified_at.is_none(),
            kind: OfflineKind::DouyinProfile,
        });
    }

    Ok(groups)
}

#[derive(Debug)]
struct OfflineRow {
    profile_id: Uuid,
    profile_name: String,
    /// "tomato" / "qimao" (a tomato cookie row may be qimao too) /
    /// "douyin"
    platform: String,
    /// "tomato:kol.fanqieopen.com" etc — disambiguates same profile
    /// on multiple cookie domains.
    detail_label: Option<String>,
    last_offline_at: Option<DateTime<Local>>,
    /// Reason text from the upstream (HTTP status, error reason, etc.).
    reason: Option<String>,
    /// True for rows that are about to be marked notified by THIS
    /// round (newly offline). False for rows already notified-but-
    /// not-yet-recovered, which we still include in the email body
    /// so the admin sees the full picture.
    is_new: bool,
    /// "tomato_cookie" or "douyin_profile" — used by the post-send
    /// stamping path to update the right column.
    kind: OfflineKind,
}

#[derive(Debug, Clone, Copy)]
enum OfflineKind {
    TomatoCookie {
        platform: &'static str,
    },
    QimaoCookie,
    DouyinProfile,
}

async fn handle_user_with_data(
    pool: &DbPool,
    settings: &email_sender::EmailSettings,
    user_id: i32,
    group: UserOfflineGroup,
) -> Result<bool, String> {
    let UserOfflineGroup { username, email, rows } = group;
    let Some(email) = email.filter(|e| !e.trim().is_empty()) else {
        tracing::debug!(
            "notification_dispatcher: user {user_id} ({username}) has no email, skip"
        );
        return Ok(false);
    };
    if rows.is_empty() || !rows.iter().any(|r| r.is_new) {
        // Caller already filtered users_with_fresh, but defensive check.
        return Ok(false);
    }

    let subject = format!("Tomato KOL · 账号掉线提醒 ({} 个)", rows.len());
    let body = render_body(&username, &rows);

    email_sender::send(settings, &[email.clone()], &subject, &body)
        .await
        .map_err(|e| format!("smtp send to {email}: {e}"))?;

    stamp_notified(pool, &rows).await?;

    tracing::info!(
        "notification_dispatcher: emailed user {user_id} ({username}) → {email} · {} offline row(s)",
        rows.len()
    );
    Ok(true)
}

fn human_label(platform: &str) -> &'static str {
    match platform {
        "tomato" => "番茄",
        "qimao" => "七猫",
        "douyin" => "抖音",
        _ => "未知",
    }
}

fn render_body(username: &str, rows: &[OfflineRow]) -> String {
    use std::fmt::Write as _;
    let mut buf = String::new();
    let _ = writeln!(
        buf,
        "您好 {username},\n\n以下 KOL 账号已掉线,需要重新登录:\n"
    );

    // Group by platform label for readability.
    let mut by_platform: HashMap<&str, Vec<&OfflineRow>> = HashMap::new();
    for r in rows {
        by_platform.entry(r.platform.as_str()).or_default().push(r);
    }
    let mut groups: Vec<&str> = by_platform.keys().copied().collect();
    groups.sort();

    for g in groups {
        let _ = writeln!(buf, "【{}】", g);
        for r in by_platform[g].iter() {
            let when = r
                .last_offline_at
                .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
                .unwrap_or_else(|| "—".to_string());
            let reason_part = r
                .reason
                .as_deref()
                .filter(|s| !s.is_empty())
                .map(|s| format!("   原因: {s}"))
                .unwrap_or_default();
            let detail_part = r
                .detail_label
                .as_deref()
                .filter(|s| !s.is_empty())
                .map(|s| format!("   ({s})"))
                .unwrap_or_default();
            let new_marker = if r.is_new { " [NEW]" } else { "" };
            let _ = writeln!(
                buf,
                "  · {}{}   最近掉线: {}{}{}",
                r.profile_name, new_marker, when, detail_part, reason_part
            );
        }
        buf.push('\n');
    }

    let _ = writeln!(
        buf,
        "请在 Donut 客户端重新登录这些账号。账号恢复后下次再次掉线会触发新的提醒。"
    );
    buf
}

async fn stamp_notified(pool: &DbPool, rows: &[OfflineRow]) -> Result<(), String> {
    // Three buckets, one statement each. Only stamp rows that were
    // newly offline this round; old ones were already stamped in a
    // prior round and stay as-is so the recovery clear-flag pattern
    // works.
    let mut tomato_ids: Vec<Uuid> = Vec::new();
    let mut qimao_ids: Vec<Uuid> = Vec::new();
    let mut douyin_ids: Vec<Uuid> = Vec::new();
    for r in rows.iter().filter(|r| r.is_new) {
        match r.kind {
            OfflineKind::TomatoCookie { .. } => tomato_ids.push(r.profile_id),
            OfflineKind::QimaoCookie => qimao_ids.push(r.profile_id),
            OfflineKind::DouyinProfile => douyin_ids.push(r.profile_id),
        }
    }

    if !tomato_ids.is_empty() {
        sqlx::query(
            r#"UPDATE platform_kol_cookies
               SET offline_notified_at = NOW()
               WHERE platform = 'tomato'
                 AND profile_id = ANY($1::uuid[])
                 AND is_online = FALSE
                 AND offline_notified_at IS NULL"#,
        )
        .bind(&tomato_ids)
        .execute(pool)
        .await
        .map_err(|e| format!("stamp tomato: {e}"))?;
    }
    if !qimao_ids.is_empty() {
        sqlx::query(
            r#"UPDATE platform_kol_cookies
               SET offline_notified_at = NOW()
               WHERE platform = 'qimao'
                 AND profile_id = ANY($1::uuid[])
                 AND is_online = FALSE
                 AND offline_notified_at IS NULL"#,
        )
        .bind(&qimao_ids)
        .execute(pool)
        .await
        .map_err(|e| format!("stamp qimao: {e}"))?;
    }
    if !douyin_ids.is_empty() {
        sqlx::query(
            r#"UPDATE browser_profiles
               SET douyin_offline_notified_at = NOW()
               WHERE id = ANY($1::uuid[])
                 AND douyin_login_state = 'unauthenticated'
                 AND douyin_offline_notified_at IS NULL"#,
        )
        .bind(&douyin_ids)
        .execute(pool)
        .await
        .map_err(|e| format!("stamp douyin: {e}"))?;
    }
    Ok(())
}
