//! Monthly 七猫达人 income notice forwarder.
//!
//! 七猫 doesn't expose a real-time income endpoint; it pushes a
//! "X月KOC七猫免费小说收益明细" notice into the user's site-wide
//! message feed (`/api/v1/message/notice/list`) once a month, usually
//! between days 10 and 20 of the following month. This job polls
//! that feed 3× a day on days 10–20, finds matching notices, and
//! forwards them as HTML email to the profile owner.
//!
//! Cron schedule (registered in `jobs::mod`):
//!   `0 0 9,13,21 10-20 * *`
//! 09:00 / 13:00 / 21:00 local (Asia/Shanghai), days 10 through 20
//! of every month. ~33 fires per month per profile; idempotency
//! comes from the `qimao_income_notice` PK `(profile_id, message_id)`,
//! so duplicate fires are safe.
//!
//! Per fire:
//!   1. List every active qimao profile (any user, not just admin) —
//!      same predicate as `qimao_alias_submitter`.
//!   2. Concurrent fetch (4-parallel — light op) of the 50 latest
//!      notices for each.
//!   3. Filter by `title.contains("七猫免费小说收益明细")`.
//!   4. For each match: skip if `(profile_id, message_id)` already
//!      in `qimao_income_notice` (already emailed). Otherwise:
//!      a. Resolve recipient: profile owner's `users.email` first,
//!         else `email_settings.recipients[0]` as admin fallback.
//!      b. Send HTML email via `email_sender::send_html`.
//!      c. INSERT into `qimao_income_notice` with `emailed_at=NOW()`
//!         (or `send_error` if SMTP failed — operator can re-send
//!         by deleting the row and waiting for the next fire).
//!   5. Token-401 → invalidate token + skip that profile (next
//!      `qimao_token_refresh` round will resign).

use std::sync::Arc;

use chrono::{DateTime, Local, NaiveDate};
use futures_util::stream::StreamExt;
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

use crate::db::DbPool;
use crate::services::email_sender::{self, EmailSettings};
use crate::services::email_template::{email_shell, notice_card};
use crate::services::qimao_account;
use crate::services::qimao_message::{
    build_http_client, list_notices, MessageItem, ENDPOINT_NOTICE_LIST, SERVICE_NAME,
};

/// Title substring marking the monthly income notice. The platform's
/// titles look like "1月KOC七猫免费小说收益明细" / "12月KOC七猫
/// 免费小说收益明细" — searching for this substring captures all
/// month variants without hard-coding the digit prefix.
const INCOME_TITLE_MARKER: &str = "七猫免费小说收益明细";

/// Concurrent fetches per fire. 4 is conservative — the message feed
/// is light traffic and only fires 3× a day on 11 days/month, so
/// even with 50 profiles total the round finishes quickly.
const FETCH_CONCURRENCY: usize = 4;

/// Single-pass entrypoint called from the cron scheduler in
/// `jobs::mod::start`. Returns Err only on infrastructure errors that
/// the operator should triage (DB unavailable, SMTP misconfigured,
/// etc.); per-profile failures are swallowed + logged so one bad
/// account doesn't block the round.
pub async fn run(pool: &DbPool) -> Result<(), String> {
    let settings = email_sender::load(pool).await?;
    if let Some(err) = settings.validation_error() {
        tracing::warn!(
            "qimao_income_notice: SMTP not configured ({err}), skipping round"
        );
        return Ok(());
    }

    let profiles = list_active_qimao_profiles(pool).await?;
    if profiles.is_empty() {
        tracing::info!("qimao_income_notice: no active qimao profiles, skip");
        return Ok(());
    }

    let http = build_http_client()?;

    // Shared collector for the admin consolidated digest. Each
    // newly-emailed (or even failed-to-email-owner) notice gets a row
    // here so the admin sees the full picture at the end of the round.
    // `Mutex` over Vec because handle_profile runs concurrently via
    // buffer_unordered.
    let admin_collector: std::sync::Arc<tokio::sync::Mutex<Vec<EmailedNotice>>> =
        std::sync::Arc::new(tokio::sync::Mutex::new(Vec::new()));

    futures_util::stream::iter(profiles)
        .map(|p| {
            let pool = pool.clone();
            let http = http.clone();
            let settings = settings.clone();
            let collector = admin_collector.clone();
            async move {
                handle_profile(&pool, &http, &settings, p, collector).await
            }
        })
        .buffer_unordered(FETCH_CONCURRENCY)
        .for_each(|_| async {})
        .await;

    // ── Admin consolidated digest ──────────────────────────────────
    // Drain everything queued by handle_notice. Since `for_each` above
    // joined every spawned task, all Arc clones have dropped by now —
    // we just need the contents.
    let collected: Vec<EmailedNotice> = {
        let mut g = admin_collector.lock().await;
        std::mem::take(&mut *g)
    };
    if !collected.is_empty() {
        send_admin_consolidated_digest(pool, &settings, &collected).await;
    }

    Ok(())
}

/// One newly-emailed (or attempted) notice queued for the admin digest.
/// Cloneable so the Mutex fallback path can copy out without consuming.
#[derive(Debug, Clone)]
struct EmailedNotice {
    profile_name: String,
    owner_username: String,
    title: String,
    content_html: String,
    notice_date: Option<NaiveDate>,
}

#[derive(Debug, Clone)]
struct ProfileInfo {
    profile_id: Uuid,
    profile_name: String,
    token: String,
    /// Owner's `users.notify_emails` list. Empty = owner didn't configure
    /// → fall back to `email_settings.recipients` at email-send time.
    owner_emails: Vec<String>,
    owner_username: String,
}

async fn list_active_qimao_profiles(pool: &DbPool) -> Result<Vec<ProfileInfo>, String> {
    let rows = sqlx::query(
        r#"SELECT bp.id              AS profile_id,
                  bp.name            AS profile_name,
                  bp.qimao_token     AS token,
                  u.notify_emails    AS owner_emails,
                  u.username         AS owner_username
           FROM browser_profiles bp
           JOIN users u ON u.id = bp.user_id
           WHERE bp.kol_platform = 'qimao'
             AND bp.qimao_token IS NOT NULL
             AND bp.qimao_token <> ''
             AND u.is_active = TRUE"#,
    )
    .fetch_all(pool)
    .await
    .map_err(|e| format!("list qimao profiles: {e}"))?;

    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        let emails_json: serde_json::Value = r
            .try_get::<serde_json::Value, _>("owner_emails")
            .unwrap_or_else(|_| serde_json::json!([]));
        let owner_emails: Vec<String> =
            serde_json::from_value(emails_json).unwrap_or_default();
        out.push(ProfileInfo {
            profile_id: r.try_get("profile_id").map_err(|e| format!("pid: {e}"))?,
            profile_name: r.try_get("profile_name").unwrap_or_default(),
            token: r.try_get("token").map_err(|e| format!("token: {e}"))?,
            owner_emails,
            owner_username: r.try_get("owner_username").unwrap_or_default(),
        });
    }
    Ok(out)
}

async fn handle_profile(
    pool: &DbPool,
    http: &reqwest_middleware::ClientWithMiddleware,
    settings: &EmailSettings,
    profile: ProfileInfo,
    admin_collector: std::sync::Arc<tokio::sync::Mutex<Vec<EmailedNotice>>>,
) {
    // Fetch the latest 50 notices.
    let outcome = list_notices(http, &profile.token).await;
    let request_summary = json!({
        "profile_id": profile.profile_id,
        "page_size": 50,
    });
    let notices = match outcome
        .audit(pool, SERVICE_NAME, ENDPOINT_NOTICE_LIST, request_summary)
        .await
    {
        Ok(v) => v,
        Err(err) if err.is_auth_failure() => {
            qimao_account::recover_or_offline(
                pool,
                http,
                profile.profile_id,
                &format!("notice_list: {err}"),
            )
            .await;
            tracing::warn!(
                "qimao_income_notice: token auth failed profile={} {err}",
                profile.profile_id
            );
            return;
        }
        Err(err) => {
            tracing::warn!(
                "qimao_income_notice: fetch failed profile={} {err}",
                profile.profile_id
            );
            return;
        }
    };

    // Filter to income notices.
    let income_notices: Vec<&MessageItem> = notices
        .iter()
        .filter(|m| m.title.contains(INCOME_TITLE_MARKER))
        .collect();
    if income_notices.is_empty() {
        return;
    }

    // For each: skip if already in qimao_income_notice; else email + insert.
    for n in income_notices {
        match handle_notice(pool, settings, &profile, n, &admin_collector).await {
            Ok(true) => tracing::info!(
                "qimao_income_notice: processed profile={} message_id={} title={:?}",
                profile.profile_id,
                n.id,
                n.title
            ),
            Ok(false) => {} // already emailed; silent
            Err(e) => tracing::warn!(
                "qimao_income_notice: handle profile={} message_id={}: {e}",
                profile.profile_id,
                n.id
            ),
        }
    }
}

/// Returns `Ok(true)` if this is a NEW notice (regardless of whether
/// owner-side email succeeded), `Ok(false)` if it was already in the
/// dedup table. New notices also get queued into `admin_collector` so
/// they show up in the consolidated admin digest at the end of the
/// round even when the per-owner email leg failed.
async fn handle_notice(
    pool: &DbPool,
    settings: &EmailSettings,
    profile: &ProfileInfo,
    notice: &MessageItem,
    admin_collector: &std::sync::Arc<tokio::sync::Mutex<Vec<EmailedNotice>>>,
) -> Result<bool, String> {
    // Dedup check. PK (profile_id, message_id) → existence = already
    // emailed. We don't UPDATE here — just skip.
    let already: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM qimao_income_notice WHERE profile_id = $1 AND message_id = $2)",
    )
    .bind(profile.profile_id)
    .bind(notice.id)
    .fetch_one(pool)
    .await
    .map_err(|e| format!("dedup check: {e}"))?;
    if already {
        return Ok(false);
    }

    // Resolve recipients: owner's notify_emails list → admin fallback.
    let recipients = resolve_recipients(profile, settings);
    // recipient_email column is TEXT — store comma-joined for the
    // admin panel's "已发到" display. Empty = no one to send to.
    let recipient_display: Option<String> = if recipients.is_empty() {
        None
    } else {
        Some(recipients.join(", "))
    };

    // Parse upstream's date string. Tolerant — failure just nulls out
    // the column, doesn't block the email.
    let notice_date: Option<NaiveDate> =
        NaiveDate::parse_from_str(&notice.create_time, "%Y-%m-%d").ok();

    let (emailed_at, send_error): (Option<DateTime<Local>>, Option<String>) = if recipients
        .is_empty()
    {
        (None, Some("no recipient (owner notify_emails empty + no admin fallback)".into()))
    } else {
        // 上游 (七猫) 推下来的 content 是一段独立 HTML — 我们把它
        // 嵌进自己的移动端友好 shell,顶部加上账号 + 通知日期的
        // 上下文卡,让用户在手机上一眼就知道是哪个 profile。
        let date_str = notice_date
            .map(|d| d.format("%Y-%m-%d").to_string())
            .unwrap_or_else(|| "—".to_string());
        let secondary = format!(
            "账号:{} · 通知日期:{}",
            profile.profile_name, date_str
        );
        let body = email_shell(
            &notice.title,
            Some(&secondary),
            &notice_card(
                &notice.title,
                Some("以下为七猫达人推送的原始通知"),
                &notice.content,
            ),
            Some("收到这封邮件意味着你的本月七猫达人收益已结算"),
        );

        match email_sender::send_html(settings, &recipients, &notice.title, &body).await {
            Ok(()) => (Some(Local::now()), None),
            Err(e) => (None, Some(format!("smtp: {e}"))),
        }
    };

    sqlx::query(
        r#"INSERT INTO qimao_income_notice (
              profile_id, message_id, title, content_html,
              notice_date, recipient_email, emailed_at, send_error
           ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
           ON CONFLICT (profile_id, message_id) DO NOTHING"#,
    )
    .bind(profile.profile_id)
    .bind(notice.id)
    .bind(&notice.title)
    .bind(&notice.content)
    .bind(notice_date)
    .bind(&recipient_display)
    .bind(emailed_at)
    .bind(&send_error)
    .execute(pool)
    .await
    .map_err(|e| format!("insert notice {}: {e}", notice.id))?;

    // Queue for admin digest BEFORE early-returning on send_error so
    // admins always see new notices even when owner delivery failed.
    {
        let mut g = admin_collector.lock().await;
        g.push(EmailedNotice {
            profile_name: profile.profile_name.clone(),
            owner_username: profile.owner_username.clone(),
            title: notice.title.clone(),
            content_html: notice.content.clone(),
            notice_date,
        });
    }

    if let Some(err) = send_error {
        return Err(err);
    }
    Ok(true)
}

/// Resolve email recipients: owner's full `notify_emails` list →
/// `email_settings.recipients` (admin fallback) → empty list.
/// Empty = no one to email; caller persists `send_error` for that case.
fn resolve_recipients(profile: &ProfileInfo, settings: &EmailSettings) -> Vec<String> {
    if !profile.owner_emails.is_empty() {
        return profile.owner_emails.clone();
    }
    settings
        .recipients
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Send ONE consolidated digest to all admin recipients summarizing
/// every new notice processed this round (across every user). Embeds
/// each notice's full HTML body separated by horizontal rules; admins
/// see exactly what each user saw, in one inbox item.
///
/// Skipped silently when no admin recipients are configured.
async fn send_admin_consolidated_digest(
    pool: &DbPool,
    settings: &EmailSettings,
    notices: &[EmailedNotice],
) {
    let recipients = email_sender::resolve_admin_recipients(pool, settings).await;
    if recipients.is_empty() {
        tracing::info!(
            "qimao_income_notice: no admin recipients configured, skip consolidated digest ({} notices)",
            notices.len()
        );
        return;
    }

    let subject = format!(
        "[管理员速览] 七猫达人收益通知 · {} 条 · {} 位用户",
        notices.len(),
        notices
            .iter()
            .map(|n| n.owner_username.as_str())
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
    );
    let body = render_admin_digest_body(notices);

    match email_sender::send_html(settings, &recipients, &subject, &body).await {
        Ok(()) => tracing::info!(
            "qimao_income_notice: admin digest sent to {} recipient(s), {} notice(s)",
            recipients.len(),
            notices.len()
        ),
        Err(e) => tracing::warn!(
            "qimao_income_notice: admin consolidated digest failed: {e}"
        ),
    }
}

/// Render the admin digest body using the shared mobile-friendly
/// template. Each notice becomes a notice_card whose header shows
/// "@owner · profile · date" and whose body embeds the upstream's
/// HTML verbatim (the qimao platform's own template — trusted).
fn render_admin_digest_body(notices: &[EmailedNotice]) -> String {
    let now_str = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let user_count = notices
        .iter()
        .map(|x| x.owner_username.as_str())
        .collect::<std::collections::BTreeSet<_>>()
        .len();

    let title = "七猫达人收益通知速览";
    let subtitle = format!(
        "本轮收到 {} 条新通知,涉及 {} 位用户",
        notices.len(),
        user_count,
    );

    let mut content = String::new();
    for n in notices {
        let date_str = n
            .notice_date
            .map(|d| d.format("%Y-%m-%d").to_string())
            .unwrap_or_else(|| "—".to_string());
        let secondary = format!(
            "@{} · 账号:{} · 通知日期:{}",
            n.owner_username, n.profile_name, date_str,
        );
        content.push_str(&notice_card(
            &n.title,
            Some(&secondary),
            &n.content_html, // platform-trusted upstream HTML, embedded as-is
        ));
    }

    let footer = format!("检查时间:{} · 数据来源:七猫达人 message/notice/list", now_str);
    email_shell(title, Some(&subtitle), &content, Some(&footer))
}
