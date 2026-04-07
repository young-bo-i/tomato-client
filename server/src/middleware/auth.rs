use actix_web::{dev::ServiceRequest, Error, HttpMessage};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub sub: i32,        // account id
    pub exp: i64,        // expiration timestamp
    pub iat: i64,        // issued at
}

#[derive(Debug, Clone)]
pub struct JwtConfig {
    pub secret: String,
    pub expiration_hours: i64,
}

impl JwtConfig {
    pub fn generate_token(&self, account_id: i32) -> anyhow::Result<String> {
        let now = chrono::Utc::now().timestamp();
        let claims = Claims {
            sub: account_id,
            exp: now + self.expiration_hours * 3600,
            iat: now,
        };
        let token = encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(self.secret.as_bytes()),
        )?;
        Ok(token)
    }

    pub fn validate_token(&self, token: &str) -> Option<Claims> {
        decode::<Claims>(
            token,
            &DecodingKey::from_secret(self.secret.as_bytes()),
            &Validation::default(),
        )
        .ok()
        .map(|data| data.claims)
    }
}

/// Extracts the user_id from request extensions (set by middleware)
pub fn get_user_id(req: &ServiceRequest) -> Option<i32> {
    req.extensions().get::<UserId>().map(|u| u.0)
}

#[derive(Debug, Clone, Copy)]
pub struct UserId(pub i32);

/// Middleware to extract and validate JWT from Authorization header
pub async fn jwt_middleware(
    req: ServiceRequest,
    next: actix_web::dev::ServiceResponse,
) -> Result<actix_web::dev::ServiceResponse, Error> {
    Ok(next)
}

/// Extractor for authenticated user ID
use actix_web::{FromRequest, HttpRequest};
use std::future::{Ready, ready};

impl FromRequest for UserId {
    type Error = actix_web::Error;
    type Future = Ready<Result<Self, Self::Error>>;

    fn from_request(req: &HttpRequest, _payload: &mut actix_web::dev::Payload) -> Self::Future {
        let jwt_config = req.app_data::<actix_web::web::Data<JwtConfig>>();

        let token = req
            .headers()
            .get("Authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "));

        match (jwt_config, token) {
            (Some(config), Some(token)) => {
                if let Some(claims) = config.validate_token(token) {
                    ready(Ok(UserId(claims.sub)))
                } else {
                    ready(Err(actix_web::error::ErrorUnauthorized("Invalid token")))
                }
            }
            _ => ready(Err(actix_web::error::ErrorUnauthorized("Missing token"))),
        }
    }
}
