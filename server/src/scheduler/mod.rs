pub mod jobs;

use crate::db::DbPool;
use tokio_cron_scheduler::{Job, JobScheduler};

pub async fn start_scheduler(pool: DbPool, redis: redis::Client) -> anyhow::Result<()> {
    let sched = JobScheduler::new().await?;

    // === Tomato Platform Jobs ===

    // Crawl books - daily at 1:00 AM
    let p = pool.clone();
    sched.add(Job::new_async("0 0 1 * * *", move |_uuid, _l| {
        let pool = p.clone();
        Box::pin(async move {
            if let Err(e) = jobs::crawler_book_job(&pool).await {
                tracing::error!("CrawlerBookJob failed: {}", e);
            }
        })
    })?).await?;

    // Write back URLs - every 30 minutes
    let p = pool.clone();
    sched.add(Job::new_async("0 0,30 * * * *", move |_uuid, _l| {
        let pool = p.clone();
        Box::pin(async move {
            if let Err(e) = jobs::write_back_job(&pool).await {
                tracing::error!("WriteBackJob failed: {}", e);
            }
        })
    })?).await?;

    // Replace write back - every 2 hours at :30
    let p = pool.clone();
    sched.add(Job::new_async("0 30 */2 * * *", move |_uuid, _l| {
        let pool = p.clone();
        Box::pin(async move {
            if let Err(e) = jobs::replace_write_back_job(&pool).await {
                tracing::error!("ReplaceWriteBackJob failed: {}", e);
            }
        })
    })?).await?;

    // Refresh KOL tokens - every hour
    let p = pool.clone();
    sched.add(Job::new_async("0 0 * * * *", move |_uuid, _l| {
        let pool = p.clone();
        Box::pin(async move {
            if let Err(e) = jobs::refresh_kol_token_job(&pool).await {
                tracing::error!("RefreshKolTokenJob failed: {}", e);
            }
        })
    })?).await?;

    // Create invite codes - every hour at :15
    let p = pool.clone();
    sched.add(Job::new_async("0 15 * * * *", move |_uuid, _l| {
        let pool = p.clone();
        Box::pin(async move {
            if let Err(e) = jobs::create_invite_code_job(&pool).await {
                tracing::error!("CreateInviteCodeJob failed: {}", e);
            }
        })
    })?).await?;

    // Income notification - every 10 minutes
    let p = pool.clone();
    sched.add(Job::new_async("0 */10 * * * *", move |_uuid, _l| {
        let pool = p.clone();
        Box::pin(async move {
            if let Err(e) = jobs::income_notice_job(&pool).await {
                tracing::error!("IncomeNoticeJob failed: {}", e);
            }
        })
    })?).await?;

    // === QiMao Platform Jobs ===

    // Crawl QiMao books - daily at 2:00 AM
    let p = pool.clone();
    sched.add(Job::new_async("0 0 2 * * *", move |_uuid, _l| {
        let pool = p.clone();
        Box::pin(async move {
            if let Err(e) = jobs::crawler_qimao_book_job(&pool).await {
                tracing::error!("CrawlerQiMaoBookJob failed: {}", e);
            }
        })
    })?).await?;

    // Refresh QiMao tokens - every hour at :30
    let p = pool.clone();
    sched.add(Job::new_async("0 30 * * * *", move |_uuid, _l| {
        let pool = p.clone();
        Box::pin(async move {
            if let Err(e) = jobs::refresh_qimao_token_job(&pool).await {
                tracing::error!("RefreshQiMaoTokenJob failed: {}", e);
            }
        })
    })?).await?;

    // Sync QiMao task statuses - every 30 minutes at :15
    let p = pool.clone();
    sched.add(Job::new_async("0 15,45 * * * *", move |_uuid, _l| {
        let pool = p.clone();
        Box::pin(async move {
            if let Err(e) = jobs::qimao_sync_tasks_job(&pool).await {
                tracing::error!("QiMaoSyncTasksJob failed: {}", e);
            }
        })
    })?).await?;

    // QiMao write back - every hour at :45
    let p = pool.clone();
    sched.add(Job::new_async("0 45 * * * *", move |_uuid, _l| {
        let pool = p.clone();
        Box::pin(async move {
            if let Err(e) = jobs::qimao_write_back_job(&pool).await {
                tracing::error!("QiMaoWriteBackJob failed: {}", e);
            }
        })
    })?).await?;

    sched.start().await?;
    tracing::info!("Scheduler started with 10 jobs");
    Ok(())
}
