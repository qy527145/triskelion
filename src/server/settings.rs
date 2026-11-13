//! 系统设置：settings 表 key-value JSON 存储，管理后台运行时可改。
//!
//! 当前承载认证配置（`key = "auth"`）：自助注册开关与 LDAP 目录集成。
//! LDAP 服务账号密码以 AES-256-GCM（复用凭据池 master key）加密后再入库，
//! 内存结构中始终为明文，对外序列化时须经 [`masked_json`] 脱敏。

use anyhow::{Context, Result, anyhow};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};

use super::crypto;
use super::db::{Db, db_params};

/// settings 表里承载认证配置的 key。
const AUTH_KEY: &str = "auth";

/// settings 表里承载「资源上传策略」的 key（自成一类，不与认证配置混在一起）。
const UPLOAD_KEY: &str = "upload";

/// 资源上传策略：控制用户上传 / 重发布资源时的可见性。
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(default)]
pub struct UploadSettings {
    /// 强制用户上传的资源仅自己可见。开启后，非管理员上传或重发布的资源一律存为
    /// private——即使请求显式要求 public 也会被压回 private（覆盖式 upsert 下，
    /// 只改「默认值」挡不住「先传私有、再用 publish --visibility public 公开」）。
    /// 管理员经管理后台的可见性修改不受此限制。
    pub force_private_uploads: bool,
}

/// 读取上传策略；无记录或解析失败一律回退默认值（默认不强制，上传按请求的可见性）。
pub async fn load_upload(db: &Db) -> UploadSettings {
    let raw: Option<String> = match db
        .query_opt("SELECT value FROM settings WHERE key = ?1", db_params![UPLOAD_KEY])
        .await
    {
        Ok(Some(r)) => r.get(0).ok(),
        _ => None,
    };
    raw.as_deref()
        .and_then(|raw| match serde_json::from_str(raw) {
            Ok(s) => Some(s),
            Err(e) => {
                eprintln!("settings[upload] 解析失败，回退默认: {e}");
                None
            }
        })
        .unwrap_or_default()
}

/// 把「请求可见性」压成实际落库可见性的 SQL 片段：上传策略开启且请求不是 private
/// 时取 `private`，否则保留请求值。直接在 upsert 里计算，避免「读设置 → 判断 → 写库」
/// 之间的竞态。
///
/// 设置 key 与策略 JSON 以字面量内联（两者都由本模块生成、不含引号和用户输入），
/// 因此 SQL 只占 `?1`（请求可见性），调用方其余业务参数编号保持从 `?2` 起。
pub fn effective_visibility_sql() -> String {
    format!(
        "(CASE WHEN ?1 <> 'private' AND EXISTS(
                  SELECT 1 FROM settings WHERE key = 'upload' AND value = {})
              THEN 'private' ELSE ?1 END)",
        upload_value_literal(true)
    )
}

/// 上传策略 JSON 的 SQL 字面量（`'…'`，内部单引号按 SQL 规则双写）。
fn upload_value_literal(force_private_uploads: bool) -> String {
    let json = serde_json::to_string(&UploadSettings { force_private_uploads }).unwrap_or_default();
    format!("'{}'", json.replace('\'', "''"))
}

/// 判断本次上传是否被强制成 private。上传响应据此提示「已按当前策略仅自己可见」。
pub async fn force_private(db: &Db) -> bool {
    load_upload(db).await.force_private_uploads
}

/// 写入上传策略（整对象覆盖，开关很少变）。
pub async fn save_upload(db: &Db, s: &UploadSettings) -> anyhow::Result<()> {
    let json = serde_json::to_string(s).context("序列化上传策略")?;
    db.execute(
        "INSERT INTO settings(key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        db_params![UPLOAD_KEY, json],
    )
    .await
    .context("写入上传策略")?;
    Ok(())
}

/// 入库 JSON 中加密字段的前缀标记：`enc:<base64(nonce || ciphertext)>`。
const ENC_PREFIX: &str = "enc:";

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct AuthSettings {
    /// 是否开放自助注册。关闭后新账号只能来自管理员创建或 LDAP 首次登录。
    pub registration_enabled: bool,
    pub ldap: LdapSettings,
}

impl Default for AuthSettings {
    fn default() -> Self {
        Self {
            registration_enabled: true,
            ldap: LdapSettings::default(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct LdapSettings {
    /// 启用后登录接口对本地不存在 / 标记为 LDAP 的账号走目录绑定认证。
    pub enabled: bool,
    /// 服务器地址：`ldap://host:389` 或 `ldaps://host:636`。
    pub url: String,
    /// 明文连接后先 StartTLS 升级（仅对 ldap:// 有意义）。
    pub start_tls: bool,
    /// 跳过 TLS 证书校验（内网自签名目录）。
    pub no_tls_verify: bool,
    /// 搜索用服务账号 DN；留空则匿名绑定后搜索。
    pub bind_dn: String,
    /// 服务账号密码。内存中为明文；入库时加密为 `enc:` 前缀密文。
    pub bind_password: String,
    /// 用户搜索基 DN，如 `ou=people,dc=example,dc=com`。
    pub user_base_dn: String,
    /// 用户搜索过滤器，`{username}`（转义后）替换为登录名，如 `(uid={username})`。
    pub user_filter: String,
    /// 用户名属性（读回规范用户名 / 同步建 RDN 用），如 `uid` / `sAMAccountName`。
    pub username_attr: String,
    /// 本地用户同步到 LDAP 的目标基 DN；留空复用 user_base_dn。
    pub sync_base_dn: String,
}

impl Default for LdapSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            url: String::new(),
            start_tls: false,
            no_tls_verify: false,
            bind_dn: String::new(),
            bind_password: String::new(),
            user_base_dn: String::new(),
            user_filter: "(uid={username})".into(),
            username_attr: "uid".into(),
            sync_base_dn: String::new(),
        }
    }
}

/// 读取认证配置；无记录或解析失败一律回退默认值（不阻断登录链路）。
pub async fn load(db: &Db, master_key: &[u8; 32]) -> AuthSettings {
    let raw: Option<String> = match db
        .query_opt("SELECT value FROM settings WHERE key = ?1", db_params![AUTH_KEY])
        .await
    {
        Ok(Some(r)) => r.get(0).ok(),
        _ => None,
    };
    let Some(raw) = raw else {
        return AuthSettings::default();
    };
    let mut s: AuthSettings = match serde_json::from_str(&raw) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("settings[auth] 解析失败，回退默认: {e}");
            return AuthSettings::default();
        }
    };
    if let Some(b64) = s.ldap.bind_password.strip_prefix(ENC_PREFIX) {
        s.ldap.bind_password = decrypt_b64(master_key, b64).unwrap_or_else(|e| {
            eprintln!("settings[auth] LDAP 绑定密码解密失败（master key 变更？）: {e}");
            String::new()
        });
    }
    s
}

/// 持久化认证配置（bind_password 加密后入库）。
pub async fn save(db: &Db, master_key: &[u8; 32], s: &AuthSettings) -> Result<()> {
    let mut stored = s.clone();
    if !stored.ldap.bind_password.is_empty() {
        let (nonce, ct) = crypto::encrypt(master_key, &stored.ldap.bind_password)?;
        let mut buf = nonce;
        buf.extend_from_slice(&ct);
        stored.ldap.bind_password = format!("{ENC_PREFIX}{}", STANDARD.encode(buf));
    }
    let json = serde_json::to_string(&stored).context("序列化认证配置")?;
    db.execute(
        "INSERT INTO settings(key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        db_params![AUTH_KEY, json],
    )
    .await
    .context("写入认证配置")?;
    Ok(())
}

/// 供管理后台展示的脱敏视图：抹掉明文密码，仅暴露「是否已设置」。
pub fn masked_json(s: &AuthSettings) -> serde_json::Value {
    let mut v = serde_json::to_value(s).unwrap_or_default();
    if let Some(ldap) = v.get_mut("ldap").and_then(|l| l.as_object_mut()) {
        ldap.insert(
            "bind_password_set".into(),
            serde_json::json!(!s.ldap.bind_password.is_empty()),
        );
        ldap.insert("bind_password".into(), serde_json::json!(""));
    }
    v
}

fn decrypt_b64(master_key: &[u8; 32], b64: &str) -> Result<String> {
    let buf = STANDARD.decode(b64).context("密文不是合法 base64")?;
    anyhow::ensure!(buf.len() > 12, "密文过短");
    let (nonce, ct) = buf.split_at(12);
    crypto::decrypt(master_key, nonce, ct).map_err(|e| anyhow!(e))
}

#[cfg(test)]
mod upload_policy_tests {
    use super::*;

    async fn db() -> (Db, std::path::PathBuf) {
        static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "triskelion-upload-policy-{}-{seq}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let db = Db::connect(None, &dir).await.unwrap();
        crate::server::db::schema::init(&db).await.unwrap();
        db.execute(
            "INSERT INTO users(id, username, password_hash, created_at) VALUES (1, 'u', '', 't')",
            db_params![],
        )
        .await
        .unwrap();
        (db, dir)
    }

    /// 用市场侧 upsert 的同一套 SQL 写法插一条 MCP，返回实际落库的可见性。
    async fn insert_mcp(db: &Db, requested: &str) -> String {
        db.execute(
            &format!(
                "INSERT INTO mcps(owner_id, name, visibility, version, manifest, updated_at)
                 SELECT 1, 'm', {}, '1', '{{}}', 't'",
                effective_visibility_sql()
            ),
            db_params![requested],
        )
        .await
        .unwrap();
        db.query_row("SELECT visibility FROM mcps WHERE name = 'm'", db_params![])
            .await
            .unwrap()
            .get(0)
            .unwrap()
    }

    #[tokio::test]
    async fn disabled_keeps_requested_visibility() {
        let (db, dir) = db().await;
        assert_eq!(insert_mcp(&db, "public").await, "public");
        assert!(!force_private(&db).await);
        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn enabled_downgrades_public_but_keeps_private() {
        let (db, dir) = db().await;
        save_upload(&db, &UploadSettings { force_private_uploads: true })
            .await
            .unwrap();
        assert!(force_private(&db).await);
        assert_eq!(insert_mcp(&db, "public").await, "private");
        // 显式 private 不受影响（换名插入，避免撞唯一键）。
        db.execute("UPDATE mcps SET name = 'm1' WHERE name = 'm'", db_params![]).await.unwrap();
        assert_eq!(insert_mcp(&db, "private").await, "private");
        std::fs::remove_dir_all(dir).ok();
    }
}
