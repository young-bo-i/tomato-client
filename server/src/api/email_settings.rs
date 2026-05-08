//! Admin-only CRUD for the SMTP / notification email settings, plus
//! a "send test email" hatch so the operator can verify the config
//! without waiting for a real notification.
//!
//! Single-row design: the table is seeded with id=1 by migration 021.
//! `GET` always returns something (defaults if never configured),
//! `PUT` upserts in place. The password is never returned in `GET`
//! responses — the API surfaces an `is_password_set` boolean instead
//! so the form can show "**** (saved)" without exposing it.

use actix_web::{web, HttpResponse};
use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as JsonValue};
use sqlx::Row;

use crate::auth::AdminUser;
use crate::db::DbPool;
use crate::errors::AppResult;
use crate::services::email_sender;

/// Public-facing email settings shape (password redacted).
#[derive(Debug, Serialize)]
pub struct EmailSettingsView {
    pub smtp_host: String,
    pub smtp_port: i32,
    pub smtp_username: String,
    /// True when a non-empty password is stored on the server. The
    /// actual value is never returned.
    pub is_password_set: bool,
    pub from_address: String,
    pub from_name: String,
    pub use_tls: bool,
    pub recipients: Vec<String>,
    pub updated_at: DateTime<Local>,
}

/// `GET /api/admin/email_settings` — fetch the current configuration.
/// Always returns 200 with defaults if never configured (the row is
/// seeded by migration 021).
pub async fn get(pool: web::Data<DbPool>, _: AdminUser) -> AppResult<HttpResponse> {
    let row = sqlx::query(
        r#"SELECT smtp_host, smtp_port, smtp_username, smtp_password,
                  from_address, from_name, use_tls, recipients, updated_at
           FROM email_settings WHERE id = 1"#,
    )
    .fetch_one(pool.get_ref())
    .await?;

    let recipients: JsonValue = row.try_get("recipients").unwrap_or(JsonValue::Array(vec![]));
    let recipients_vec: Vec<String> = recipients
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let smtp_password: String = row.try_get("smtp_password").unwrap_or_default();

    Ok(HttpResponse::Ok().json(EmailSettingsView {
        smtp_host: row.try_get("smtp_host").unwrap_or_default(),
        smtp_port: row.try_get("smtp_port").unwrap_or(587),
        smtp_username: row.try_get("smtp_username").unwrap_or_default(),
        is_password_set: !smtp_password.is_empty(),
        from_address: row.try_get("from_address").unwrap_or_default(),
        from_name: row.try_get("from_name").unwrap_or_default(),
        use_tls: row.try_get("use_tls").unwrap_or(true),
        recipients: recipients_vec,
        updated_at: row.try_get("updated_at")?,
    }))
}

#[derive(Debug, Deserialize)]
pub struct UpdateBody {
    pub smtp_host: String,
    pub smtp_port: i32,
    pub smtp_username: String,
    /// Optional. If omitted (`null` / missing), the existing password
    /// stays unchanged — lets the UI submit the form without re-typing
    /// the password every time. Empty string explicitly clears it.
    #[serde(default)]
    pub smtp_password: Option<String>,
    pub from_address: String,
    #[serde(default)]
    pub from_name: String,
    pub use_tls: bool,
    #[serde(default)]
    pub recipients: Vec<String>,
}

/// `PUT /api/admin/email_settings` — replace the (single) settings
/// row. Password handling: omit / null = preserve existing, "" =
/// explicitly clear, anything else = set new.
pub async fn put(
    pool: web::Data<DbPool>,
    body: web::Json<UpdateBody>,
    _: AdminUser,
) -> AppResult<HttpResponse> {
    let b = body.into_inner();
    let recipients_json = serde_json::to_value(&b.recipients).unwrap_or(JsonValue::Array(vec![]));

    // Two SQL paths: with password or without. Branched explicitly so
    // "absent (omit field) preserves existing" stays distinct from
    // "explicit empty string clears" — COALESCE can't express that.
    let updated = if let Some(pw) = b.smtp_password {
        sqlx::query(
            r#"UPDATE email_settings
               SET smtp_host=$1, smtp_port=$2, smtp_username=$3,
                   smtp_password=$4,
                   from_address=$5, from_name=$6, use_tls=$7,
                   recipients=$8, updated_at=NOW()
               WHERE id=1"#,
        )
        .bind(&b.smtp_host)
        .bind(b.smtp_port)
        .bind(&b.smtp_username)
        .bind(&pw)
        .bind(&b.from_address)
        .bind(&b.from_name)
        .bind(b.use_tls)
        .bind(&recipients_json)
        .execute(pool.get_ref())
        .await?
        .rows_affected()
    } else {
        sqlx::query(
            r#"UPDATE email_settings
               SET smtp_host=$1, smtp_port=$2, smtp_username=$3,
                   from_address=$4, from_name=$5, use_tls=$6,
                   recipients=$7, updated_at=NOW()
               WHERE id=1"#,
        )
        .bind(&b.smtp_host)
        .bind(b.smtp_port)
        .bind(&b.smtp_username)
        .bind(&b.from_address)
        .bind(&b.from_name)
        .bind(b.use_tls)
        .bind(&recipients_json)
        .execute(pool.get_ref())
        .await?
        .rows_affected()
    };

    if updated == 0 {
        // Should be unreachable: migration 021 seeds the row.
        return Ok(HttpResponse::InternalServerError().json(json!({
            "ok": false, "error": "settings row missing"
        })));
    }
    Ok(HttpResponse::Ok().json(json!({ "ok": true })))
}

#[derive(Debug, Deserialize)]
pub struct TestBody {
    /// Optional explicit recipient. When omitted, send to the first
    /// configured recipient (`recipients[0]`).
    #[serde(default)]
    pub to: Option<String>,
}

/// `POST /api/admin/email_settings/test` — synchronously send a one-off
/// test email so the operator gets immediate feedback on whether the
/// SMTP config actually works.
///
/// Returns `502` with the SMTP error verbatim when the send fails.
/// `400` when there's no usable recipient.
pub async fn send_test(
    pool: web::Data<DbPool>,
    body: web::Json<TestBody>,
    _: AdminUser,
) -> AppResult<HttpResponse> {
    let settings = email_sender::load(pool.get_ref())
        .await
        .map_err(|e| crate::errors::AppError::BadRequest(format!("load settings: {e}")))?;

    let to_addr = body
        .to
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| settings.recipients.first().cloned());
    let Some(to_addr) = to_addr else {
        return Ok(HttpResponse::BadRequest().json(json!({
            "ok": false,
            "error": "no recipient: pass {\"to\": \"...\"} or configure default recipients"
        })));
    };

    // 提前抓常见配置坑,避免用户陷在"SMTP 接受了但邮件不到"的迷局里。
    // 网易 163 / QQ / Gmail 都要求 From 头的地址必须等于已认证的 SMTP
    // 账号,否则服务商内部丢弃邮件(SMTP 不报错)。
    let mut warnings: Vec<String> = Vec::new();
    let host_lower = settings.smtp_host.trim().to_lowercase();
    let from_lower = settings.from_address.trim().to_lowercase();
    let user_lower = settings.smtp_username.trim().to_lowercase();
    let strict_provider = host_lower.contains("163.com")
        || host_lower.contains("qq.com")
        || host_lower.contains("126.com")
        || host_lower.contains("gmail.com")
        || host_lower.contains("aliyun.com")
        || host_lower.contains("sina.com");
    if strict_provider && !from_lower.is_empty() && !user_lower.is_empty() && from_lower != user_lower {
        warnings.push(format!(
            "from_address ({}) 与 smtp_username ({}) 不一致 — {} 强制要求一致,否则邮件会被服务商静默丢弃。建议把 from_address 改成 {}。",
            settings.from_address, settings.smtp_username, settings.smtp_host, settings.smtp_username
        ));
    }
    if host_lower == "smtp.163.com" {
        warnings.push(
            "163 邮箱坑提醒:smtp_password 必须是邮箱设置里的「客户端授权码」(开启 SMTP 服务时分配的 16 位字符串),不是邮箱登录密码。如果填登录密码会卡在认证失败。"
                .into(),
        );
    }
    if (host_lower.contains("163.com") || host_lower.contains("qq.com")) && settings.smtp_port == 25 {
        warnings.push(
            "国内邮箱 25 端口大多被云厂商屏蔽,推荐改用 587 (STARTTLS) 或 465 (SSL/TLS)。"
                .into(),
        );
    }

    // 在跑 lettre 之前先做一次裸 TCP connect 探测,能把"网络层不通"
    // (云厂商屏蔽 outbound SMTP 端口、防火墙等)和"SMTP 业务错误"
    // (账号/密码/from 不对)区分开。Aliyun ECS 默认屏蔽 25 端口,
    // 部分账号也限制 587/465 — 如果是这种情况,业务报错没法说清,
    // TCP probe 直接告诉用户"网络不通"。
    let probe_addr = format!("{}:{}", settings.smtp_host, settings.smtp_port);
    let probe_result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tokio::net::TcpStream::connect(&probe_addr),
    )
    .await;
    if let Err(_) = probe_result {
        // outer Err = elapsed (5s timeout). DNS+TCP 都超时了。
        return Ok(HttpResponse::BadGateway().json(json!({
            "ok": false,
            "error": format!(
                "TCP 连接 {} 5 秒内没建立成功 — 极大概率是云厂商屏蔽了 outbound SMTP 端口。\n\
                 Aliyun ECS 默认禁用 25 端口,部分账号也限制 465/587(需要在 Aliyun 控制台「申请解封 25 端口」或改走 SMTP 中继商如 Mailgun / 阿里邮件推送 Direct Mail / 腾讯云 SES)。",
                probe_addr
            ),
            "diagnostic": "tcp_blocked",
        })));
    }
    if let Ok(Err(e)) = probe_result {
        // TCP connect 在 5s 内立即返回错误 (DNS resolve fail, ECONNREFUSED 等)。
        return Ok(HttpResponse::BadGateway().json(json!({
            "ok": false,
            "error": format!("无法连接 SMTP 服务器 {}: {}", probe_addr, e),
            "diagnostic": "tcp_error",
        })));
    }
    // TCP probe 成功 — 网络层 OK,继续走完整 SMTP 流程。

    let subject = "Tomato KOL · 测试邮件";

    // Mobile-friendly HTML — admin clicks "send test" from the UI and
    // sees the same chrome they'd see for a real notification.
    use crate::services::email_template::{card, email_shell, html_escape, Field};
    let host_str = format!("{}:{}", settings.smtp_host, settings.smtp_port);
    let host_e = html_escape(&host_str);
    let from_e = html_escape(&settings.from_address);
    let to_e = html_escape(&to_addr);
    let fields = [
        Field { label: "发件主机", value: &host_e, highlight: false },
        Field { label: "发件地址", value: &from_e, highlight: false },
        Field { label: "收件人", value: &to_e, highlight: false },
    ];
    let content = card("SMTP 测试邮件", Some("如果你看到这封邮件,SMTP 配置就是正常的"), &fields);
    let body_html = email_shell(
        subject,
        Some("用于验证邮件服务能否正常发送"),
        &content,
        Some("Tomato KOL · 自动测试通知"),
    );

    // 记下发送耗时,管理员就能直观看到 SMTP 是不是慢/卡。
    let t0 = std::time::Instant::now();
    let send_result = email_sender::send_html(&settings, &[to_addr.clone()], subject, &body_html).await;
    let elapsed_ms = t0.elapsed().as_millis() as u64;

    match send_result {
        Ok(()) => Ok(HttpResponse::Ok().json(json!({
            "ok": true,
            "to": to_addr,
            "elapsed_ms": elapsed_ms,
            "warnings": warnings,
            "hint": "SMTP 返回成功 ≠ 邮件一定到达。如果几分钟还看不到:① 检查垃圾邮件文件夹;② 上面 warnings 列出的配置问题(尤其 from_address 必须 = SMTP 认证账号);③ 163/QQ 等服务商的密码是「授权码」不是登录密码。",
        }))),
        Err(e) => Ok(HttpResponse::BadGateway().json(json!({
            "ok": false,
            "error": e,
            "elapsed_ms": elapsed_ms,
            "warnings": warnings,
        }))),
    }
}
