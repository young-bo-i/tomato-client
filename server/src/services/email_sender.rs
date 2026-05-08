//! Thin SMTP wrapper around `lettre`. Reads `email_settings` (the
//! single-row config table) and ships a message.
//!
//! Used by the admin "send test" endpoint today; future notification
//! jobs (income digest, alias-state diffs, etc.) will plug in here.
//!
//! TLS strategy:
//!   * `use_tls = false`              → plaintext (port 25, internal MTAs only)
//!   * `use_tls = true`  + port 465   → implicit TLS (legacy SSL)
//!   * `use_tls = true`  + any other  → STARTTLS upgrade (modern submission)

use lettre::message::header::ContentType;
use lettre::message::Mailbox;
use lettre::transport::smtp::authentication::Credentials;
use lettre::transport::smtp::client::Tls;
use lettre::transport::smtp::client::TlsParameters;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use sqlx::Row;

use crate::db::DbPool;

/// Verbatim copy of the row in `email_settings`. Loader returns this
/// rather than expose sqlx types so callers don't pull in the schema
/// just to call `send`.
#[derive(Debug, Clone, Default)]
pub struct EmailSettings {
    pub smtp_host: String,
    pub smtp_port: i32,
    pub smtp_username: String,
    pub smtp_password: String,
    pub from_address: String,
    pub from_name: String,
    pub use_tls: bool,
    pub recipients: Vec<String>,
}

impl EmailSettings {
    /// Returns the first hard error if the config is incomplete enough
    /// to make sending impossible. Used by both the API validator and
    /// the sender as a defense-in-depth check.
    pub fn validation_error(&self) -> Option<&'static str> {
        if self.smtp_host.trim().is_empty() {
            return Some("smtp_host is empty");
        }
        if self.smtp_port <= 0 || self.smtp_port > 65535 {
            return Some("smtp_port out of range");
        }
        if self.from_address.trim().is_empty() {
            return Some("from_address is empty");
        }
        None
    }
}

/// Load the (single) row from `email_settings`. The row is seeded by
/// migration 021 so this never returns `None` in practice; if it does
/// the database wasn't migrated and we surface that as an error.
pub async fn load(pool: &DbPool) -> Result<EmailSettings, String> {
    let row = sqlx::query(
        r#"SELECT smtp_host, smtp_port, smtp_username, smtp_password,
                  from_address, from_name, use_tls, recipients
           FROM email_settings WHERE id = 1"#,
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("load email_settings: {e}"))?;
    let Some(row) = row else {
        return Err("email_settings row missing — migration 021 not applied".into());
    };
    let recipients: serde_json::Value = row
        .try_get("recipients")
        .map_err(|e| format!("recipients col: {e}"))?;
    let recipients: Vec<String> = recipients
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();
    Ok(EmailSettings {
        smtp_host: row.try_get("smtp_host").unwrap_or_default(),
        smtp_port: row.try_get("smtp_port").unwrap_or(587),
        smtp_username: row.try_get("smtp_username").unwrap_or_default(),
        smtp_password: row.try_get("smtp_password").unwrap_or_default(),
        from_address: row.try_get("from_address").unwrap_or_default(),
        from_name: row.try_get("from_name").unwrap_or_default(),
        use_tls: row.try_get("use_tls").unwrap_or(true),
        recipients,
    })
}

/// Send a plain-text email. Returns `Ok(())` only after the SMTP
/// server has acknowledged the message. Caller decides how to
/// surface failures (the admin "test" endpoint surfaces the error
/// string verbatim so misconfigurations are easy to debug).
pub async fn send(
    settings: &EmailSettings,
    to: &[String],
    subject: &str,
    body: &str,
) -> Result<(), String> {
    send_with_content_type(settings, to, subject, body, ContentType::TEXT_PLAIN).await
}

/// Send an HTML email. Same delivery contract as `send`, but the
/// body is rendered as `text/html`. Used by the qimao income notice
/// pipeline to forward the platform's inline-styled HTML notices
/// verbatim instead of stripping them to plain text.
pub async fn send_html(
    settings: &EmailSettings,
    to: &[String],
    subject: &str,
    html_body: &str,
) -> Result<(), String> {
    send_with_content_type(settings, to, subject, html_body, ContentType::TEXT_HTML).await
}

async fn send_with_content_type(
    settings: &EmailSettings,
    to: &[String],
    subject: &str,
    body: &str,
    content_type: ContentType,
) -> Result<(), String> {
    if let Some(err) = settings.validation_error() {
        return Err(err.to_string());
    }
    if to.is_empty() {
        return Err("no recipients".into());
    }

    let from: Mailbox = if settings.from_name.trim().is_empty() {
        settings
            .from_address
            .parse()
            .map_err(|e| format!("parse from_address: {e}"))?
    } else {
        format!("{} <{}>", settings.from_name, settings.from_address)
            .parse()
            .map_err(|e| format!("parse from with name: {e}"))?
    };

    let mut builder = Message::builder()
        .from(from)
        .subject(subject)
        .header(content_type);
    for addr in to {
        let parsed: Mailbox = addr
            .parse()
            .map_err(|e| format!("parse recipient {addr}: {e}"))?;
        builder = builder.to(parsed);
    }
    let message = builder
        .body(body.to_string())
        .map_err(|e| format!("build message: {e}"))?;

    let creds = if !settings.smtp_username.is_empty() {
        Some(Credentials::new(
            settings.smtp_username.clone(),
            settings.smtp_password.clone(),
        ))
    } else {
        None
    };

    // Pick the right transport variant for the (TLS, port) combo. See
    // the module-level docstring for the full matrix.
    let port = settings.smtp_port as u16;
    let transport: AsyncSmtpTransport<Tokio1Executor> = if settings.use_tls {
        let tls = TlsParameters::new(settings.smtp_host.clone())
            .map_err(|e| format!("tls params: {e}"))?;
        let mut tb = if port == 465 {
            // Implicit TLS from the very first byte (legacy SSL submission).
            AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&settings.smtp_host)
                .port(port)
                .tls(Tls::Wrapper(tls))
        } else {
            // STARTTLS: handshake starts plaintext, upgrades to TLS.
            AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&settings.smtp_host)
                .port(port)
                .tls(Tls::Required(tls))
        };
        if let Some(c) = creds {
            tb = tb.credentials(c);
        }
        tb.build()
    } else {
        let mut tb = AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&settings.smtp_host)
            .port(port)
            .tls(Tls::None);
        if let Some(c) = creds {
            tb = tb.credentials(c);
        }
        tb.build()
    };

    transport
        .send(message)
        .await
        .map_err(|e| format!("send: {e}"))?;
    Ok(())
}
