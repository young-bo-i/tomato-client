use actix_web::{web, HttpResponse};
use serde::{Deserialize, Serialize};

use crate::auth::jwt::JwtConfig;
use crate::auth::password;
use crate::auth::AuthUser;
use crate::db::DbPool;
use crate::errors::{AppError, AppResult};
use crate::models::user::{User, UserView};

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub token: String,
    pub user: UserView,
}

/// Common projection — pulls full User row + LEFT JOIN parent username
/// and EXISTS-test for subordinates so the UI can render team UI
/// conditionally without a follow-up request.
const SELECT_USER_WITH_TIER: &str = r#"
    SELECT u.id, u.username, u.password_hash, u.role, u.is_active, u.email,
           u.parent_user_id, u.tier2_contribution_pct,
           u.created_at, u.updated_at,
           p.username AS parent_username,
           EXISTS(SELECT 1 FROM users s WHERE s.parent_user_id = u.id) AS has_subordinates
    FROM users u
    LEFT JOIN users p ON p.id = u.parent_user_id
"#;

/// Hydrate a `UserView` from the dynamic-row query. Pulls the joined
/// parent_username + has_subordinates fields out of the row and
/// hands the rest to `User → UserView`.
async fn fetch_user_view(pool: &DbPool, user_id: i32) -> AppResult<UserView> {
    use sqlx::Row;
    let sql = format!("{SELECT_USER_WITH_TIER} WHERE u.id = $1");
    let row = sqlx::query(&sql)
        .bind(user_id)
        .fetch_optional(pool)
        .await?
        .ok_or(AppError::Unauthorized)?;

    let user = User {
        id: row.try_get("id")?,
        username: row.try_get("username")?,
        password_hash: row.try_get("password_hash")?,
        role: row.try_get("role")?,
        is_active: row.try_get("is_active")?,
        email: row.try_get("email").ok(),
        parent_user_id: row.try_get("parent_user_id").ok(),
        tier2_contribution_pct: row.try_get("tier2_contribution_pct").unwrap_or(0),
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    };

    let mut view: UserView = user.into();
    view.parent_username = row.try_get("parent_username").ok();
    view.has_subordinates = row.try_get("has_subordinates").unwrap_or(false);
    Ok(view)
}

pub async fn login(
    pool: web::Data<DbPool>,
    jwt: web::Data<JwtConfig>,
    body: web::Json<LoginRequest>,
) -> AppResult<HttpResponse> {
    let body = body.into_inner();
    if body.username.is_empty() || body.password.is_empty() {
        return Err(AppError::BadRequest("username and password required".into()));
    }

    // Use the legacy minimal projection here so we can hash-verify the
    // password without paying for the JOINs every login attempt.
    let user = sqlx::query_as::<_, User>(
        r#"SELECT id, username, password_hash, role, is_active, email,
                  parent_user_id, tier2_contribution_pct,
                  created_at, updated_at
           FROM users WHERE username = $1"#,
    )
    .bind(&body.username)
    .fetch_optional(pool.get_ref())
    .await?
    .ok_or(AppError::Unauthorized)?;

    if !user.is_active {
        return Err(AppError::Forbidden);
    }
    if !password::verify(&body.password, &user.password_hash) {
        return Err(AppError::Unauthorized);
    }

    let token = jwt
        .encode(user.id, &user.username, &user.role)
        .map_err(AppError::Internal)?;

    // Now fetch the full hierarchy view for the response so the UI
    // gets `parent_username` + `has_subordinates` immediately.
    let view = fetch_user_view(pool.get_ref(), user.id).await?;
    Ok(HttpResponse::Ok().json(LoginResponse { token, user: view }))
}

pub async fn me(pool: web::Data<DbPool>, user: AuthUser) -> AppResult<HttpResponse> {
    let view = fetch_user_view(pool.get_ref(), user.0.sub).await?;
    if !view.is_active {
        return Err(AppError::Forbidden);
    }
    Ok(HttpResponse::Ok().json(view))
}
