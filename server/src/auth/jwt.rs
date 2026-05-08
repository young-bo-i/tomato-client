use chrono::{Duration, Local};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: i32,
    pub username: String,
    pub role: String,
    pub exp: i64,
}

#[derive(Clone)]
pub struct JwtConfig {
    enc: EncodingKey,
    dec: DecodingKey,
    ttl: Duration,
}

impl JwtConfig {
    pub fn new(secret: &str, ttl_hours: i64) -> Self {
        Self {
            enc: EncodingKey::from_secret(secret.as_bytes()),
            dec: DecodingKey::from_secret(secret.as_bytes()),
            ttl: Duration::hours(ttl_hours),
        }
    }

    pub fn encode(&self, user_id: i32, username: &str, role: &str) -> Result<String, String> {
        let exp = (Local::now() + self.ttl).timestamp();
        let claims = Claims {
            sub: user_id,
            username: username.into(),
            role: role.into(),
            exp,
        };
        encode(&Header::default(), &claims, &self.enc).map_err(|e| format!("encode: {e}"))
    }

    pub fn decode(&self, token: &str) -> Result<Claims, String> {
        decode::<Claims>(token, &self.dec, &Validation::default())
            .map(|d| d.claims)
            .map_err(|e| format!("decode: {e}"))
    }
}
