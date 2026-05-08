use actix_web::{web, HttpResponse};
use serde::Deserialize;
use sqlx::Row;

use crate::auth::password;
use crate::auth::AdminUser;
use crate::db::DbPool;
use crate::errors::{AppError, AppResult};
use crate::models::user::{normalize_notify_emails, Role, User, UserView};

/// Allowed buckets for `tier2_contribution_pct`. Same set the global
/// admin contribution accepts; centralized at the API layer because
/// the DB CHECK is the wider 0..=100 envelope.
const ALLOWED_TIER2_PCT: &[i32] = &[0, 10, 20, 50, 100];

#[derive(Debug, Deserialize)]
pub struct CreateUserRequest {
    pub username: String,
    pub password: String,
    pub role: Role,
    /// 收件邮箱数组(可空)。每条通知发送给数组里的所有地址。
    /// 服务端做基础格式校验 + 去重 + trim,详见
    /// `models::user::normalize_notify_emails`。
    #[serde(default)]
    pub notify_emails: Vec<String>,
    /// Optional parent (creates this row as a tier-2 user). Must point
    /// at an existing tier-1 non-admin user; admins cannot have a
    /// parent. Validated server-side.
    #[serde(default)]
    pub parent_user_id: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateUserRequest {
    pub password: Option<String>,
    pub role: Option<Role>,
    pub is_active: Option<bool>,
    /// `None` (字段省略) 表示不改;`Some([])` 清空收件人;`Some([...])`
    /// 替换为新数组。规范化 + 校验同 create。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notify_emails: Option<Vec<String>>,
    /// Tri-state: `None` (omit) preserves; `Some(None)` clears (promotes
    /// tier-2 → tier-1); `Some(Some(id))` reassigns to a different
    /// parent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_user_id: Option<Option<i32>>,
    /// Admin-side override for the user's `tier2_contribution_pct`.
    /// Users themselves should use `PUT /api/users/me/tier2_contribution`.
    #[serde(default)]
    pub tier2_contribution_pct: Option<i32>,
}

pub async fn list(pool: web::Data<DbPool>, _: AdminUser) -> AppResult<HttpResponse> {
    // Enrich every row with parent_username (LEFT JOIN) and the
    // has_subordinates EXISTS test so the admin UI can render tier
    // badges without N follow-up queries.
    let rows = sqlx::query(
        r#"SELECT u.id, u.username, u.password_hash, u.role, u.is_active,
                  u.notify_emails,
                  u.parent_user_id, u.tier2_contribution_pct,
                  u.created_at, u.updated_at,
                  p.username AS parent_username,
                  EXISTS(SELECT 1 FROM users s WHERE s.parent_user_id = u.id) AS has_subordinates
           FROM users u
           LEFT JOIN users p ON p.id = u.parent_user_id
           ORDER BY u.id ASC"#,
    )
    .fetch_all(pool.get_ref())
    .await?;

    let mut views: Vec<UserView> = Vec::with_capacity(rows.len());
    for r in rows {
        // notify_emails 是 JSONB array,直接用 sqlx 的 json 解码读出。
        let notify_emails_json: serde_json::Value = r
            .try_get::<serde_json::Value, _>("notify_emails")
            .unwrap_or_else(|_| serde_json::json!([]));
        let notify_emails: Vec<String> = serde_json::from_value(notify_emails_json)
            .unwrap_or_default();
        let user = User {
            id: r.try_get("id")?,
            username: r.try_get("username")?,
            password_hash: r.try_get("password_hash")?,
            role: r.try_get("role")?,
            is_active: r.try_get("is_active")?,
            notify_emails,
            parent_user_id: r.try_get("parent_user_id").ok(),
            tier2_contribution_pct: r.try_get("tier2_contribution_pct").unwrap_or(0),
            created_at: r.try_get("created_at")?,
            updated_at: r.try_get("updated_at")?,
        };
        let mut view: UserView = user.into();
        view.parent_username = r.try_get("parent_username").ok();
        view.has_subordinates = r.try_get("has_subordinates").unwrap_or(false);
        views.push(view);
    }

    Ok(HttpResponse::Ok().json(views))
}

pub async fn create(
    pool: web::Data<DbPool>,
    _: AdminUser,
    body: web::Json<CreateUserRequest>,
) -> AppResult<HttpResponse> {
    let body = body.into_inner();
    if body.username.trim().is_empty() || body.password.is_empty() {
        return Err(AppError::BadRequest("username and password required".into()));
    }
    if body.password.len() < 6 {
        return Err(AppError::BadRequest("password must be at least 6 chars".into()));
    }

    // Validate parent assignment up front. Rules:
    //   * admin role cannot have a parent (admin ⊥ tier hierarchy)
    //   * parent must be an existing user
    //   * parent must itself be a tier-1 (parent_user_id IS NULL) so
    //     we never form a 3-level chain
    //   * parent must be non-admin (admins aren't tier-1 in this model)
    if let Some(pid) = body.parent_user_id {
        if matches!(body.role, Role::Admin) {
            return Err(AppError::BadRequest(
                "admin users cannot have a parent_user_id".into(),
            ));
        }
        validate_parent_candidate(pool.get_ref(), pid).await?;
    }

    let hash = password::hash(&body.password).map_err(AppError::Internal)?;

    let notify_emails =
        normalize_notify_emails(&body.notify_emails).map_err(AppError::BadRequest)?;
    let notify_emails_json = serde_json::to_value(&notify_emails)
        .map_err(|e| AppError::Internal(format!("encode notify_emails: {e}")))?;

    let result = sqlx::query_as::<_, User>(
        r#"INSERT INTO users (username, password_hash, role, is_active, notify_emails, parent_user_id)
           VALUES ($1, $2, $3, TRUE, $4, $5)
           RETURNING id, username, password_hash, role, is_active, notify_emails,
                     parent_user_id, tier2_contribution_pct,
                     created_at, updated_at"#,
    )
    .bind(body.username.trim())
    .bind(&hash)
    .bind(body.role.as_str())
    .bind(&notify_emails_json)
    .bind(body.parent_user_id)
    .fetch_one(pool.get_ref())
    .await;

    match result {
        Ok(u) => Ok(HttpResponse::Created().json(UserView::from(u))),
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
            Err(AppError::Conflict("username already exists".into()))
        }
        Err(e) => Err(AppError::Database(e)),
    }
}

/// Validate that `pid` is an acceptable parent for a tier-2 user:
/// must exist, must be active, must not itself have a parent, and
/// must not be admin. Returns Err(BadRequest) otherwise.
async fn validate_parent_candidate(pool: &DbPool, pid: i32) -> AppResult<()> {
    let row = sqlx::query(
        r#"SELECT role, is_active, parent_user_id
           FROM users WHERE id = $1"#,
    )
    .bind(pid)
    .fetch_optional(pool)
    .await?;

    let row = row.ok_or_else(|| {
        AppError::BadRequest(format!("parent_user_id {pid} does not exist"))
    })?;

    let role: String = row.try_get("role").unwrap_or_default();
    let is_active: bool = row.try_get("is_active").unwrap_or(false);
    let parent_of_parent: Option<i32> = row.try_get("parent_user_id").ok();

    if !is_active {
        return Err(AppError::BadRequest(format!(
            "parent_user_id {pid} is deactivated"
        )));
    }
    if role == "admin" {
        return Err(AppError::BadRequest(format!(
            "parent_user_id {pid} is an admin; admins are not tier-1 in the user hierarchy"
        )));
    }
    if parent_of_parent.is_some() {
        return Err(AppError::BadRequest(format!(
            "parent_user_id {pid} is itself a tier-2 user; only 2-level chains are allowed"
        )));
    }
    Ok(())
}

pub async fn update(
    pool: web::Data<DbPool>,
    admin: AdminUser,
    path: web::Path<i32>,
    body: web::Json<UpdateUserRequest>,
) -> AppResult<HttpResponse> {
    let id = path.into_inner();
    let body = body.into_inner();

    // Prevent admin from locking themselves out via their own PATCH call.
    if id == admin.0.sub {
        if matches!(body.is_active, Some(false)) {
            return Err(AppError::BadRequest("cannot disable yourself".into()));
        }
        if matches!(body.role, Some(Role::User)) {
            return Err(AppError::BadRequest("cannot demote yourself".into()));
        }
    }

    if body.password.is_none()
        && body.role.is_none()
        && body.is_active.is_none()
        && body.notify_emails.is_none()
        && body.parent_user_id.is_none()
        && body.tier2_contribution_pct.is_none()
    {
        return Err(AppError::BadRequest("no fields to update".into()));
    }

    let pw_hash = match &body.password {
        Some(pw) if pw.len() < 6 => {
            return Err(AppError::BadRequest("password must be at least 6 chars".into()))
        }
        Some(pw) => Some(password::hash(pw).map_err(AppError::Internal)?),
        None => None,
    };

    // notify_emails 二元状态:None = 不改;Some([...]) = 整体替换。
    // 空数组合法,表示"清空所有收件人"。
    let (touch_emails, emails_json): (bool, Option<serde_json::Value>) =
        match &body.notify_emails {
            None => (false, None),
            Some(list) => {
                let normalized =
                    normalize_notify_emails(list).map_err(AppError::BadRequest)?;
                let json = serde_json::to_value(&normalized).map_err(|e| {
                    AppError::Internal(format!("encode notify_emails: {e}"))
                })?;
                (true, Some(json))
            }
        };

    // Parent assignment uses the same tri-state semantics. If we're
    // touching it, run the full validator chain:
    //
    //   * Setting to a real id: candidate must be active tier-1 non-admin
    //   * Demoting THIS row to tier-2 (Some(Some(_))): forbidden if THIS
    //     row currently has its own subordinates (cycle prevention —
    //     can't have a tier-2 with tier-2 children).
    //   * Promoting this row to admin while ALSO holding a parent: reject
    //     (admin ⊥ tier hierarchy). Effectively: if final role is admin
    //     and final parent_user_id is Some, reject.
    let (touch_parent, parent_value): (bool, Option<i32>) = match &body.parent_user_id {
        None => (false, None),
        Some(None) => (true, None),
        Some(Some(pid)) => (true, Some(*pid)),
    };

    if touch_parent {
        if let Some(pid) = parent_value {
            // Self-reference is also blocked by the table CHECK, but
            // surfacing it as a 400 here is friendlier than letting
            // sqlx return a constraint-violation error.
            if pid == id {
                return Err(AppError::BadRequest(
                    "cannot set parent_user_id to self".into(),
                ));
            }
            validate_parent_candidate(pool.get_ref(), pid).await?;

            // Cycle / tier-3 prevention: if THIS user has subordinates,
            // they're a tier-1; demoting them to tier-2 would create a
            // 3-level chain (sub → this → newparent).
            let has_subs = sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS(SELECT 1 FROM users WHERE parent_user_id = $1)",
            )
            .bind(id)
            .fetch_one(pool.get_ref())
            .await
            .unwrap_or(false);
            if has_subs {
                return Err(AppError::BadRequest(format!(
                    "user {id} has tier-2 subordinates; cannot become tier-2 themselves \
                     (would form a 3-level chain). Reassign or remove the subordinates first."
                )));
            }
        }
        // If the post-update role is admin AND we're assigning a parent,
        // that's invalid (admin ⊥ tier hierarchy). We compute the
        // post-update role from `body.role` if supplied, else the
        // current row's role.
        let role_after_is_admin = match &body.role {
            Some(r) => matches!(r, Role::Admin),
            None => {
                let cur: Option<String> =
                    sqlx::query_scalar("SELECT role FROM users WHERE id = $1")
                        .bind(id)
                        .fetch_optional(pool.get_ref())
                        .await?;
                match cur {
                    Some(s) => s == "admin",
                    None => return Err(AppError::NotFound(format!("user {id}"))),
                }
            }
        };
        if parent_value.is_some() && role_after_is_admin {
            return Err(AppError::BadRequest(
                "admin users cannot have a parent_user_id".into(),
            ));
        }
    }

    // Validate tier2_contribution_pct bucket if supplied.
    if let Some(pct) = body.tier2_contribution_pct {
        if !ALLOWED_TIER2_PCT.contains(&pct) {
            return Err(AppError::BadRequest(format!(
                "tier2_contribution_pct must be one of {ALLOWED_TIER2_PCT:?}, got {pct}"
            )));
        }
    }

    let row = sqlx::query_as::<_, User>(
        r#"UPDATE users SET
             password_hash          = COALESCE($1, password_hash),
             role                   = COALESCE($2, role),
             is_active              = COALESCE($3, is_active),
             notify_emails          = CASE WHEN $4 THEN $5 ELSE notify_emails END,
             parent_user_id         = CASE WHEN $6 THEN $7 ELSE parent_user_id END,
             tier2_contribution_pct = COALESCE($8, tier2_contribution_pct)
           WHERE id = $9
           RETURNING id, username, password_hash, role, is_active, notify_emails,
                     parent_user_id, tier2_contribution_pct,
                     created_at, updated_at"#,
    )
    .bind(pw_hash)
    .bind(body.role.map(|r| r.as_str()))
    .bind(body.is_active)
    .bind(touch_emails)
    .bind(&emails_json)
    .bind(touch_parent)
    .bind(parent_value)
    .bind(body.tier2_contribution_pct)
    .bind(id)
    .fetch_optional(pool.get_ref())
    .await?
    .ok_or_else(|| AppError::NotFound(format!("user {id}")))?;

    Ok(HttpResponse::Ok().json(UserView::from(row)))
}

pub async fn delete(
    pool: web::Data<DbPool>,
    admin: AdminUser,
    path: web::Path<i32>,
) -> AppResult<HttpResponse> {
    let id = path.into_inner();
    if id == admin.0.sub {
        return Err(AppError::BadRequest("cannot delete yourself".into()));
    }

    let res = sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(id)
        .execute(pool.get_ref())
        .await?;

    if res.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("user {id}")));
    }
    Ok(HttpResponse::NoContent().finish())
}
