//! 鉴权：argon2 口令哈希 + RS256 JWT。

use anyhow::{Context, Result, anyhow};
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use axum::http::HeaderMap;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use rand::TryRng;
use rsa::RsaPrivateKey;
use rsa::pkcs8::{DecodePrivateKey, EncodePrivateKey, EncodePublicKey, LineEnding};
use serde::{Deserialize, Serialize};

use super::error::ApiError;

/// 30 天有效期（秒）。
const TOKEN_TTL_SECS: u64 = 30 * 24 * 3600;

/// RS256 签发/校验密钥对（启动时从 PEM 解析好，避免每个请求重复解析）。
pub struct JwtKeys {
    encoding: EncodingKey,
    decoding: DecodingKey,
}

/// 读取 RSA 私钥 PEM（PKCS#8），不存在则生成 2048 位密钥对并写入（0600 权限）。
/// 公钥不单独落盘，每次启动从私钥导出。换发密钥（吊销所有 token）删除该文件即可。
pub fn load_or_create_keys(path: &std::path::Path) -> Result<JwtKeys> {
    let private = match std::fs::read_to_string(path) {
        Ok(pem) => RsaPrivateKey::from_pkcs8_pem(&pem).with_context(|| {
            format!(
                "解析 JWT RSA 私钥 {} 失败（如需重新生成请删除该文件）",
                path.display()
            )
        })?,
        Err(_) => {
            // debug 构建下 2048 位素数搜索可能要数秒，打点以免看起来像卡死。
            eprintln!("首次启动：生成 JWT RSA-2048 密钥对…");
            let key = RsaPrivateKey::new(&mut rsa::rand_core::OsRng, 2048)
                .map_err(|e| anyhow!("生成 RSA 密钥失败: {e}"))?;
            let pem = key
                .to_pkcs8_pem(LineEnding::LF)
                .map_err(|e| anyhow!("私钥 PEM 编码失败: {e}"))?;
            std::fs::write(path, pem.as_bytes())
                .with_context(|| format!("写入密钥 {}", path.display()))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
            }
            key
        }
    };
    let private_pem = private
        .to_pkcs8_pem(LineEnding::LF)
        .map_err(|e| anyhow!("私钥 PEM 编码失败: {e}"))?;
    let public_pem = private
        .to_public_key()
        .to_public_key_pem(LineEnding::LF)
        .map_err(|e| anyhow!("公钥 PEM 编码失败: {e}"))?;
    Ok(JwtKeys {
        encoding: EncodingKey::from_rsa_pem(private_pem.as_bytes())
            .map_err(|e| anyhow!("构建 JWT 签名密钥失败: {e}"))?,
        decoding: DecodingKey::from_rsa_pem(public_pem.as_bytes())
            .map_err(|e| anyhow!("构建 JWT 校验密钥失败: {e}"))?,
    })
}

#[derive(Serialize, Deserialize)]
pub struct Claims {
    /// 用户 id。
    pub sub: i64,
    pub username: String,
    pub exp: usize,
}

pub fn hash_password(password: &str) -> Result<String> {
    // 用 rand 0.10 的系统随机源自行生成推荐长度（16 字节）的盐，再走 password_hash 的
    // base64 编码，避免依赖 password_hash 内部旧版 rand_core（0.6）的 OsRng。
    let mut salt_bytes = [0u8; 16];
    rand::rngs::SysRng
        .try_fill_bytes(&mut salt_bytes)
        .map_err(|e| anyhow!("生成随机盐失败: {e}"))?;
    let salt = SaltString::encode_b64(&salt_bytes).map_err(|e| anyhow!("盐编码失败: {e}"))?;
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

pub fn issue_token(keys: &JwtKeys, user_id: i64, username: &str) -> Result<String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| anyhow!("时钟错误: {e}"))?
        .as_secs();
    let claims = Claims {
        sub: user_id,
        username: username.to_string(),
        exp: (now + TOKEN_TTL_SECS) as usize,
    };
    encode(&Header::new(Algorithm::RS256), &claims, &keys.encoding)
        .map_err(|e| anyhow!("签发 JWT 失败: {e}"))
}

/// 从 `Authorization: Bearer <jwt>` 头解析并校验 token，返回声明。
pub fn authenticate(keys: &JwtKeys, headers: &HeaderMap) -> Result<Claims, ApiError> {
    let raw = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| ApiError::unauthorized("缺少 Authorization 头，请先 tsk login"))?;
    let token = raw
        .strip_prefix("Bearer ")
        .or_else(|| raw.strip_prefix("bearer "))
        .ok_or_else(|| ApiError::unauthorized("Authorization 头格式应为 Bearer <token>"))?;
    let data = decode::<Claims>(token, &keys.decoding, &Validation::new(Algorithm::RS256))
        .map_err(|_| ApiError::unauthorized("token 无效或已过期，请重新 tsk login"))?;
    Ok(data.claims)
}

/// 鉴权 + 落地本地用户：验签后按 `username` 解析本地 users.id，并把
/// `claims.sub` 重写为本地 id 返回，调用方照旧用 `claims.sub` 即可。
///
/// 为什么不能直接用 token 里的 sub：token 可能由共享同一把 RSA 私钥的网关
/// （aiko_gateway 统一登录 JWT）签发，其 sub 是**网关侧**用户 id，与本地
/// id 空间无关——直接当 owner_id 用轻则外键失败，重则写到同 id 的别人名下。
/// username 才是跨服务的自然键。本地无此用户时即时建号（随机不可登录口令），
/// 顺带消除网关注册联动（provision-user 异步）的竞态。
pub async fn require_user(
    state: &super::AppState,
    headers: &HeaderMap,
) -> Result<Claims, ApiError> {
    let mut claims = authenticate(&state.jwt_keys, headers)?;
    claims.sub = super::admin::ensure_user(&state.db, &claims.username).await?;
    Ok(claims)
}

/// 可选鉴权：有合法 token 返回声明（sub 已落地为本地 id），否则 None（匿名视角）。
/// 用于公开市场接口——匿名访客只看「所有分组可见」资源，登录用户额外看到其分组可见的。
pub async fn require_user_opt(state: &super::AppState, headers: &HeaderMap) -> Option<Claims> {
    let mut claims = authenticate(&state.jwt_keys, headers).ok()?;
    match super::admin::ensure_user(&state.db, &claims.username).await {
        Ok(id) => {
            claims.sub = id;
            Some(claims)
        }
        Err(_) => None,
    }
}
