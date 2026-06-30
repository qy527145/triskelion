//! 管理后台（`/v1/admin/*`）。
//!
//! 通过 `ADMIN_TOKEN` 环境变量启用、并以该令牌鉴权（请求头 `X-Admin-Token`，
//! 或 `Authorization: Bearer <token>`），专供平台管理员使用。提供：
//! - 概览统计（用户 / 技能 / MCP / 调用量 / 热门工具 / 最近错误）；
//! - 资源清单（用户、技能、MCP、调用日志）；
//! - **全量资源包**导入 / 导出（`.tskpack` = tar + zstd），用于跨实例数据迁移。
//!
//! 资源包包含全部数据库行（用户、MCP、技能、加密凭据、调用日志）与按 sha256 内容
//! 寻址的全部压缩体 blob。凭据以「nonce + 密文」原样承载，迁移目标须共用同一
//! `master.key`（或 `TRISKELION_MASTER_KEY`）方可解密。

use std::collections::{BTreeSet, HashMap};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde::{Deserialize, Serialize};

use crate::archive::ZSTD_LEVEL;

use super::error::ApiError;
use super::routes::{db_err, now_string, now_unix};
use super::skills;
use super::AppState;

type S = State<Arc<AppState>>;

/// 资源包格式版本。
const PACK_VERSION: u32 = 1;

// ---------------------------------------------------------------------------
// 鉴权
// ---------------------------------------------------------------------------

/// 校验管理令牌。未配置 ADMIN_TOKEN → 503；令牌不匹配 → 401。
fn require_admin(state: &AppState, headers: &HeaderMap) -> Result<(), ApiError> {
    let expected = state.admin_token.as_deref().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "管理后台未启用：请在服务端设置 ADMIN_TOKEN 环境变量",
        )
    })?;
    let provided = headers
        .get("x-admin-token")
        .and_then(|v| v.to_str().ok())
        .or_else(|| {
            headers
                .get(header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.strip_prefix("Bearer ").or_else(|| s.strip_prefix("bearer ")))
        })
        .unwrap_or("");
    if ct_eq(provided.as_bytes(), expected.as_bytes()) {
        Ok(())
    } else {
        Err(ApiError::unauthorized("管理令牌无效"))
    }
}

/// 定长（对长度泄漏不敏感）的常量时间比较，避免按字节短路造成的计时侧信道。
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b) {
        diff |= x ^ y;
    }
    diff == 0
}

fn count(conn: &rusqlite::Connection, sql: &str) -> Result<i64, ApiError> {
    conn.query_row(sql, [], |r| r.get(0)).map_err(db_err)
}

// ---------------------------------------------------------------------------
// 概览统计
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct TopTool {
    tool: String,
    count: i64,
}

#[derive(Serialize)]
struct RecentError {
    tool: String,
    caller: String,
    error: String,
    at: String,
}

#[derive(Serialize)]
pub struct AdminStats {
    users: i64,
    skills: i64,
    skills_public: i64,
    mcps: i64,
    mcps_public: i64,
    secrets: i64,
    blobs: i64,
    blobs_bytes: i64,
    calls_total: i64,
    calls_24h: i64,
    calls_errors_24h: i64,
    top_tools: Vec<TopTool>,
    recent_errors: Vec<RecentError>,
    admin_enabled: bool,
    generated_at: String,
}

pub async fn stats(State(state): S, headers: HeaderMap) -> Result<Json<AdminStats>, ApiError> {
    require_admin(&state, &headers)?;
    let since = now_unix() - 86_400;
    let conn = state.db.lock().unwrap();

    let users = count(&conn, "SELECT COUNT(*) FROM users")?;
    let skills = count(&conn, "SELECT COUNT(*) FROM skills")?;
    let skills_public = count(&conn, "SELECT COUNT(*) FROM skills WHERE visibility='public'")?;
    let mcps = count(&conn, "SELECT COUNT(*) FROM mcps")?;
    let mcps_public = count(&conn, "SELECT COUNT(*) FROM mcps WHERE visibility='public'")?;
    let secrets = count(&conn, "SELECT COUNT(*) FROM secrets")?;
    let calls_total = count(&conn, "SELECT COUNT(*) FROM tool_calls")?;
    let calls_24h: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM tool_calls WHERE created_ts >= ?1",
            [since],
            |r| r.get(0),
        )
        .map_err(db_err)?;
    let calls_errors_24h: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM tool_calls WHERE created_ts >= ?1 AND ok = 0",
            [since],
            |r| r.get(0),
        )
        .map_err(db_err)?;

    let top_tools = {
        let mut stmt = conn
            .prepare(
                "SELECT mcp_name || '/' || tool AS t, COUNT(*) AS c
                 FROM tool_calls WHERE created_ts >= ?1
                 GROUP BY t ORDER BY c DESC, t LIMIT 8",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map([since], |r| {
                Ok(TopTool {
                    tool: r.get(0)?,
                    count: r.get(1)?,
                })
            })
            .map_err(db_err)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(db_err)?
    };

    let recent_errors = {
        let mut stmt = conn
            .prepare(
                "SELECT mcp_name || '/' || tool, caller, error, created_at
                 FROM tool_calls WHERE ok = 0 ORDER BY id DESC LIMIT 8",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map([], |r| {
                Ok(RecentError {
                    tool: r.get(0)?,
                    caller: r.get(1)?,
                    error: r.get(2)?,
                    at: r.get(3)?,
                })
            })
            .map_err(db_err)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(db_err)?
    };
    drop(conn);

    let (blobs, blobs_bytes) = blob_stats(&state);

    Ok(Json(AdminStats {
        users,
        skills,
        skills_public,
        mcps,
        mcps_public,
        secrets,
        blobs,
        blobs_bytes,
        calls_total,
        calls_24h,
        calls_errors_24h,
        top_tools,
        recent_errors,
        admin_enabled: true,
        generated_at: now_string(),
    }))
}

/// 统计 blobs 目录里的文件数与总字节（实际磁盘占用）。
fn blob_stats(state: &AppState) -> (i64, i64) {
    let mut n = 0i64;
    let mut bytes = 0i64;
    if let Ok(rd) = std::fs::read_dir(&state.blobs_dir) {
        for e in rd.flatten() {
            if let Ok(meta) = e.metadata() {
                if meta.is_file() {
                    n += 1;
                    bytes += meta.len() as i64;
                }
            }
        }
    }
    (n, bytes)
}

// ---------------------------------------------------------------------------
// 资源清单
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct AdminUser {
    username: String,
    created_at: String,
    skills: i64,
    mcps: i64,
    secrets: i64,
}

pub async fn users(State(state): S, headers: HeaderMap) -> Result<Json<Vec<AdminUser>>, ApiError> {
    require_admin(&state, &headers)?;
    let conn = state.db.lock().unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT u.username, u.created_at,
                    (SELECT COUNT(*) FROM skills s WHERE s.owner_id = u.id),
                    (SELECT COUNT(*) FROM mcps m WHERE m.owner_id = u.id),
                    (SELECT COUNT(*) FROM secrets x WHERE x.owner_id = u.id)
             FROM users u ORDER BY u.id",
        )
        .map_err(db_err)?;
    let rows = stmt
        .query_map([], |r| {
            Ok(AdminUser {
                username: r.get(0)?,
                created_at: r.get(1)?,
                skills: r.get(2)?,
                mcps: r.get(3)?,
                secrets: r.get(4)?,
            })
        })
        .map_err(db_err)?;
    let out = rows.collect::<rusqlite::Result<Vec<_>>>().map_err(db_err)?;
    Ok(Json(out))
}

#[derive(Serialize)]
pub struct AdminSkill {
    owner: String,
    name: String,
    category: String,
    visibility: String,
    version: String,
    description: String,
    archive_size: i64,
    has_archive: bool,
    updated_at: String,
}

pub async fn skills_all(
    State(state): S,
    headers: HeaderMap,
) -> Result<Json<Vec<AdminSkill>>, ApiError> {
    require_admin(&state, &headers)?;
    let conn = state.db.lock().unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT u.username, s.name, s.category, s.visibility, s.version, s.description,
                    s.archive_size, s.archive_sha256, s.updated_at
             FROM skills s JOIN users u ON u.id = s.owner_id
             ORDER BY s.updated_at DESC, s.name",
        )
        .map_err(db_err)?;
    let rows = stmt
        .query_map([], |r| {
            let sha: String = r.get(7)?;
            Ok(AdminSkill {
                owner: r.get(0)?,
                name: r.get(1)?,
                category: r.get(2)?,
                visibility: r.get(3)?,
                version: r.get(4)?,
                description: r.get(5)?,
                archive_size: r.get(6)?,
                has_archive: !sha.is_empty(),
                updated_at: r.get(8)?,
            })
        })
        .map_err(db_err)?;
    let out = rows.collect::<rusqlite::Result<Vec<_>>>().map_err(db_err)?;
    Ok(Json(out))
}

#[derive(Serialize)]
pub struct AdminMcp {
    owner: String,
    name: String,
    visibility: String,
    version: String,
    runtime: String,
    protocol: String,
    updated_at: String,
}

pub async fn mcps_all(State(state): S, headers: HeaderMap) -> Result<Json<Vec<AdminMcp>>, ApiError> {
    require_admin(&state, &headers)?;
    let conn = state.db.lock().unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT u.username, m.name, m.visibility, m.version, m.manifest, m.updated_at
             FROM mcps m JOIN users u ON u.id = m.owner_id
             ORDER BY m.updated_at DESC, m.name",
        )
        .map_err(db_err)?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
            ))
        })
        .map_err(db_err)?;
    let mut out = Vec::new();
    for row in rows {
        let (owner, name, visibility, version, manifest_json, updated_at) = row.map_err(db_err)?;
        // 仅取运行拓扑用于展示，损坏的 manifest 不致整表失败。
        let manifest: serde_json::Value = serde_json::from_str(&manifest_json).unwrap_or_default();
        let runtime = manifest
            .get("runtime")
            .and_then(|v| v.as_str())
            .unwrap_or("?")
            .to_string();
        let protocol = manifest
            .get("protocol")
            .and_then(|v| v.as_str())
            .unwrap_or("?")
            .to_string();
        out.push(AdminMcp {
            owner,
            name,
            visibility,
            version,
            runtime,
            protocol,
            updated_at,
        });
    }
    Ok(Json(out))
}

#[derive(Serialize)]
pub struct CallLog {
    caller: String,
    owner: String,
    mcp_name: String,
    tool: String,
    ok: bool,
    error: String,
    ms: i64,
    created_at: String,
}

pub async fn calls(State(state): S, headers: HeaderMap) -> Result<Json<Vec<CallLog>>, ApiError> {
    require_admin(&state, &headers)?;
    let conn = state.db.lock().unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT caller, owner, mcp_name, tool, ok, error, ms, created_at
             FROM tool_calls ORDER BY id DESC LIMIT 200",
        )
        .map_err(db_err)?;
    let rows = stmt
        .query_map([], |r| {
            Ok(CallLog {
                caller: r.get(0)?,
                owner: r.get(1)?,
                mcp_name: r.get(2)?,
                tool: r.get(3)?,
                ok: r.get::<_, i64>(4)? != 0,
                error: r.get(5)?,
                ms: r.get(6)?,
                created_at: r.get(7)?,
            })
        })
        .map_err(db_err)?;
    let out = rows.collect::<rusqlite::Result<Vec<_>>>().map_err(db_err)?;
    Ok(Json(out))
}

// ---------------------------------------------------------------------------
// 全量资源包：导入 / 导出
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
struct Pack {
    version: u32,
    exported_at: String,
    users: Vec<PackUser>,
    mcps: Vec<PackMcp>,
    skills: Vec<PackSkill>,
    secrets: Vec<PackSecret>,
    #[serde(default)]
    calls: Vec<PackCall>,
}

#[derive(Serialize, Deserialize)]
struct PackUser {
    username: String,
    password_hash: String,
    created_at: String,
}

#[derive(Serialize, Deserialize)]
struct PackMcp {
    owner: String,
    name: String,
    visibility: String,
    version: String,
    manifest: serde_json::Value,
    tools: serde_json::Value,
    updated_at: String,
}

#[derive(Serialize, Deserialize)]
struct PackSkill {
    owner: String,
    name: String,
    category: String,
    visibility: String,
    version: String,
    description: String,
    tags: serde_json::Value,
    skill_md: String,
    metadata: serde_json::Value,
    archive_sha256: String,
    archive_size: i64,
    updated_at: String,
}

#[derive(Serialize, Deserialize)]
struct PackSecret {
    owner: String,
    key: String,
    nonce_b64: String,
    ciphertext_b64: String,
    updated_at: String,
}

#[derive(Serialize, Deserialize)]
struct PackCall {
    caller: String,
    owner: String,
    mcp_name: String,
    tool: String,
    ok: i64,
    error: String,
    ms: i64,
    created_at: String,
    created_ts: i64,
}

/// 导出全量资源包：`GET /v1/admin/export` → `.tskpack`（tar + zstd）下载。
pub async fn export(State(state): S, headers: HeaderMap) -> Result<Response, ApiError> {
    require_admin(&state, &headers)?;
    let pack = collect_pack(&state)?;
    let json = serde_json::to_vec_pretty(&pack)
        .map_err(|e| ApiError::internal(format!("序列化资源包失败: {e}")))?;

    // 收集被引用的 blob（去重）及其落盘路径。
    let mut blob_files: Vec<(String, PathBuf)> = Vec::new();
    let mut seen = BTreeSet::new();
    for s in &pack.skills {
        if s.archive_sha256.is_empty() || !seen.insert(s.archive_sha256.clone()) {
            continue;
        }
        if let Some(path) = skills::find_blob(&state, &s.archive_sha256) {
            blob_files.push((s.archive_sha256.clone(), path));
        }
    }

    // tar + zstd 压缩较重，移出异步线程。
    let bytes = tokio::task::spawn_blocking(move || build_pack_tar(json, blob_files))
        .await
        .map_err(|e| ApiError::internal(format!("打包任务失败: {e}")))?
        .map_err(|e| ApiError::internal(format!("打包资源包失败: {e}")))?;

    let date = now_string();
    let date = date.get(..10).unwrap_or("export");
    let filename = format!("triskelion-export-{date}.tskpack");
    Ok((
        [
            (header::CONTENT_TYPE, "application/zstd".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{filename}\""),
            ),
        ],
        bytes,
    )
        .into_response())
}

#[derive(Serialize)]
pub struct ImportSummary {
    users: usize,
    mcps: usize,
    skills: usize,
    secrets: usize,
    calls: usize,
    blobs: usize,
    skipped: Vec<String>,
}

/// 导入全量资源包：`POST /v1/admin/import`，请求体为 `.tskpack` 字节。
/// 采用「合并/upsert」语义：按用户名、`(owner,name)`、`(owner,key)` 等自然键覆盖更新，
/// 不删除目标实例已有数据；blob 按 sha256 内容寻址，缺失才写入。
pub async fn import(
    State(state): S,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<ImportSummary>, ApiError> {
    require_admin(&state, &headers)?;
    if body.is_empty() {
        return Err(ApiError::bad_request("资源包为空"));
    }
    let body = body.to_vec();
    let (pack, blobs) = tokio::task::spawn_blocking(move || read_pack_tar(&body))
        .await
        .map_err(|e| ApiError::internal(format!("解包任务失败: {e}")))?
        .map_err(|e| ApiError::bad_request(format!("解析资源包失败: {e}")))?;

    if pack.version > PACK_VERSION {
        return Err(ApiError::bad_request(format!(
            "资源包版本 {} 高于本实例支持的 {PACK_VERSION}，请升级 triskelion",
            pack.version
        )));
    }

    // 先落盘 blob（内容寻址、校验 sha256，缺失才写）。
    let mut blob_n = 0usize;
    for (sha, bytes) in blobs {
        if skills::sha256_hex(&bytes) != sha {
            return Err(ApiError::bad_request(format!("blob 校验失败: {sha}")));
        }
        if skills::find_blob(&state, &sha).is_none() {
            let path = skills::blob_write_path(&state, &sha, &bytes);
            std::fs::write(&path, &bytes)
                .map_err(|e| ApiError::internal(format!("写入 blob 失败: {e}")))?;
            blob_n += 1;
        }
    }

    let mut summary = apply_pack(&state, &pack)?;
    summary.blobs = blob_n;
    Ok(Json(summary))
}

/// 读取全部数据库行，组装资源包（不含 blob 字节；blob 由调用方按 sha 落盘进 tar）。
fn collect_pack(state: &AppState) -> Result<Pack, ApiError> {
    let conn = state.db.lock().unwrap();

    let users = {
        let mut stmt = conn
            .prepare("SELECT username, password_hash, created_at FROM users ORDER BY id")
            .map_err(db_err)?;
        let rows = stmt
            .query_map([], |r| {
                Ok(PackUser {
                    username: r.get(0)?,
                    password_hash: r.get(1)?,
                    created_at: r.get(2)?,
                })
            })
            .map_err(db_err)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(db_err)?
    };

    let mcps = {
        let mut stmt = conn
            .prepare(
                "SELECT u.username, m.name, m.visibility, m.version, m.manifest, m.tools, m.updated_at
                 FROM mcps m JOIN users u ON u.id = m.owner_id ORDER BY m.id",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map([], |r| {
                let manifest: String = r.get(4)?;
                let tools: String = r.get(5)?;
                Ok(PackMcp {
                    owner: r.get(0)?,
                    name: r.get(1)?,
                    visibility: r.get(2)?,
                    version: r.get(3)?,
                    manifest: serde_json::from_str(&manifest).unwrap_or_default(),
                    tools: serde_json::from_str(&tools).unwrap_or(serde_json::json!([])),
                    updated_at: r.get(6)?,
                })
            })
            .map_err(db_err)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(db_err)?
    };

    let skills = {
        let mut stmt = conn
            .prepare(
                "SELECT u.username, s.name, s.category, s.visibility, s.version, s.description,
                        s.tags, s.skill_md, s.metadata, s.archive_sha256, s.archive_size, s.updated_at
                 FROM skills s JOIN users u ON u.id = s.owner_id ORDER BY s.id",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map([], |r| {
                let tags: String = r.get(6)?;
                let metadata: String = r.get(8)?;
                Ok(PackSkill {
                    owner: r.get(0)?,
                    name: r.get(1)?,
                    category: r.get(2)?,
                    visibility: r.get(3)?,
                    version: r.get(4)?,
                    description: r.get(5)?,
                    tags: serde_json::from_str(&tags).unwrap_or(serde_json::json!([])),
                    skill_md: r.get(7)?,
                    metadata: serde_json::from_str(&metadata).unwrap_or(serde_json::json!({})),
                    archive_sha256: r.get(9)?,
                    archive_size: r.get(10)?,
                    updated_at: r.get(11)?,
                })
            })
            .map_err(db_err)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(db_err)?
    };

    let secrets = {
        let mut stmt = conn
            .prepare(
                "SELECT u.username, s.key, s.nonce, s.ciphertext, s.updated_at
                 FROM secrets s JOIN users u ON u.id = s.owner_id ORDER BY s.id",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map([], |r| {
                let nonce: Vec<u8> = r.get(2)?;
                let ct: Vec<u8> = r.get(3)?;
                Ok(PackSecret {
                    owner: r.get(0)?,
                    key: r.get(1)?,
                    nonce_b64: STANDARD.encode(nonce),
                    ciphertext_b64: STANDARD.encode(ct),
                    updated_at: r.get(4)?,
                })
            })
            .map_err(db_err)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(db_err)?
    };

    let calls = {
        let mut stmt = conn
            .prepare(
                "SELECT caller, owner, mcp_name, tool, ok, error, ms, created_at, created_ts
                 FROM tool_calls ORDER BY id",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map([], |r| {
                Ok(PackCall {
                    caller: r.get(0)?,
                    owner: r.get(1)?,
                    mcp_name: r.get(2)?,
                    tool: r.get(3)?,
                    ok: r.get(4)?,
                    error: r.get(5)?,
                    ms: r.get(6)?,
                    created_at: r.get(7)?,
                    created_ts: r.get(8)?,
                })
            })
            .map_err(db_err)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(db_err)?
    };

    Ok(Pack {
        version: PACK_VERSION,
        exported_at: now_string(),
        users,
        mcps,
        skills,
        secrets,
        calls,
    })
}

/// 把资源包写入数据库（一个事务内 upsert）。
fn apply_pack(state: &AppState, pack: &Pack) -> Result<ImportSummary, ApiError> {
    let mut guard = state.db.lock().unwrap();
    let tx = guard.transaction().map_err(db_err)?;
    let mut skipped = Vec::new();

    for u in &pack.users {
        tx.execute(
            "INSERT INTO users(username, password_hash, created_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(username) DO UPDATE SET password_hash = excluded.password_hash",
            rusqlite::params![u.username, u.password_hash, u.created_at],
        )
        .map_err(db_err)?;
    }

    // 用户名 → 本地 id 映射（含目标实例既有用户）。
    let mut ids: HashMap<String, i64> = HashMap::new();
    {
        let mut stmt = tx.prepare("SELECT username, id FROM users").map_err(db_err)?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
            .map_err(db_err)?;
        for row in rows {
            let (name, id) = row.map_err(db_err)?;
            ids.insert(name, id);
        }
    }

    let mut mcps = 0usize;
    for m in &pack.mcps {
        let Some(&oid) = ids.get(&m.owner) else {
            skipped.push(format!("mcp {}/{} (owner 缺失)", m.owner, m.name));
            continue;
        };
        let manifest = m.manifest.to_string();
        let tools = m.tools.to_string();
        tx.execute(
            "INSERT INTO mcps(owner_id, name, visibility, version, manifest, tools, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(owner_id, name) DO UPDATE SET visibility=excluded.visibility,
                 version=excluded.version, manifest=excluded.manifest, tools=excluded.tools,
                 updated_at=excluded.updated_at",
            rusqlite::params![oid, m.name, m.visibility, m.version, manifest, tools, m.updated_at],
        )
        .map_err(db_err)?;
        mcps += 1;
    }

    let mut skills = 0usize;
    for s in &pack.skills {
        let Some(&oid) = ids.get(&s.owner) else {
            skipped.push(format!("skill {}/{} (owner 缺失)", s.owner, s.name));
            continue;
        };
        tx.execute(
            "INSERT INTO skills(owner_id, name, category, visibility, version, description,
                                tags, skill_md, metadata, archive_sha256, archive_size, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
             ON CONFLICT(owner_id, name) DO UPDATE SET category=excluded.category,
                 visibility=excluded.visibility, version=excluded.version,
                 description=excluded.description, tags=excluded.tags, skill_md=excluded.skill_md,
                 metadata=excluded.metadata, archive_sha256=excluded.archive_sha256,
                 archive_size=excluded.archive_size, updated_at=excluded.updated_at",
            rusqlite::params![
                oid,
                s.name,
                s.category,
                s.visibility,
                s.version,
                s.description,
                s.tags.to_string(),
                s.skill_md,
                s.metadata.to_string(),
                s.archive_sha256,
                s.archive_size,
                s.updated_at
            ],
        )
        .map_err(db_err)?;
        skills += 1;
    }

    let mut secrets = 0usize;
    for sec in &pack.secrets {
        let Some(&oid) = ids.get(&sec.owner) else {
            skipped.push(format!("secret {}/{} (owner 缺失)", sec.owner, sec.key));
            continue;
        };
        let nonce = STANDARD
            .decode(&sec.nonce_b64)
            .map_err(|e| ApiError::bad_request(format!("凭据 nonce 非法 base64: {e}")))?;
        let ct = STANDARD
            .decode(&sec.ciphertext_b64)
            .map_err(|e| ApiError::bad_request(format!("凭据密文非法 base64: {e}")))?;
        tx.execute(
            "INSERT INTO secrets(owner_id, key, nonce, ciphertext, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(owner_id, key) DO UPDATE SET nonce=excluded.nonce,
                 ciphertext=excluded.ciphertext, updated_at=excluded.updated_at",
            rusqlite::params![oid, sec.key, nonce, ct, sec.updated_at],
        )
        .map_err(db_err)?;
        secrets += 1;
    }

    // 调用日志：仅当目标为空表时导入，避免重复导入造成统计翻倍。
    let mut calls = 0usize;
    let existing: i64 = tx
        .query_row("SELECT COUNT(*) FROM tool_calls", [], |r| r.get(0))
        .map_err(db_err)?;
    if existing == 0 {
        for c in &pack.calls {
            tx.execute(
                "INSERT INTO tool_calls(caller, owner, mcp_name, tool, ok, error, ms, created_at, created_ts)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                rusqlite::params![
                    c.caller, c.owner, c.mcp_name, c.tool, c.ok, c.error, c.ms, c.created_at,
                    c.created_ts
                ],
            )
            .map_err(db_err)?;
            calls += 1;
        }
    } else if !pack.calls.is_empty() {
        skipped.push(format!("调用日志 {} 条（目标已有日志，跳过以免重复）", pack.calls.len()));
    }

    tx.commit().map_err(db_err)?;
    Ok(ImportSummary {
        users: pack.users.len(),
        mcps,
        skills,
        secrets,
        calls,
        blobs: 0,
        skipped,
    })
}

// ---------------------------------------------------------------------------
// tar + zstd 编解码
// ---------------------------------------------------------------------------

fn append_bytes<W: Write>(
    tar: &mut tar::Builder<W>,
    name: &str,
    data: &[u8],
) -> std::io::Result<()> {
    let mut header = tar::Header::new_gnu();
    header.set_size(data.len() as u64);
    header.set_mode(0o644);
    header.set_mtime(0);
    tar.append_data(&mut header, name, data)
}

fn build_pack_tar(json: Vec<u8>, blob_files: Vec<(String, PathBuf)>) -> anyhow::Result<Vec<u8>> {
    use anyhow::Context as _;
    let mut encoder = zstd::stream::Encoder::new(Vec::new(), ZSTD_LEVEL)?;
    let workers = std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(1);
    let _ = encoder.multithread(workers);
    let mut tar = tar::Builder::new(encoder);
    append_bytes(&mut tar, "manifest.json", &json)?;
    for (sha, path) in blob_files {
        let bytes = std::fs::read(&path).with_context(|| format!("读取 blob {sha}"))?;
        append_bytes(&mut tar, &format!("blobs/{sha}"), &bytes)?;
    }
    let encoder = tar.into_inner()?;
    Ok(encoder.finish()?)
}

fn read_pack_tar(body: &[u8]) -> anyhow::Result<(Pack, Vec<(String, Vec<u8>)>)> {
    use anyhow::Context as _;
    let dec = zstd::stream::Decoder::new(body)
        .context("资源包不是合法的 zstd（.tskpack 应为 tar+zstd）")?;
    let mut ar = tar::Archive::new(dec);
    let mut pack: Option<Pack> = None;
    let mut blobs = Vec::new();
    for entry in ar.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.to_string_lossy().into_owned();
        let mut data = Vec::new();
        entry.read_to_end(&mut data)?;
        if path == "manifest.json" {
            pack = Some(serde_json::from_slice(&data).context("解析 manifest.json")?);
        } else if let Some(sha) = path.strip_prefix("blobs/") {
            blobs.push((sha.to_string(), data));
        }
    }
    let pack = pack.ok_or_else(|| anyhow::anyhow!("资源包缺少 manifest.json"))?;
    Ok((pack, blobs))
}
