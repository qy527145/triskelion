//! 本地配置与数据目录。客户端数据统一存放在 `~/.tsk`（可用 `TRISKELION_CLIENT_DATA_DIR`
//! 覆盖）：`config.json`（Hub 地址与登录 token）、`secrets.json`（本地变量）。

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

/// 客户端数据目录：优先 `TRISKELION_CLIENT_DATA_DIR`，否则 `~/.tsk`。
pub fn client_data_dir() -> PathBuf {
    if let Ok(p) = std::env::var("TRISKELION_CLIENT_DATA_DIR") {
        let p = p.trim();
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".tsk")
}

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct Config {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hub_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
}

impl Config {
    pub fn path() -> PathBuf {
        if let Ok(p) = std::env::var("TRISKELION_CONFIG") {
            return PathBuf::from(p);
        }
        client_data_dir().join("config.json")
    }

    pub fn load() -> Self {
        let path = Self::path();
        match std::fs::read_to_string(&path) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => Config::default(),
        }
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("创建配置目录 {}", dir.display()))?;
        }
        let s = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, s).with_context(|| format!("写入配置 {}", path.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }

    /// 解析 Hub 地址：配置文件优先，否则回退环境变量 `TRISKELION_HUB`。
    /// 未登录用户也可借此访问公开市场（无需 token）。
    pub fn hub(&self) -> Option<String> {
        self.hub_url.clone().or_else(|| {
            std::env::var("TRISKELION_HUB")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        })
    }

    /// 是否已登录（持有 token）。
    pub fn logged_in(&self) -> bool {
        self.token.is_some()
    }

    /// 返回已登录的 (hub_url, token)，否则报错提示登录。
    pub fn require_auth(&self) -> Result<(String, String)> {
        match (self.hub(), &self.token) {
            (Some(h), Some(t)) => Ok((h, t.clone())),
            _ => bail!("尚未登录，请先执行 tsk login --hub <url>"),
        }
    }
}
