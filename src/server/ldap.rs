//! LDAP 目录集成：绑定认证（bind）、连通性测试与本地用户同步。
//!
//! 认证走标准两段式：先用服务账号（或匿名）绑定搜索出用户 DN，再以用户 DN +
//! 登录口令重新绑定验证。所有对目录的操作都带超时，避免目录不可达时拖死登录。

use std::collections::HashSet;
use std::time::Duration;

use ldap3::{Ldap, LdapConnAsync, LdapConnSettings, Mod, Scope, SearchEntry};

use super::settings::LdapSettings;

/// 连接与单次操作超时。
const TIMEOUT: Duration = Duration::from_secs(10);

/// LDAP RFC 4511 resultCode: invalidCredentials。
const RC_INVALID_CREDENTIALS: u32 = 49;

/// 认证失败归因，登录处理器据此区分「用户不存在」与「密码错误」。
pub enum LdapAuthError {
    /// 目录中搜不到该用户。
    NotFound,
    /// 搜到了但口令绑定失败。
    BadCredentials,
    /// 连接 / 配置 / 目录侧异常。
    Other(String),
}

/// 单个用户的同步结果。
pub struct SyncItem {
    pub username: String,
    /// created / updated / skipped / failed。
    pub action: &'static str,
    pub error: String,
}

/// 校验配置完整性（认证与同步共用的最小集）。
pub fn validate(cfg: &LdapSettings) -> Result<(), String> {
    if cfg.url.trim().is_empty() {
        return Err("LDAP 服务器地址不能为空".into());
    }
    if !cfg.url.starts_with("ldap://") && !cfg.url.starts_with("ldaps://") {
        return Err("LDAP 地址须以 ldap:// 或 ldaps:// 开头".into());
    }
    if cfg.user_base_dn.trim().is_empty() {
        return Err("用户基 DN（user_base_dn）不能为空".into());
    }
    if !cfg.user_filter.contains("{username}") {
        return Err("用户过滤器须包含 {username} 占位符".into());
    }
    if cfg.username_attr.trim().is_empty() {
        return Err("用户名属性（username_attr）不能为空".into());
    }
    Ok(())
}

async fn connect(cfg: &LdapSettings) -> Result<Ldap, String> {
    let mut settings = LdapConnSettings::new().set_conn_timeout(TIMEOUT);
    if cfg.start_tls {
        settings = settings.set_starttls(true);
    }
    if cfg.no_tls_verify {
        settings = settings.set_no_tls_verify(true);
    }
    let (conn, ldap) = LdapConnAsync::with_settings(settings, cfg.url.trim())
        .await
        .map_err(|e| format!("连接 LDAP 服务器失败: {e}"))?;
    ldap3::drive!(conn);
    Ok(ldap)
}

/// 服务账号绑定（bind_dn 为空则跳过，走匿名）。
async fn service_bind(ldap: &mut Ldap, cfg: &LdapSettings) -> Result<(), String> {
    if cfg.bind_dn.trim().is_empty() {
        return Ok(());
    }
    let res = ldap
        .with_timeout(TIMEOUT)
        .simple_bind(cfg.bind_dn.trim(), &cfg.bind_password)
        .await
        .map_err(|e| format!("服务账号绑定失败: {e}"))?;
    if res.rc != 0 {
        return Err(format!(
            "服务账号绑定被拒绝 (rc={}): {}",
            res.rc,
            if res.text.is_empty() { "请检查 Bind DN 与密码".into() } else { res.text }
        ));
    }
    Ok(())
}

/// 按配置的过滤器搜索用户，返回 (DN, 规范用户名)。
async fn find_user(
    ldap: &mut Ldap,
    cfg: &LdapSettings,
    username: &str,
) -> Result<Option<(String, String)>, String> {
    let filter = cfg
        .user_filter
        .replace("{username}", &ldap3::ldap_escape(username));
    let (entries, _) = ldap
        .with_timeout(TIMEOUT)
        .search(
            cfg.user_base_dn.trim(),
            Scope::Subtree,
            &filter,
            vec![cfg.username_attr.trim()],
        )
        .await
        .map_err(|e| format!("搜索用户失败: {e}"))?
        .success()
        .map_err(|e| format!("搜索用户失败: {e}"))?;
    let Some(entry) = entries.into_iter().next() else {
        return Ok(None);
    };
    let entry = SearchEntry::construct(entry);
    // 属性名大小写不敏感：按小写比对取回规范用户名，取不到则沿用登录名。
    let attr_lc = cfg.username_attr.trim().to_lowercase();
    let canonical = entry
        .attrs
        .iter()
        .find(|(k, _)| k.to_lowercase() == attr_lc)
        .and_then(|(_, vs)| vs.first())
        .cloned()
        .unwrap_or_else(|| username.to_string());
    Ok(Some((entry.dn, canonical)))
}

/// 绑定认证：成功返回目录中的规范用户名。
pub async fn authenticate(
    cfg: &LdapSettings,
    username: &str,
    password: &str,
) -> Result<String, LdapAuthError> {
    validate(cfg).map_err(LdapAuthError::Other)?;
    // LDAP 空口令绑定会被目录视为匿名绑定「成功」，必须显式拒绝。
    if password.is_empty() {
        return Err(LdapAuthError::BadCredentials);
    }
    let mut ldap = connect(cfg).await.map_err(LdapAuthError::Other)?;
    service_bind(&mut ldap, cfg)
        .await
        .map_err(LdapAuthError::Other)?;
    let Some((dn, canonical)) = find_user(&mut ldap, cfg, username)
        .await
        .map_err(LdapAuthError::Other)?
    else {
        let _ = ldap.unbind().await;
        return Err(LdapAuthError::NotFound);
    };
    // 以用户 DN 重新绑定验证口令。
    let res = ldap
        .with_timeout(TIMEOUT)
        .simple_bind(&dn, password)
        .await
        .map_err(|e| LdapAuthError::Other(format!("用户绑定失败: {e}")))?;
    let _ = ldap.unbind().await;
    match res.rc {
        0 => Ok(canonical),
        RC_INVALID_CREDENTIALS => Err(LdapAuthError::BadCredentials),
        rc => Err(LdapAuthError::Other(format!(
            "用户绑定被拒绝 (rc={rc}): {}",
            res.text
        ))),
    }
}

/// 连通性测试：连接 + 服务账号绑定 + 确认用户基 DN 存在，返回人类可读结论。
pub async fn test_connection(cfg: &LdapSettings) -> Result<String, String> {
    validate(cfg)?;
    let mut ldap = connect(cfg).await?;
    service_bind(&mut ldap, cfg).await?;
    let (entries, _) = ldap
        .with_timeout(TIMEOUT)
        .search(
            cfg.user_base_dn.trim(),
            Scope::Base,
            "(objectClass=*)",
            vec!["1.1"],
        )
        .await
        .map_err(|e| format!("查询用户基 DN 失败: {e}"))?
        .success()
        .map_err(|e| format!("用户基 DN 不可用（请检查 user_base_dn）: {e}"))?;
    let _ = ldap.unbind().await;
    if entries.is_empty() {
        return Err("用户基 DN 不存在".into());
    }
    Ok(format!(
        "连接成功：{}，服务账号{}，用户基 DN 可用",
        cfg.url.trim(),
        if cfg.bind_dn.trim().is_empty() { "未配置（匿名）" } else { "绑定通过" },
    ))
}

/// 把本地用户批量同步进目录：不存在则按 inetOrgPerson 建条目，已存在且开启口令
/// 哈希同步时覆写 userPassword。
///
/// 口令以 `{ARGON2}<PHC>` 形式写入，需目录侧支持 ARGON2 口令方案（如 OpenLDAP
/// 2.5+ 的 argon2 模块）才能用原口令登录；不支持时条目仍可建，口令需另行重置。
pub async fn sync_users(
    cfg: &LdapSettings,
    users: &[(String, String)],
    sync_password_hashes: bool,
) -> Result<Vec<SyncItem>, String> {
    validate(cfg)?;
    if cfg.bind_dn.trim().is_empty() {
        return Err("同步用户需要配置可写的服务账号（Bind DN）".into());
    }
    let base = if cfg.sync_base_dn.trim().is_empty() {
        cfg.user_base_dn.trim()
    } else {
        cfg.sync_base_dn.trim()
    };
    let attr = cfg.username_attr.trim();
    let mut ldap = connect(cfg).await?;
    service_bind(&mut ldap, cfg).await?;

    let mut report = Vec::with_capacity(users.len());
    for (username, hash) in users {
        let item = sync_one(&mut ldap, cfg, base, attr, username, hash, sync_password_hashes).await;
        report.push(item);
    }
    let _ = ldap.unbind().await;
    Ok(report)
}

async fn sync_one(
    ldap: &mut Ldap,
    cfg: &LdapSettings,
    base: &str,
    attr: &str,
    username: &str,
    hash: &str,
    sync_password_hashes: bool,
) -> SyncItem {
    let ok = |action| SyncItem {
        username: username.to_string(),
        action,
        error: String::new(),
    };
    let fail = |e: String| SyncItem {
        username: username.to_string(),
        action: "failed",
        error: e,
    };
    let existing = match find_user(ldap, cfg, username).await {
        Ok(v) => v,
        Err(e) => return fail(e),
    };
    let password = format!("{{ARGON2}}{hash}");
    match existing {
        Some((dn, _)) => {
            if !sync_password_hashes {
                return ok("skipped");
            }
            let mods = vec![Mod::Replace(
                "userPassword".to_string(),
                HashSet::from([password]),
            )];
            match ldap.with_timeout(TIMEOUT).modify(&dn, mods).await {
                Ok(res) if res.rc == 0 => ok("updated"),
                Ok(res) => fail(format!("修改被拒绝 (rc={}): {}", res.rc, res.text)),
                Err(e) => fail(format!("修改失败: {e}")),
            }
        }
        None => {
            let dn = format!("{attr}={},{base}", ldap3::dn_escape(username));
            let mut attrs: Vec<(String, HashSet<String>)> = vec![
                (
                    "objectClass".into(),
                    HashSet::from([
                        "top".to_string(),
                        "person".to_string(),
                        "organizationalPerson".to_string(),
                        "inetOrgPerson".to_string(),
                    ]),
                ),
                (attr.to_string(), HashSet::from([username.to_string()])),
                ("cn".into(), HashSet::from([username.to_string()])),
                ("sn".into(), HashSet::from([username.to_string()])),
            ];
            if sync_password_hashes {
                attrs.push(("userPassword".into(), HashSet::from([password])));
            }
            match ldap.with_timeout(TIMEOUT).add(&dn, attrs).await {
                Ok(res) if res.rc == 0 => ok("created"),
                Ok(res) => fail(format!("创建被拒绝 (rc={}): {}", res.rc, res.text)),
                Err(e) => fail(format!("创建失败: {e}")),
            }
        }
    }
}
