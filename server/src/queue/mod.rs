pub mod consumer;

use crate::db::DbPool;

pub async fn start_consumers(pool: DbPool, redis: redis::Client, worker_count: usize) {
    // Create consumer group if not exists
    if let Ok(mut conn) = redis.get_multiplexed_async_connection().await {
        let _: Result<(), _> = redis::cmd("XGROUP")
            .arg("CREATE")
            .arg("stream:submit_brush")
            .arg("brush_consumers")
            .arg("0")
            .arg("MKSTREAM")
            .query_async(&mut conn)
            .await;
    }

    for i in 0..worker_count {
        let pool = pool.clone();
        let redis = redis.clone();
        let consumer_name = format!("worker-{}", i);

        tokio::spawn(async move {
            let consumer = consumer::BrushConsumer::new(pool, redis, consumer_name);
            consumer.run().await;
        });
    }

    tracing::info!("Started {} stream consumers", worker_count);
}
