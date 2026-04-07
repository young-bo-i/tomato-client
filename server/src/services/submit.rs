use crate::db::DbPool;
use crate::errors::{AppError, AppResult};
use crate::models::brush_task::{SubmitBrushMessage, SubmitBrushTaskRequest};
use crate::services::text_filter;
use redis::AsyncCommands;

const STREAM_KEY: &str = "stream:submit_brush";

pub struct SubmitService {
    pool: DbPool,
    redis: redis::Client,
}

impl SubmitService {
    pub fn new(pool: DbPool, redis: redis::Client) -> Self {
        Self { pool, redis }
    }

    /// High-throughput submit handler
    /// 1. Filter text (CPU only, no IO)
    /// 2. Record statistics (async DB write)
    /// 3. Push to Redis Stream (microsecond latency)
    /// 4. Return immediately
    pub async fn submit(&self, account_id: i32, req: SubmitBrushTaskRequest) -> AppResult<bool> {
        if req.douyin_id == 0 || req.alias_name.is_empty() {
            return Err(AppError::BadRequest("Invalid parameters".into()));
        }

        // Step 1: Text filtering (pure CPU, zero-copy)
        let filter_word = text_filter::filter_title(&req.alias_name);

        // Step 2: Record statistics (fire-and-forget DB writes)
        let pool = self.pool.clone();
        let alias = req.alias_name.clone();
        let fw = filter_word.clone().unwrap_or_default();

        // Spawn both DB writes concurrently
        let (stats_result, request_result) = tokio::join!(
            sqlx::query(
                "INSERT INTO submit_word_statistics (account_id, douyin_id, original_word, filter_word) VALUES ($1, $2, $3, $4)",
            )
            .bind(account_id)
            .bind(req.douyin_id)
            .bind(&alias)
            .bind(&fw)
            .execute(&pool),
            sqlx::query(
                "INSERT INTO submit_brush_request (account_id, douyin_id) VALUES ($1, $2)",
            )
            .bind(account_id)
            .bind(req.douyin_id)
            .execute(&pool),
        );

        if let Err(e) = stats_result {
            tracing::error!("Failed to record word statistics: {}", e);
        }
        if let Err(e) = request_result {
            tracing::error!("Failed to record request: {}", e);
        }

        // Step 3: Validate filtered word
        let filter_word = match filter_word {
            Some(w) => w,
            None => return Ok(false),
        };

        // Step 4: Dedup check via Redis (5-day TTL)
        let dedup_key = format!("dedup:{}:{}", account_id, filter_word);
        let mut conn = self.redis
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| AppError::Redis(e))?;

        let exists: bool = conn.exists(&dedup_key).await.map_err(|e| AppError::Redis(e))?;
        if exists {
            return Ok(true); // Already submitted, skip but return success
        }

        // Set dedup key with 5-day TTL
        let _: () = conn.set_ex(&dedup_key, "1", 5 * 24 * 3600)
            .await
            .map_err(|e| AppError::Redis(e))?;

        // Step 5: Push message to Redis Stream
        let message = SubmitBrushMessage {
            account_id,
            douyin_id: req.douyin_id,
            alias_name: filter_word,
            share_url: req.share_url,
            first_picture_url: req.first_picture_url,
        };

        let payload = serde_json::to_string(&message)
            .map_err(|e| AppError::Internal(e.to_string()))?;

        let _: String = redis::cmd("XADD")
            .arg(STREAM_KEY)
            .arg("MAXLEN")
            .arg("~")
            .arg(100000)   // Keep at most ~100k messages
            .arg("*")      // Auto-generate ID
            .arg("data")
            .arg(&payload)
            .query_async(&mut conn)
            .await
            .map_err(|e| AppError::Redis(e))?;

        Ok(true)
    }
}
