//! 本地变量（凭据）存储：未登录也可用。存于 `~/.tsk/secrets.json`（0600 权限）。
//!
//! 与线上「我的变量」相对：本地变量优先级更高（解析运行时本地覆盖线上）。登录后写变量
//! 会同时落本地与线上；未登录则仅落本地。

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::config::client_data_dir;

#[derive(Serialize, Deserialize, Clone)]
pub struct LocalSecret {
    pub value: String,
    #[serde(default)]
    pub updated_at: String,
}

/// 本地变量集合。文件内容即 `{ "KEY": { "value": ..., "updated_at": ... } }`。
#[derive(Default)]
pub struct LocalSecrets {
    map: BTreeMap<String, LocalSecret>,
}

impl LocalSecrets {
    pub fn file_path() -> PathBuf {
        client_data_dir().join("secrets.json")
    }

    pub fn load() -> Self {
        match std::fs::read_to_string(Self::file_path()) {
            Ok(s) => LocalSecrets {
                map: serde_json::from_str(&s).unwrap_or_default(),
            },
            Err(_) => LocalSecrets::default(),
        }
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::file_path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("创建数据目录 {}", dir.display()))?;
        }
        let s = serde_json::to_string_pretty(&self.map)?;
        std::fs::write(&path, s).with_context(|| format!("写入本地变量 {}", path.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }

    pub fn set(&mut self, key: &str, value: &str) {
        self.map.insert(
            key.to_string(),
            LocalSecret {
                value: value.to_string(),
                updated_at: now_string(),
            },
        );
    }

    pub fn remove(&mut self, key: &str) -> bool {
        self.map.remove(key).is_some()
    }

    pub fn get(&self, key: &str) -> Option<&LocalSecret> {
        self.map.get(key)
    }

    pub fn keys(&self) -> impl Iterator<Item = &String> {
        self.map.keys()
    }

    /// 变量名 → 值（供运行时凭据缝合）。
    pub fn values_map(&self) -> BTreeMap<String, String> {
        self.map
            .iter()
            .map(|(k, v)| (k.clone(), v.value.clone()))
            .collect()
    }
}

/// 当前时间 `YYYY-MM-DD HH:MM:SS UTC`（不引入 chrono，与服务端格式一致）。
fn now_string() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = (secs / 86400) as i64;
    let rem = secs % 86400;
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let (y, mo, d) = civil_from_days(days);
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{m:02}:{s:02} UTC")
}

/// days = 1970-01-01 起的天数 → (year, month, day)。Howard Hinnant 算法。
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (y + if m <= 2 { 1 } else { 0 }, m, d)
}
