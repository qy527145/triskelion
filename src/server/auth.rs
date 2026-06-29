//! 鉴权：argon2 口令哈希 + HS256 JWT。

use anyhow::{Result, anyhow};
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use axum::http::HeaderMap;
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};

use super::error::ApiError;

/// 30 天有效期（秒）。
const TOKEN_TTL_SECS: u64 = 30 * 24 * 3600;

#[derive(Serialize, Deserialize)]
pub struct Claims {
    /// 用户 id。
    pub sub: i64,
    pub username: String,
    pub exp: usize,
}

pub fn hash_password(password: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| anyhow!("口令哈希失败: {e}"))
}

pub fn verify_password(password: &str, hash: &str) -> bool {
    match PasswordHash::new(hash) {
        Ok(parsed) => Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok(),
        Err(_) => false,
    }
}

pub fn issue_token(secret: &[u8], user_id: i64, username: &str) -> Result<String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| anyhow!("时钟错误: {e}"))?
        .as_secs();
    let claims = Claims {
        sub: user_id,
        username: username.to_string(),
        exp: (now + TOKEN_TTL_SECS) as usize,
    };
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret),
    )
    .map_err(|e| anyhow!("签发 JWT 失败: {e}"))
}

/// 从 `Authorization: Bearer <jwt>` 头解析并校验 token，返回声明。
pub fn authenticate(secret: &[u8], headers: &HeaderMap) -> Result<Claims, ApiError> {
    let raw = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| ApiError::unauthorized("缺少 Authorization 头，请先 tsk login"))?;
    let token = raw
        .strip_prefix("Bearer ")
        .or_else(|| raw.strip_prefix("bearer "))
        .ok_or_else(|| ApiError::unauthorized("Authorization 头格式应为 Bearer <token>"))?;
    let data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret),
        &Validation::default(),
    )
    .map_err(|_| ApiError::unauthorized("token 无效或已过期，请重新 tsk login"))?;
    Ok(data.claims)
}
