//! Triskelion Hub 服务端。
//!
//! 多拓扑反向代理网关的最小闭环实现：用户名密码注册/登录（JWT）、MCP 注册表、
//! AES-256-GCM 加密的凭据池，以及 `tsk run` 的凭据缝合解析接口。
//! 持久化用 SQLite（rusqlite，bundled）。

mod auth;
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
}

/// 启动 Hub。自建多线程 tokio runtime 并阻塞，bin 侧保持同步 main。
pub fn run() -> Result<()> {
    let data_dir = std::env::var("TRISKELION_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("triskelion-data"));
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

    let state = Arc::new(AppState {
        db: Mutex::new(conn),
        jwt_secret,
        master_key,
        blobs_dir,
    });

    let bind = std::env::var("TRISKELION_BIND").unwrap_or_else(|_| "127.0.0.1:8787".into());

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("构建 tokio runtime")?;

    rt.block_on(async move {
        let app = routes::router(state);
        let listener = tokio::net::TcpListener::bind(&bind)
            .await
            .with_context(|| format!("绑定 {bind}"))?;
        let local = listener.local_addr()?;
        println!("triskelion hub listening on http://{local}");
        println!("  data dir: {}", data_dir.display());
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
