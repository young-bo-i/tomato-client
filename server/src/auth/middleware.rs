use std::future::{ready, Ready};

use actix_web::{dev::Payload, web, FromRequest, HttpRequest};

use super::jwt::{Claims, JwtConfig};
use crate::errors::AppError;

/// Extracts an authenticated user from `Authorization: Bearer <token>`.
/// Rejects with 401 if missing/invalid/expired.
pub struct AuthUser(pub Claims);

impl FromRequest for AuthUser {
    type Error = AppError;
    type Future = Ready<Result<Self, AppError>>;

    fn from_request(req: &HttpRequest, _: &mut Payload) -> Self::Future {
        ready(extract_claims(req).map(AuthUser))
    }
}

/// Same as `AuthUser` but additionally requires `role == "admin"`.
pub struct AdminUser(pub Claims);

impl FromRequest for AdminUser {
    type Error = AppError;
    type Future = Ready<Result<Self, AppError>>;

    fn from_request(req: &HttpRequest, _: &mut Payload) -> Self::Future {
        ready(extract_claims(req).and_then(|c| {
            if c.role == "admin" {
                Ok(AdminUser(c))
            } else {
                Err(AppError::Forbidden)
            }
        }))
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
