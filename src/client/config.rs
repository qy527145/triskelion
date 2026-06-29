//! 本地配置：Hub 地址与登录 token。存于用户配置目录的 `triskelion/config.json`。

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

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
        let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
        base.join("triskelion").join("config.json")
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

    /// 返回已登录的 (hub_url, token)，否则报错提示登录。
    pub fn require_auth(&self) -> Result<(String, String)> {
        match (&self.hub_url, &self.token) {
            (Some(h), Some(t)) => Ok((h.clone(), t.clone())),
            _ => bail!("尚未登录，请先执行 tsk login --hub <url>"),
        }
    }
}
