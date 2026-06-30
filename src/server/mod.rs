//! Triskelion Hub 服务端。
//!
//! 多拓扑反向代理网关的最小闭环实现：用户名密码注册/登录（JWT）、MCP 注册表、
//! AES-256-GCM 加密的凭据池，以及 `tsk run` 的凭据缝合解析接口。
//! 持久化用 SQLite（rusqlite，bundled）。

mod auth;
mod admin;
mod crypto;
mod db;
mod error;
mod routes;
mod skills;
mod web;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use rusqlite::Connection;

/// 进程全局状态。
pub struct AppState {
    pub db: Mutex<Connection>,
    /// JWT 签名密钥。
    pub jwt_secret: Vec<u8>,
    /// AES-256-GCM 主密钥（32 字节），加密凭据池。
    pub master_key: [u8; 32],
    /// 技能包压缩体落盘目录（按 sha256 内容寻址）。
    pub blobs_dir: PathBuf,
    /// 管理后台令牌（取自 `ADMIN_TOKEN` 环境变量）。为 None 时管理后台 API 禁用。
    pub admin_token: Option<String>,
}

/// 启动 Hub。自建多线程 tokio runtime 并阻塞，bin 侧保持同步 main。
pub fn run() -> Result<()> {
    let data_dir = server_data_dir();
    std::fs::create_dir_all(&data_dir)
        .with_context(|| format!("创建数据目录 {}", data_dir.display()))?;

    let db_path = data_dir.join("hub.db");
    let conn = Connection::open(&db_path)
        .with_context(|| format!("打开数据库 {}", db_path.display()))?;
    db::init(&conn)?;

    let jwt_secret = load_or_create_key(&data_dir.join("jwt.key"), 32)?;
    let master_key_vec = match std::env::var("TRISKELION_MASTER_KEY") {
        Ok(b64) => {
            use base64::{Engine, engine::general_purpose::STANDARD};
            STANDARD
                .decode(b64.trim())
                .context("TRISKELION_MASTER_KEY 不是合法 base64")?
        }
        Err(_) => load_or_create_key(&data_dir.join("master.key"), 32)?,
    };
    anyhow::ensure!(
        master_key_vec.len() == 32,
        "主密钥必须为 32 字节，实际 {} 字节",
        master_key_vec.len()
    );
    let mut master_key = [0u8; 32];
    master_key.copy_from_slice(&master_key_vec);

    let blobs_dir = data_dir.join("blobs");
    std::fs::create_dir_all(&blobs_dir)
        .with_context(|| format!("创建技能包目录 {}", blobs_dir.display()))?;

    // 管理后台令牌：仅当设置了 ADMIN_TOKEN 才启用 /v1/admin 接口。
    let admin_token = std::env::var("ADMIN_TOKEN")
        .ok()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty());

    let state = Arc::new(AppState {
        db: Mutex::new(conn),
        jwt_secret,
        master_key,
        blobs_dir,
        admin_token,
    });

    let bind = std::env::var("TRISKELION_BIND").unwrap_or_else(|_| "127.0.0.1:8787".into());

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("构建 tokio runtime")?;

    rt.block_on(async move {
        let admin_enabled = state.admin_token.is_some();
        let app = routes::router(state);
        let listener = tokio::net::TcpListener::bind(&bind)
            .await
            .with_context(|| format!("绑定 {bind}"))?;
        let local = listener.local_addr()?;
        println!("triskelion hub listening on http://{local}");
        println!("  data dir: {}", data_dir.display());
        if admin_enabled {
            println!("  admin panel: enabled (ADMIN_TOKEN set) → http://{local}/#admin");
        } else {
            println!("  admin panel: disabled (set ADMIN_TOKEN to enable)");
        }
        axum::serve(listener, app)
            .with_graceful_shutdown(shutdown_signal())
            .await
            .context("axum serve")?;
        Ok::<_, anyhow::Error>(())
    })
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    println!("\ntriskelion hub shutting down");
}

/// 服务端数据目录：优先 `TRISKELION_SERVER_DATA_DIR`，兼容旧的 `TRISKELION_DATA_DIR`，
/// 否则默认用户主目录下的 `~/.triskelion`。
fn server_data_dir() -> PathBuf {
    if let Ok(p) = std::env::var("TRISKELION_SERVER_DATA_DIR").or_else(|_| std::env::var("TRISKELION_DATA_DIR")) {
        let p = p.trim();
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".triskelion")
}

/// 读取密钥文件，不存在则生成随机字节并写入（0600 权限）。
fn load_or_create_key(path: &std::path::Path, len: usize) -> Result<Vec<u8>> {
    if let Ok(bytes) = std::fs::read(path)
        && bytes.len() == len
    {
        return Ok(bytes);
    }
    use rand::RngCore;
    let mut buf = vec![0u8; len];
    rand::rngs::OsRng.fill_bytes(&mut buf);
    std::fs::write(path, &buf).with_context(|| format!("写入密钥 {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(buf)
}
