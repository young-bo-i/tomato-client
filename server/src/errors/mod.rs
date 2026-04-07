use actix_web::{HttpResponse, ResponseError};
use std::fmt;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("Unauthorized: {0}")]
    Unauthorized(String),

    #[error("Bad request: {0}")]
    BadRequest(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Redis error: {0}")]
    Redis(#[from] redis::RedisError),
}

impl ResponseError for AppError {
    fn error_response(&self) -> HttpResponse {
        let (status, message) = match self {
            AppError::Unauthorized(msg) => (actix_web::http::StatusCode::UNAUTHORIZED, msg.clone()),
            AppError::BadRequest(msg) => (actix_web::http::StatusCode::BAD_REQUEST, msg.clone()),
            AppError::NotFound(msg) => (actix_web::http::StatusCode::NOT_FOUND, msg.clone()),
            AppError::Internal(msg) => {
                tracing::error!("Internal error: {}", msg);
                (actix_web::http::StatusCode::INTERNAL_SERVER_ERROR, "Internal server error".into())
            }
            AppError::Database(e) => {
                tracing::error!("Database error: {}", e);
                (actix_web::http::StatusCode::INTERNAL_SERVER_ERROR, "Database error".into())
            }
            AppError::Redis(e) => {
                tracing::error!("Redis error: {}", e);
                (actix_web::http::StatusCode::INTERNAL_SERVER_ERROR, "Cache error".into())
            }
        };
        HttpResponse::build(status).json(serde_json::json!({
            "success": false,
            "message": message,
        }))
    }
}

pub type AppResult<T> = Result<T, AppError>;
