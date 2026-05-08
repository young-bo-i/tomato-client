use std::future::Future;
use std::pin::Pin;

use actix_web::{dev::Payload, web, FromRequest, HttpRequest};

use super::jwt::{Claims, JwtConfig};
use crate::db::DbPool;
use crate::errors::AppError;

/// Extracts an authenticated user from `Authorization: Bearer <token>`.
///
/// On every request we re-check that the user row exists AND
/// `is_active = TRUE`. This makes admin's "disable user" toggle take
/// effect immediately — no token blocklist needed, no waiting for JWT
/// expiry. Cost: one PK-indexed `SELECT is_active FROM users WHERE id`
/// per authenticated request, which is in the noise compared to the
/// handlers' own queries.
///
/// Outcomes:
///   * Token valid + user active   → `Ok(AuthUser(claims))`
///   * Token valid + user disabled → `Err(Forbidden)`
///   * Token valid + user deleted  → `Err(Unauthorized)` (treat as logged-out)
///   * Token missing/invalid       → `Err(Unauthorized)`
pub struct AuthUser(pub Claims);

impl FromRequest for AuthUser {
    type Error = AppError;
    type Future = Pin<Box<dyn Future<Output = Result<Self, AppError>>>>;

    fn from_request(req: &HttpRequest, _: &mut Payload) -> Self::Future {
        let req = req.clone();
        Box::pin(async move {
            let claims = extract_claims(&req)?;
            check_active(&req, claims.sub).await?;
            Ok(AuthUser(claims))
        })
    }
}

/// Same as `AuthUser` but additionally requires `role == "admin"`.
/// Inherits the active-user check from the same `check_active` call.
pub struct AdminUser(pub Claims);

impl FromRequest for AdminUser {
    type Error = AppError;
    type Future = Pin<Box<dyn Future<Output = Result<Self, AppError>>>>;

    fn from_request(req: &HttpRequest, _: &mut Payload) -> Self::Future {
        let req = req.clone();
        Box::pin(async move {
            let claims = extract_claims(&req)?;
            check_active(&req, claims.sub).await?;
            if claims.role != "admin" {
                return Err(AppError::Forbidden);
            }
            Ok(AdminUser(claims))
        })
    }
}

fn extract_claims(req: &HttpRequest) -> Result<Claims, AppError> {
    let header = req
        .headers()
        .get(actix_web::http::header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .ok_or(AppError::Unauthorized)?;

    let token = header
        .strip_prefix("Bearer ")
        .ok_or(AppError::Unauthorized)?;

    let cfg = req
        .app_data::<web::Data<JwtConfig>>()
        .ok_or_else(|| AppError::Internal("jwt config missing".into()))?;

    cfg.decode(token).map_err(|_| AppError::Unauthorized)
}

/// Verify the user identified by `user_id` is still active in the DB.
/// PK-indexed lookup, ~sub-millisecond on a warm pool. Failures map
/// to either Forbidden (disabled) or Unauthorized (deleted).
async fn check_active(req: &HttpRequest, user_id: i32) -> Result<(), AppError> {
    let pool = req
        .app_data::<web::Data<DbPool>>()
        .ok_or_else(|| AppError::Internal("db pool missing".into()))?;

    let active: Option<bool> =
        sqlx::query_scalar("SELECT is_active FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_optional(pool.get_ref())
            .await
            .map_err(AppError::Database)?;

    match active {
        Some(true) => Ok(()),
        // User row exists but is disabled → 403, distinct from "no
        // such user" (deleted) so the client can show the right copy.
        Some(false) => Err(AppError::Forbidden),
        None => Err(AppError::Unauthorized),
    }
}
