use crate::db::DbPool;
use crate::models::brush_task::SubmitBrushMessage;
use crate::services::platform::tomato::TomatoClient;
use crate::services::platform::qimao::QiMaoClient;
use redis::AsyncCommands;
use sqlx::Row;

const STREAM_KEY: &str = "stream:submit_brush";
const GROUP_NAME: &str = "brush_consumers";

pub struct BrushConsumer {
    pool: DbPool,
    redis: redis::Client,
    consumer_name: String,
    tomato: TomatoClient,
    qimao: QiMaoClient,
}

impl BrushConsumer {
    pub fn new(pool: DbPool, redis: redis::Client, consumer_name: String) -> Self {
        Self {
            pool,
            redis,
            consumer_name,
            tomato: TomatoClient::new(),
            qimao: QiMaoClient::new(),
        }
    }

    pub async fn run(&self) {
        tracing::info!("Consumer {} started", self.consumer_name);

        loop {
            match self.read_messages().await {
                Ok(messages) => {
                    for (msg_id, message) in messages {
                        if let Err(e) = self.process_message(&message).await {
                            tracing::error!(
                                "Consumer {} failed to process {}: {}",
                                self.consumer_name, msg_id, e
                            );
                        }
                        // ACK the message
                        if let Err(e) = self.ack_message(&msg_id).await {
                            tracing::error!("Failed to ACK {}: {}", msg_id, e);
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("Consumer {} read error: {}", self.consumer_name, e);
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }
            }
        }
    }

    async fn read_messages(&self) -> anyhow::Result<Vec<(String, SubmitBrushMessage)>> {
        let mut conn = self.redis.get_multiplexed_async_connection().await?;

        // XREADGROUP GROUP brush_consumers worker-N COUNT 10 BLOCK 5000 STREAMS stream:submit_brush >
        let result: redis::Value = redis::cmd("XREADGROUP")
            .arg("GROUP")
            .arg(GROUP_NAME)
            .arg(&self.consumer_name)
            .arg("COUNT")
            .arg(10)
            .arg("BLOCK")
            .arg(5000)          // Block for 5 seconds max
            .arg("STREAMS")
            .arg(STREAM_KEY)
            .arg(">")           // Only new messages
            .query_async(&mut conn)
            .await?;

        parse_stream_response(result)
    }

    async fn ack_message(&self, msg_id: &str) -> anyhow::Result<()> {
        let mut conn = self.redis.get_multiplexed_async_connection().await?;
        let _: i32 = redis::cmd("XACK")
            .arg(STREAM_KEY)
            .arg(GROUP_NAME)
            .arg(msg_id)
            .query_async(&mut conn)
            .await?;
        Ok(())
    }

    async fn process_message(&self, message: &SubmitBrushMessage) -> anyhow::Result<()> {
        // Process for both Tomato and QiMao platforms concurrently
        let (tomato_result, qimao_result) = tokio::join!(
            self.process_tomato(message),
            self.process_qimao(message),
        );

        if let Err(e) = tomato_result {
            tracing::warn!("Tomato processing failed for '{}': {}", message.alias_name, e);
        }
        if let Err(e) = qimao_result {
            tracing::warn!("QiMao processing failed for '{}': {}", message.alias_name, e);
        }

        Ok(())
    }

    async fn process_tomato(&self, message: &SubmitBrushMessage) -> anyhow::Result<()> {
        // Get available KOL accounts for this user
        let kol_accounts = sqlx::query(
            "SELECT id, cookies FROM kol_account WHERE account_id = $1 AND is_deleted = FALSE AND status = 1",
        )
        .bind(message.account_id)
        .fetch_all(&self.pool)
        .await?;

        if kol_accounts.is_empty() {
            return Ok(());
        }

        // Get enabled platforms and limits from settings
        let settings = sqlx::query(
            "SELECT scene, setting_value FROM common_setting WHERE account_id = $1 AND is_deleted = FALSE",
        )
        .bind(message.account_id)
        .fetch_all(&self.pool)
        .await?;

        // Get random book for each platform
        let books = sqlx::query(
            "SELECT id, book_id, platform FROM kol_book WHERE is_deleted = FALSE ORDER BY RANDOM() LIMIT 4",
        )
        .fetch_all(&self.pool)
        .await?;

        // Try submitting to each KOL account across enabled platforms
        for kol in &kol_accounts {
            let kol_id: i32 = kol.get("id");
            let cookies: Option<String> = kol.get("cookies");
            let cookies = match cookies {
                Some(c) => c,
                None => continue,
            };

            // Get invite code with x_kol_token for this KOL
            let invite = sqlx::query(
                "SELECT x_kol_token FROM kol_invite_code WHERE kol_id = $1 AND x_kol_token IS NOT NULL AND is_deleted = FALSE LIMIT 1",
            )
            .bind(kol_id)
            .fetch_optional(&self.pool)
            .await?;

            let x_token: Option<String> = invite.and_then(|i| i.get("x_kol_token"));

            for book in &books {
                let book_id: String = book.get("book_id");
                let book_platform: i16 = book.get("platform");

                let result = self.tomato
                    .send_word(
                        &cookies,
                        x_token.as_deref(),
                        &message.alias_name,
                        &book_id,
                        book_platform,
                        "2",
                    )
                    .await;

                match result {
                    Ok(r) if r.is_succeed => {
                        sqlx::query(
                            r#"INSERT INTO kol_brush_task
                             (account_id, kol_id, alias_name, alias_id, share_url, first_picture_url, platform)
                             VALUES ($1, $2, $3, $4, $5, $6, $7)"#,
                        )
                        .bind(message.account_id)
                        .bind(kol_id)
                        .bind(&message.alias_name)
                        .bind(&r.alias_id)
                        .bind(&message.share_url)
                        .bind(&message.first_picture_url)
                        .bind(book_platform)
                        .execute(&self.pool)
                        .await?;

                        tracing::debug!(
                            "Tomato submit success: {} -> kol {} platform {}",
                            message.alias_name, kol_id, book_platform
                        );
                    }
                    Ok(r) if r.frequency_limiting => {
                        tracing::warn!("Tomato frequency limit hit for kol {}", kol_id);
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    }
                    Ok(r) => {
                        // Record as non-task (failed)
                        sqlx::query(
                            "INSERT INTO kol_brush_non_task (account_id, kol_id, alias_name, share_url, platform) VALUES ($1, $2, $3, $4, $5)",
                        )
                        .bind(message.account_id)
                        .bind(kol_id)
                        .bind(&message.alias_name)
                        .bind(&message.share_url)
                        .bind(book_platform)
                        .execute(&self.pool)
                        .await?;

                        tracing::debug!(
                            "Tomato submit failed: {} -> {:?}",
                            message.alias_name, r.message
                        );
                    }
                    Err(e) => {
                        tracing::error!("Tomato HTTP error: {}", e);
                    }
                }
            }
        }

        Ok(())
    }

    async fn process_qimao(&self, message: &SubmitBrushMessage) -> anyhow::Result<()> {
        // Get QiMao accounts
        let accounts = sqlx::query(
            "SELECT id, token FROM qimao_account WHERE account_id = $1 AND is_deleted = FALSE AND status = 1 AND token IS NOT NULL",
        )
        .bind(message.account_id)
        .fetch_all(&self.pool)
        .await?;

        if accounts.is_empty() {
            return Ok(());
        }

        // Get a random QiMao book
        let book = sqlx::query(
            "SELECT book_id FROM qimao_book WHERE is_deleted = FALSE AND is_forbidden = FALSE ORDER BY RANDOM() LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await?;

        let book_id: String = match book {
            Some(b) => b.get("book_id"),
            None => return Ok(()),
        };

        for account in &accounts {
            let account_id: i32 = account.get("id");
            let token: Option<String> = account.get("token");
            let token = match token {
                Some(t) => t,
                None => continue,
            };

            // Pre-check keyword
            let precheck = self.qimao.keyword_precheck(&token, &message.alias_name).await;
            if !precheck.unwrap_or(false) {
                continue;
            }

            let result = self.qimao
                .add_words(&token, &message.alias_name, &book_id)
                .await;

            match result {
                Ok(true) => {
                    sqlx::query(
                        r#"INSERT INTO qimao_brush_task
                         (account_id, qimao_account_id, alias_name, share_url, platform)
                         VALUES ($1, $2, $3, $4, 1)"#,
                    )
                    .bind(message.account_id)
                    .bind(account_id)
                    .bind(&message.alias_name)
                    .bind(&message.share_url)
                    .execute(&self.pool)
                    .await?;

                    tracing::debug!("QiMao submit success: {}", message.alias_name);
                }
                Ok(false) => {
                    sqlx::query(
                        "INSERT INTO qimao_brush_non_task (account_id, qimao_account_id, alias_name, share_url, platform) VALUES ($1, $2, $3, $4, 1)",
                    )
                    .bind(message.account_id)
                    .bind(account_id)
                    .bind(&message.alias_name)
                    .bind(&message.share_url)
                    .execute(&self.pool)
                    .await?;
                }
                Err(e) => {
                    tracing::error!("QiMao HTTP error: {}", e);
                }
            }
        }

        Ok(())
    }
}

/// Parse Redis XREADGROUP response into typed messages
fn parse_stream_response(value: redis::Value) -> anyhow::Result<Vec<(String, SubmitBrushMessage)>> {
    let mut results = Vec::new();

    // Response format: [[stream_name, [[msg_id, [field, value, ...]], ...]]]
    if let redis::Value::Array(streams) = value {
        for stream in streams {
            if let redis::Value::Array(parts) = stream {
                if parts.len() >= 2 {
                    if let redis::Value::Array(messages) = &parts[1] {
                        for msg in messages {
                            if let redis::Value::Array(msg_parts) = msg {
                                if msg_parts.len() >= 2 {
                                    let msg_id = match &msg_parts[0] {
                                        redis::Value::BulkString(b) => String::from_utf8_lossy(b).to_string(),
                                        _ => continue,
                                    };

                                    if let redis::Value::Array(fields) = &msg_parts[1] {
                                        // fields = [key, value, key, value, ...]
                                        for chunk in fields.chunks(2) {
                                            if chunk.len() == 2 {
                                                if let redis::Value::BulkString(val) = &chunk[1] {
                                                    if let Ok(msg) = serde_json::from_slice::<SubmitBrushMessage>(val) {
                                                        results.push((msg_id.clone(), msg));
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(results)
}
