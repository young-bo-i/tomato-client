use crate::db::DbPool;
use crate::services::platform::tomato::TomatoClient;
use crate::services::platform::qimao::QiMaoClient;
use sqlx::Row;

/// Crawl books from Tomato platform (4 content types)
pub async fn crawler_book_job(pool: &DbPool) -> anyhow::Result<()> {
    tracing::info!("Starting CrawlerBookJob");
    let client = TomatoClient::new();

    let kol = sqlx::query(
        "SELECT id, cookies FROM kol_account WHERE is_deleted = FALSE AND status = 1 AND cookies IS NOT NULL LIMIT 1",
    )
    .fetch_optional(pool)
    .await?;

    let kol = match kol {
        Some(k) => k,
        None => {
            tracing::warn!("No active KOL accounts for book crawling");
            return Ok(());
        }
    };
    let cookies: Option<String> = kol.get("cookies");
    let cookies = cookies.unwrap_or_default();

    let content_tabs = [
        (1i16, "novel"),
        (2, "toutiao"),
        (3, "changting"),
        (4, "wukong"),
    ];

    for (platform, tab) in &content_tabs {
        let books = client.get_books(&cookies, tab, 1).await?;
        for book in &books {
            let book_id = book.get("book_id").and_then(|v| v.as_str()).unwrap_or("");
            let book_name = book.get("book_name").and_then(|v| v.as_str()).unwrap_or("");
            if book_id.is_empty() || book_name.is_empty() {
                continue;
            }
            // Upsert
            sqlx::query(
                r#"INSERT INTO kol_book (book_id, book_name, platform)
                 VALUES ($1, $2, $3)
                 ON CONFLICT DO NOTHING"#,
            )
            .bind(book_id)
            .bind(book_name)
            .bind(platform)
            .execute(pool)
            .await?;
        }
        tracing::info!("Crawled {} books for platform {}", books.len(), tab);
    }

    tracing::info!("CrawlerBookJob completed");
    Ok(())
}

/// Write back sharing URLs for pending tasks
pub async fn write_back_job(pool: &DbPool) -> anyhow::Result<()> {
    tracing::info!("Starting WriteBackJob");
    let client = TomatoClient::new();

    // Expire old tasks (>20 days)
    sqlx::query(
        r#"UPDATE kol_brush_task SET write_back_status = 2, updated_at = NOW()
         WHERE write_back_status = 0 AND created_at < NOW() - INTERVAL '20 days' AND is_deleted = FALSE"#,
    )
    .execute(pool)
    .await?;

    // Get pending tasks in batches
    let tasks = sqlx::query(
        r#"SELECT t.id, t.kol_id, t.alias_id, t.share_url,
                  k.cookies, ic.x_kol_token
         FROM kol_brush_task t
         JOIN kol_account k ON k.id = t.kol_id AND k.is_deleted = FALSE
         LEFT JOIN kol_invite_code ic ON ic.kol_id = t.kol_id AND ic.x_kol_token IS NOT NULL AND ic.is_deleted = FALSE
         WHERE t.write_back_status = 0 AND t.alias_id IS NOT NULL
           AND t.is_deleted = FALSE
         ORDER BY t.id ASC LIMIT 5000"#,
    )
    .fetch_all(pool)
    .await?;

    tracing::info!("WriteBackJob: {} pending tasks", tasks.len());

    for task in &tasks {
        let task_id: i64 = task.get("id");
        let alias_id: Option<String> = task.get("alias_id");
        let share_url: Option<String> = task.get("share_url");
        let cookies: Option<String> = task.get("cookies");
        let x_kol_token: Option<String> = task.get("x_kol_token");

        let alias_id = match alias_id {
            Some(id) => id,
            None => continue,
        };
        let share_url = match share_url {
            Some(ref url) if !url.is_empty() => url.clone(),
            _ => {
                sqlx::query(
                    "UPDATE kol_brush_task SET write_back_status = 3, updated_at = NOW() WHERE id = $1",
                )
                .bind(task_id)
                .execute(pool)
                .await?;
                continue;
            }
        };
        let cookies = cookies.unwrap_or_default();

        let result = client
            .write_back(&cookies, x_kol_token.as_deref(), &alias_id, &share_url)
            .await;

        match result {
            Ok(true) => {
                sqlx::query(
                    "UPDATE kol_brush_task SET write_back_status = 1, write_back_time = NOW(), updated_at = NOW() WHERE id = $1",
                )
                .bind(task_id)
                .execute(pool)
                .await?;
            }
            Ok(false) => {
                tracing::warn!("WriteBack failed for task {}", task_id);
            }
            Err(e) => {
                tracing::error!("WriteBack HTTP error for task {}: {}", task_id, e);
            }
        }
    }

    tracing::info!("WriteBackJob completed");
    Ok(())
}

/// Replace old sharing URLs with new ones
pub async fn replace_write_back_job(pool: &DbPool) -> anyhow::Result<()> {
    tracing::info!("Starting ReplaceWriteBackJob");
    // Similar to write_back_job but replaces URLs
    // Get tasks with write_back_status = 3 (ChangeShareUrl)
    let tasks = sqlx::query(
        r#"SELECT t.id, t.kol_id, t.alias_id,
                  k.cookies, ic.x_kol_token
         FROM kol_brush_task t
         JOIN kol_account k ON k.id = t.kol_id AND k.is_deleted = FALSE
         LEFT JOIN kol_invite_code ic ON ic.kol_id = t.kol_id AND ic.x_kol_token IS NOT NULL AND ic.is_deleted = FALSE
         WHERE t.write_back_status = 3 AND t.alias_id IS NOT NULL
           AND t.is_deleted = FALSE
         ORDER BY t.id DESC LIMIT 5000"#,
    )
    .fetch_all(pool)
    .await?;

    // Get today's non-task URLs for replacement
    let non_tasks = sqlx::query(
        "SELECT share_url FROM kol_brush_non_task WHERE share_url IS NOT NULL AND created_at >= CURRENT_DATE ORDER BY RANDOM() LIMIT 100",
    )
    .fetch_all(pool)
    .await?;

    let urls: Vec<String> = non_tasks.iter()
        .filter_map(|n| {
            let url: Option<String> = n.get("share_url");
            url
        })
        .collect();

    if urls.is_empty() {
        tracing::info!("No replacement URLs available");
        return Ok(());
    }

    let client = TomatoClient::new();
    for (i, task) in tasks.iter().enumerate() {
        let task_id: i64 = task.get("id");
        let alias_id: Option<String> = task.get("alias_id");
        let cookies: Option<String> = task.get("cookies");
        let x_kol_token: Option<String> = task.get("x_kol_token");

        let alias_id = match alias_id {
            Some(id) => id,
            None => continue,
        };
        let url = &urls[i % urls.len()];
        let cookies = cookies.unwrap_or_default();

        let result = client
            .write_back(&cookies, x_kol_token.as_deref(), &alias_id, url)
            .await;

        if let Ok(true) = result {
            sqlx::query(
                "UPDATE kol_brush_task SET write_back_status = 1, share_url = $1, write_back_time = NOW(), updated_at = NOW() WHERE id = $2",
            )
            .bind(url)
            .bind(task_id)
            .execute(pool)
            .await?;
        }
    }

    tracing::info!("ReplaceWriteBackJob completed, processed {} tasks", tasks.len());
    Ok(())
}

/// Refresh KOL invite code tokens
pub async fn refresh_kol_token_job(pool: &DbPool) -> anyhow::Result<()> {
    tracing::info!("Starting RefreshKolTokenJob");
    let client = TomatoClient::new();

    let codes = sqlx::query(
        r#"SELECT id, invite_code, share_token FROM kol_invite_code
         WHERE is_deleted = FALSE
           AND (last_refresh_time IS NULL OR last_refresh_time < NOW() - INTERVAL '20 hours')
         LIMIT 500"#,
    )
    .fetch_all(pool)
    .await?;

    let mut refreshed = 0;
    for code in &codes {
        let code_id: i64 = code.get("id");
        let invite_code: String = code.get("invite_code");
        let share_token: Option<String> = code.get("share_token");

        let share_token = match share_token {
            Some(t) => t,
            None => continue,
        };

        let token = client.invite_code_login(&invite_code, &share_token).await;
        if let Ok(Some(x_kol_token)) = token {
            sqlx::query(
                "UPDATE kol_invite_code SET x_kol_token = $1, last_refresh_time = NOW(), updated_at = NOW() WHERE id = $2",
            )
            .bind(&x_kol_token)
            .bind(code_id)
            .execute(pool)
            .await?;
            refreshed += 1;
        }
    }

    tracing::info!("RefreshKolTokenJob completed: refreshed {}/{}", refreshed, codes.len());
    Ok(())
}

/// Create invite codes for KOL accounts
pub async fn create_invite_code_job(pool: &DbPool) -> anyhow::Result<()> {
    tracing::info!("Starting CreateInviteCodeJob");
    let client = TomatoClient::new();

    let kols = sqlx::query(
        "SELECT id, account_id, cookies FROM kol_account WHERE is_deleted = FALSE AND status = 1 AND cookies IS NOT NULL",
    )
    .fetch_all(pool)
    .await?;

    for kol in &kols {
        let kol_id: i32 = kol.get("id");
        let kol_account_id: i32 = kol.get("account_id");
        let kol_cookies: Option<String> = kol.get("cookies");

        let count: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM kol_invite_code WHERE kol_id = $1 AND is_deleted = FALSE"
        )
        .bind(kol_id)
        .fetch_one(pool)
        .await?;

        if count.0 >= 100 {
            continue;
        }

        let cookies = kol_cookies.unwrap_or_default();
        let to_create = 100 - count.0 as i32;

        for _ in 0..to_create.min(10) {
            match client.create_invite_code(&cookies).await {
                Ok(data) => {
                    let code = data.get("invite_code").and_then(|v| v.as_str()).unwrap_or("");
                    let token = data.get("share_token").and_then(|v| v.as_str()).unwrap_or("");
                    if !code.is_empty() {
                        sqlx::query(
                            "INSERT INTO kol_invite_code (account_id, kol_id, invite_code, share_token) VALUES ($1, $2, $3, $4)",
                        )
                        .bind(kol_account_id)
                        .bind(kol_id)
                        .bind(code)
                        .bind(token)
                        .execute(pool)
                        .await?;
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to create invite code for kol {}: {}", kol_id, e);
                    break;
                }
            }
        }
    }

    tracing::info!("CreateInviteCodeJob completed");
    Ok(())
}

/// Check income and send email notifications
pub async fn income_notice_job(pool: &DbPool) -> anyhow::Result<()> {
    tracing::info!("Starting IncomeNoticeJob");
    let client = TomatoClient::new();

    let kols = sqlx::query(
        "SELECT id, account_id, cookies FROM kol_account WHERE is_deleted = FALSE AND status = 1 AND cookies IS NOT NULL",
    )
    .fetch_all(pool)
    .await?;

    for kol in &kols {
        let kol_id: i32 = kol.get("id");
        let kol_account_id: i32 = kol.get("account_id");
        let kol_cookies: Option<String> = kol.get("cookies");

        let cookies = kol_cookies.unwrap_or_default();
        let income_data = match client.get_income(&cookies).await {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!("Failed to fetch income for kol {}: {}", kol_id, e);
                continue;
            }
        };

        let total = income_data.get("total_income").and_then(|v| v.as_i64()).unwrap_or(0);
        let regular = income_data.get("regular_income").and_then(|v| v.as_i64()).unwrap_or(0);
        let bonus = income_data.get("bonus_income").and_then(|v| v.as_i64()).unwrap_or(0);

        // Upsert income record
        sqlx::query(
            r#"INSERT INTO kol_income (account_id, kol_id, total_income, regular_income, bonus_income, income_json, last_update_time)
             VALUES ($1, $2, $3, $4, $5, $6, NOW())
             ON CONFLICT (id) DO UPDATE SET
               total_income = $3, regular_income = $4, bonus_income = $5,
               income_json = $6, last_update_time = NOW(), updated_at = NOW()"#,
        )
        .bind(kol_account_id)
        .bind(kol_id)
        .bind(total)
        .bind(regular)
        .bind(bonus)
        .bind(serde_json::to_string(&income_data).ok())
        .execute(pool)
        .await?;
    }

    // TODO: Compare with previous income data and send email notifications
    // using lettre crate for SMTP

    tracing::info!("IncomeNoticeJob completed");
    Ok(())
}

/// Crawl QiMao books
pub async fn crawler_qimao_book_job(pool: &DbPool) -> anyhow::Result<()> {
    tracing::info!("Starting CrawlerQiMaoBookJob");
    let client = QiMaoClient::new();

    let account = sqlx::query(
        "SELECT token FROM qimao_account WHERE is_deleted = FALSE AND status = 1 AND token IS NOT NULL LIMIT 1",
    )
    .fetch_optional(pool)
    .await?;

    let token: Option<String> = account.and_then(|a| a.get("token"));
    let token = match token {
        Some(t) => t,
        None => {
            tracing::warn!("No QiMao account with token for book crawling");
            return Ok(());
        }
    };

    for page in 1..=10 {
        let books = client.get_books(&token, page).await?;
        if books.is_empty() {
            break;
        }
        for book in &books {
            let book_id = book.get("book_id").and_then(|v| v.as_str()).unwrap_or("");
            let book_name = book.get("book_name").and_then(|v| v.as_str()).unwrap_or("");
            let is_forbid = book.get("is_forbid").and_then(|v| v.as_bool()).unwrap_or(false);
            if book_id.is_empty() {
                continue;
            }
            sqlx::query(
                r#"INSERT INTO qimao_book (book_id, book_name, is_forbidden)
                 VALUES ($1, $2, $3)
                 ON CONFLICT DO NOTHING"#,
            )
            .bind(book_id)
            .bind(book_name)
            .bind(is_forbid)
            .execute(pool)
            .await?;
        }
    }

    tracing::info!("CrawlerQiMaoBookJob completed");
    Ok(())
}

/// Refresh QiMao account tokens
pub async fn refresh_qimao_token_job(pool: &DbPool) -> anyhow::Result<()> {
    tracing::info!("Starting RefreshQiMaoTokenJob");
    let client = QiMaoClient::new();

    let accounts = sqlx::query(
        r#"SELECT id, phone, password_hash FROM qimao_account
         WHERE is_deleted = FALSE AND status = 1
           AND (last_refresh_time IS NULL OR last_refresh_time < NOW() - INTERVAL '12 hours')"#,
    )
    .fetch_all(pool)
    .await?;

    for account in &accounts {
        let account_id: i32 = account.get("id");
        let phone: Option<String> = account.get("phone");
        let password_hash: Option<String> = account.get("password_hash");

        let phone = match phone {
            Some(p) => p,
            None => continue,
        };

        let token = client.signin(&phone, &password_hash.unwrap_or_default()).await;
        if let Ok(Some(t)) = token {
            sqlx::query(
                "UPDATE qimao_account SET token = $1, last_refresh_time = NOW(), updated_at = NOW() WHERE id = $2",
            )
            .bind(&t)
            .bind(account_id)
            .execute(pool)
            .await?;
        }
    }

    tracing::info!("RefreshQiMaoTokenJob completed");
    Ok(())
}

/// Sync QiMao task statuses
pub async fn qimao_sync_tasks_job(pool: &DbPool) -> anyhow::Result<()> {
    tracing::info!("Starting QiMaoSyncTasksJob");

    // Expire old tasks (>7 days)
    sqlx::query(
        r#"UPDATE qimao_brush_task SET task_status = 2, updated_at = NOW()
         WHERE task_status = 0 AND created_at < NOW() - INTERVAL '7 days' AND is_deleted = FALSE"#,
    )
    .execute(pool)
    .await?;

    // TODO: Fetch task statuses from QiMao API and update
    tracing::info!("QiMaoSyncTasksJob completed");
    Ok(())
}

/// QiMao write back links
pub async fn qimao_write_back_job(pool: &DbPool) -> anyhow::Result<()> {
    tracing::info!("Starting QiMaoWriteBackJob");

    // Expire old tasks (>30 days)
    sqlx::query(
        r#"UPDATE qimao_brush_task SET write_back_status = 2, updated_at = NOW()
         WHERE write_back_status = 0 AND created_at < NOW() - INTERVAL '30 days' AND is_deleted = FALSE"#,
    )
    .execute(pool)
    .await?;

    // TODO: Write back links for pending QiMao tasks
    tracing::info!("QiMaoWriteBackJob completed");
    Ok(())
}
