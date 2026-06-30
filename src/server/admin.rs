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
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};

use crate::archive::ZSTD_LEVEL;

use super::auth;
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

#[derive(Serialize, Clone)]
pub struct GroupBrief {
    id: i64,
    name: String,
}

#[derive(Serialize)]
pub struct AdminUser {
    id: i64,
    username: String,
    groups: Vec<GroupBrief>,
    created_at: String,
    skills: i64,
    mcps: i64,
    secrets: i64,
}

pub async fn users(State(state): S, headers: HeaderMap) -> Result<Json<Vec<AdminUser>>, ApiError> {
    require_admin(&state, &headers)?;
    let conn = state.db.lock().unwrap();

    // 先聚合每个用户的分组（多对多），再装配用户行。
    let mut by_user: HashMap<i64, Vec<GroupBrief>> = HashMap::new();
    {
        let mut stmt = conn
            .prepare(
                "SELECT ug.user_id, g.id, g.name FROM user_groups ug
                 JOIN groups g ON g.id = ug.group_id ORDER BY g.name",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map([], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?))
            })
            .map_err(db_err)?;
        for row in rows {
            let (uid, gid, name) = row.map_err(db_err)?;
            by_user.entry(uid).or_default().push(GroupBrief { id: gid, name });
        }
    }

    let mut stmt = conn
        .prepare(
            "SELECT u.id, u.username, u.created_at,
                    (SELECT COUNT(*) FROM skills s WHERE s.owner_id = u.id),
                    (SELECT COUNT(*) FROM mcps m WHERE m.owner_id = u.id),
                    (SELECT COUNT(*) FROM secrets x WHERE x.owner_id = u.id)
             FROM users u ORDER BY u.id",
        )
        .map_err(db_err)?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, i64>(5)?,
            ))
        })
        .map_err(db_err)?;
    let mut out = Vec::new();
    for row in rows {
        let (id, username, created_at, skills, mcps, secrets) = row.map_err(db_err)?;
        out.push(AdminUser {
            groups: by_user.remove(&id).unwrap_or_default(),
            id,
            username,
            created_at,
            skills,
            mcps,
            secrets,
        });
    }
    Ok(Json(out))
}

#[derive(Serialize)]
pub struct AdminSkill {
    owner: String,
    name: String,
    category: String,
    visibility: String,
    group_visibility: String,
    version: String,
    description: String,
    archive_size: i64,
    has_archive: bool,
    labels: Vec<LabelBrief>,
    updated_at: String,
}

#[derive(Serialize, Clone)]
pub struct LabelBrief {
    id: i64,
    name: String,
}

/// 全量映射：资源 id → 已分配标签（id+name）。供管理列表/详情一次性装配。
fn all_label_briefs(
    conn: &rusqlite::Connection,
    junction: &str,
    fk: &str,
) -> HashMap<i64, Vec<LabelBrief>> {
    let mut map: HashMap<i64, Vec<LabelBrief>> = HashMap::new();
    let sql = format!(
        "SELECT j.{fk}, l.id, l.name FROM {junction} j JOIN labels l ON l.id = j.label_id
         ORDER BY l.name"
    );
    if let Ok(mut stmt) = conn.prepare(&sql) {
        if let Ok(rows) = stmt.query_map([], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?))
        }) {
            for (rid, lid, name) in rows.flatten() {
                map.entry(rid).or_default().push(LabelBrief { id: lid, name });
            }
        }
    }
    map
}

pub async fn skills_all(
    State(state): S,
    headers: HeaderMap,
) -> Result<Json<Vec<AdminSkill>>, ApiError> {
    require_admin(&state, &headers)?;
    let conn = state.db.lock().unwrap();
    let mut label_map = all_label_briefs(&conn, "skill_labels", "skill_id");
    let mut stmt = conn
        .prepare(
            "SELECT u.username, s.name, s.category, s.visibility, s.version, s.description,
                    s.archive_size, s.archive_sha256, s.updated_at, s.group_visibility, s.id
             FROM skills s JOIN users u ON u.id = s.owner_id
             ORDER BY s.updated_at DESC, s.name",
        )
        .map_err(db_err)?;
    let rows = stmt
        .query_map([], |r| {
            let sha: String = r.get(7)?;
            Ok((
                AdminSkill {
                    owner: r.get(0)?,
                    name: r.get(1)?,
                    category: r.get(2)?,
                    visibility: r.get(3)?,
                    version: r.get(4)?,
                    description: r.get(5)?,
                    archive_size: r.get(6)?,
                    has_archive: !sha.is_empty(),
                    updated_at: r.get(8)?,
                    group_visibility: r.get(9)?,
                    labels: Vec::new(),
                },
                r.get::<_, i64>(10)?,
            ))
        })
        .map_err(db_err)?;
    let mut out = Vec::new();
    for row in rows {
        let (mut s, id) = row.map_err(db_err)?;
        s.labels = label_map.remove(&id).unwrap_or_default();
        out.push(s);
    }
    Ok(Json(out))
}

#[derive(Serialize)]
pub struct AdminMcp {
    owner: String,
    name: String,
    visibility: String,
    group_visibility: String,
    version: String,
    runtime: String,
    protocol: String,
    labels: Vec<LabelBrief>,
    updated_at: String,
}

pub async fn mcps_all(State(state): S, headers: HeaderMap) -> Result<Json<Vec<AdminMcp>>, ApiError> {
    require_admin(&state, &headers)?;
    let conn = state.db.lock().unwrap();
    let mut label_map = all_label_briefs(&conn, "mcp_labels", "mcp_id");
    let mut stmt = conn
        .prepare(
            "SELECT u.username, m.name, m.visibility, m.version, m.manifest, m.updated_at,
                    m.group_visibility, m.id
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
                r.get::<_, String>(6)?,
                r.get::<_, i64>(7)?,
            ))
        })
        .map_err(db_err)?;
    let mut out = Vec::new();
    for row in rows {
        let (owner, name, visibility, version, manifest_json, updated_at, group_visibility, id) =
            row.map_err(db_err)?;
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
            group_visibility,
            version,
            runtime,
            protocol,
            labels: label_map.remove(&id).unwrap_or_default(),
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
// 分组 CRUD
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct AdminGroup {
    id: i64,
    name: String,
    description: String,
    users: i64,
    created_at: String,
}

pub async fn groups(State(state): S, headers: HeaderMap) -> Result<Json<Vec<AdminGroup>>, ApiError> {
    require_admin(&state, &headers)?;
    let conn = state.db.lock().unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT g.id, g.name, g.description, g.created_at,
                    (SELECT COUNT(*) FROM user_groups ug WHERE ug.group_id = g.id)
             FROM groups g ORDER BY g.name",
        )
        .map_err(db_err)?;
    let rows = stmt
        .query_map([], |r| {
            Ok(AdminGroup {
                id: r.get(0)?,
                name: r.get(1)?,
                description: r.get(2)?,
                created_at: r.get(3)?,
                users: r.get(4)?,
            })
        })
        .map_err(db_err)?;
    let out = rows.collect::<rusqlite::Result<Vec<_>>>().map_err(db_err)?;
    Ok(Json(out))
}

#[derive(Deserialize)]
pub struct GroupReq {
    name: String,
    #[serde(default)]
    description: String,
}

fn valid_group_name(s: &str) -> bool {
    let s = s.trim();
    !s.is_empty() && s.chars().count() <= 64
}

pub async fn group_create(
    State(state): S,
    headers: HeaderMap,
    Json(req): Json<GroupReq>,
) -> Result<Json<AdminGroup>, ApiError> {
    require_admin(&state, &headers)?;
    let name = req.name.trim().to_string();
    if !valid_group_name(&name) {
        return Err(ApiError::bad_request("分组名不能为空且长度 ≤64"));
    }
    let now = now_string();
    let conn = state.db.lock().unwrap();
    let exists: bool = conn
        .query_row("SELECT 1 FROM groups WHERE name = ?1", [&name], |_| Ok(true))
        .optional()
        .map_err(db_err)?
        .unwrap_or(false);
    if exists {
        return Err(ApiError::conflict("分组名已存在"));
    }
    conn.execute(
        "INSERT INTO groups(name, description, created_at) VALUES (?1, ?2, ?3)",
        rusqlite::params![name, req.description.trim(), now],
    )
    .map_err(db_err)?;
    let id = conn.last_insert_rowid();
    Ok(Json(AdminGroup {
        id,
        name,
        description: req.description.trim().to_string(),
        users: 0,
        created_at: now,
    }))
}

#[derive(Deserialize)]
pub struct GroupPatch {
    name: Option<String>,
    description: Option<String>,
}

pub async fn group_update(
    State(state): S,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Json(req): Json<GroupPatch>,
) -> Result<StatusCode, ApiError> {
    require_admin(&state, &headers)?;
    let conn = state.db.lock().unwrap();
    if let Some(name) = req.name.as_deref().map(str::trim) {
        if !valid_group_name(name) {
            return Err(ApiError::bad_request("分组名不能为空且长度 ≤64"));
        }
        let clash: bool = conn
            .query_row(
                "SELECT 1 FROM groups WHERE name = ?1 AND id != ?2",
                rusqlite::params![name, id],
                |_| Ok(true),
            )
            .optional()
            .map_err(db_err)?
            .unwrap_or(false);
        if clash {
            return Err(ApiError::conflict("分组名已存在"));
        }
        conn.execute(
            "UPDATE groups SET name = ?1 WHERE id = ?2",
            rusqlite::params![name, id],
        )
        .map_err(db_err)?;
    }
    if let Some(desc) = req.description.as_deref() {
        conn.execute(
            "UPDATE groups SET description = ?1 WHERE id = ?2",
            rusqlite::params![desc.trim(), id],
        )
        .map_err(db_err)?;
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn group_delete(
    State(state): S,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    require_admin(&state, &headers)?;
    let conn = state.db.lock().unwrap();
    // 解除该分组的全部成员关联（FK 亦会级联，这里显式清理以防 FK 未启用）。
    conn.execute("DELETE FROM user_groups WHERE group_id = ?1", [id])
        .map_err(db_err)?;
    let n = conn
        .execute("DELETE FROM groups WHERE id = ?1", [id])
        .map_err(db_err)?;
    if n == 0 {
        return Err(ApiError::not_found("未找到该分组"));
    }
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// 标签（labels）CRUD
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct AdminLabel {
    id: i64,
    name: String,
    skills: i64,
    mcps: i64,
    created_at: String,
}

pub async fn labels(State(state): S, headers: HeaderMap) -> Result<Json<Vec<AdminLabel>>, ApiError> {
    require_admin(&state, &headers)?;
    let conn = state.db.lock().unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT l.id, l.name, l.created_at,
                    (SELECT COUNT(*) FROM skill_labels sl WHERE sl.label_id = l.id),
                    (SELECT COUNT(*) FROM mcp_labels ml WHERE ml.label_id = l.id)
             FROM labels l ORDER BY l.name",
        )
        .map_err(db_err)?;
    let rows = stmt
        .query_map([], |r| {
            Ok(AdminLabel {
                id: r.get(0)?,
                name: r.get(1)?,
                created_at: r.get(2)?,
                skills: r.get(3)?,
                mcps: r.get(4)?,
            })
        })
        .map_err(db_err)?;
    let out = rows.collect::<rusqlite::Result<Vec<_>>>().map_err(db_err)?;
    Ok(Json(out))
}

#[derive(Deserialize)]
pub struct LabelReq {
    name: String,
}

fn valid_label_name(s: &str) -> bool {
    let s = s.trim();
    !s.is_empty() && s.chars().count() <= 32
}

pub async fn label_create(
    State(state): S,
    headers: HeaderMap,
    Json(req): Json<LabelReq>,
) -> Result<Json<AdminLabel>, ApiError> {
    require_admin(&state, &headers)?;
    let name = req.name.trim().to_string();
    if !valid_label_name(&name) {
        return Err(ApiError::bad_request("标签名不能为空且长度 ≤32"));
    }
    let now = now_string();
    let conn = state.db.lock().unwrap();
    let exists: bool = conn
        .query_row("SELECT 1 FROM labels WHERE name = ?1", [&name], |_| Ok(true))
        .optional()
        .map_err(db_err)?
        .unwrap_or(false);
    if exists {
        return Err(ApiError::conflict("标签名已存在"));
    }
    conn.execute(
        "INSERT INTO labels(name, created_at) VALUES (?1, ?2)",
        rusqlite::params![name, now],
    )
    .map_err(db_err)?;
    let id = conn.last_insert_rowid();
    Ok(Json(AdminLabel {
        id,
        name,
        skills: 0,
        mcps: 0,
        created_at: now,
    }))
}

pub async fn label_update(
    State(state): S,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Json(req): Json<LabelReq>,
) -> Result<StatusCode, ApiError> {
    require_admin(&state, &headers)?;
    let name = req.name.trim().to_string();
    if !valid_label_name(&name) {
        return Err(ApiError::bad_request("标签名不能为空且长度 ≤32"));
    }
    let conn = state.db.lock().unwrap();
    let clash: bool = conn
        .query_row(
            "SELECT 1 FROM labels WHERE name = ?1 AND id != ?2",
            rusqlite::params![name, id],
            |_| Ok(true),
        )
        .optional()
        .map_err(db_err)?
        .unwrap_or(false);
    if clash {
        return Err(ApiError::conflict("标签名已存在"));
    }
    let n = conn
        .execute(
            "UPDATE labels SET name = ?1 WHERE id = ?2",
            rusqlite::params![name, id],
        )
        .map_err(db_err)?;
    if n == 0 {
        return Err(ApiError::not_found("未找到该标签"));
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn label_delete(
    State(state): S,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    require_admin(&state, &headers)?;
    let conn = state.db.lock().unwrap();
    // 解除该标签在各资源上的关联（FK 亦会级联，这里显式清理以防 FK 未启用）。
    conn.execute("DELETE FROM skill_labels WHERE label_id = ?1", [id])
        .map_err(db_err)?;
    conn.execute("DELETE FROM mcp_labels WHERE label_id = ?1", [id])
        .map_err(db_err)?;
    let n = conn
        .execute("DELETE FROM labels WHERE id = ?1", [id])
        .map_err(db_err)?;
    if n == 0 {
        return Err(ApiError::not_found("未找到该标签"));
    }
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// 用户 CRUD
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct CreateUserReq {
    username: String,
    password: String,
    #[serde(default)]
    group_ids: Vec<i64>,
}

pub async fn user_create(
    State(state): S,
    headers: HeaderMap,
    Json(req): Json<CreateUserReq>,
) -> Result<StatusCode, ApiError> {
    require_admin(&state, &headers)?;
    let username = req.username.trim().to_string();
    if !super::routes::is_valid_username(&username) {
        return Err(ApiError::bad_request(
            "用户名仅允许字母、数字、_、-，且长度 1..=64",
        ));
    }
    if req.password.len() < 6 {
        return Err(ApiError::bad_request("密码至少 6 位"));
    }
    let hash = auth::hash_password(&req.password).map_err(|e| ApiError::internal(e.to_string()))?;
    let now = now_string();
    let conn = state.db.lock().unwrap();
    for gid in &req.group_ids {
        ensure_group_exists(&conn, *gid)?;
    }
    let exists: bool = conn
        .query_row("SELECT 1 FROM users WHERE username = ?1", [&username], |_| {
            Ok(true)
        })
        .optional()
        .map_err(db_err)?
        .unwrap_or(false);
    if exists {
        return Err(ApiError::conflict("用户名已存在"));
    }
    conn.execute(
        "INSERT INTO users(username, password_hash, created_at) VALUES (?1, ?2, ?3)",
        rusqlite::params![username, hash, now],
    )
    .map_err(db_err)?;
    let uid = conn.last_insert_rowid();
    replace_user_groups(&conn, uid, &req.group_ids)?;
    Ok(StatusCode::CREATED)
}

#[derive(Deserialize)]
pub struct UserPatch {
    /// 非空则重置密码。
    #[serde(default)]
    password: Option<String>,
    /// 提供则整体覆盖分组归属（空数组=移出全部分组）；缺省则不改动分组。
    #[serde(default)]
    group_ids: Option<Vec<i64>>,
}

pub async fn user_update(
    State(state): S,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Json(req): Json<UserPatch>,
) -> Result<StatusCode, ApiError> {
    require_admin(&state, &headers)?;
    let conn = state.db.lock().unwrap();
    let exists: bool = conn
        .query_row("SELECT 1 FROM users WHERE id = ?1", [id], |_| Ok(true))
        .optional()
        .map_err(db_err)?
        .unwrap_or(false);
    if !exists {
        return Err(ApiError::not_found("未找到该用户"));
    }
    if let Some(gids) = &req.group_ids {
        for gid in gids {
            ensure_group_exists(&conn, *gid)?;
        }
        replace_user_groups(&conn, id, gids)?;
    }
    if let Some(pw) = req.password.as_deref().filter(|p| !p.is_empty()) {
        if pw.len() < 6 {
            return Err(ApiError::bad_request("密码至少 6 位"));
        }
        let hash = auth::hash_password(pw).map_err(|e| ApiError::internal(e.to_string()))?;
        conn.execute(
            "UPDATE users SET password_hash = ?1 WHERE id = ?2",
            rusqlite::params![hash, id],
        )
        .map_err(db_err)?;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// 整体覆盖某用户的分组归属：先清空再按给定 id 集重建（去重）。
fn replace_user_groups(conn: &rusqlite::Connection, uid: i64, gids: &[i64]) -> Result<(), ApiError> {
    conn.execute("DELETE FROM user_groups WHERE user_id = ?1", [uid])
        .map_err(db_err)?;
    for gid in gids {
        conn.execute(
            "INSERT OR IGNORE INTO user_groups(user_id, group_id) VALUES (?1, ?2)",
            rusqlite::params![uid, gid],
        )
        .map_err(db_err)?;
    }
    Ok(())
}

pub async fn user_delete(
    State(state): S,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    require_admin(&state, &headers)?;
    let conn = state.db.lock().unwrap();
    // 技能压缩体 blob 的清理：先收集该用户引用且无人共享的 sha，删除后 GC。
    let orphan_shas = collect_user_skill_blobs(&conn, id)?;
    let n = conn
        .execute("DELETE FROM users WHERE id = ?1", [id])
        .map_err(db_err)?;
    if n == 0 {
        return Err(ApiError::not_found("未找到该用户"));
    }
    // 删除用户级联清掉其 skills（外键 ON DELETE CASCADE），此后无人引用的 blob 落盘清理。
    for sha in orphan_shas {
        let still: bool = conn
            .query_row(
                "SELECT 1 FROM skills WHERE archive_sha256 = ?1 LIMIT 1",
                [&sha],
                |_| Ok(true),
            )
            .optional()
            .map_err(db_err)?
            .unwrap_or(false);
        if !still {
            if let Some(p) = skills::find_blob(&state, &sha) {
                let _ = std::fs::remove_file(p);
            }
        }
    }
    Ok(StatusCode::NO_CONTENT)
}

fn ensure_group_exists(conn: &rusqlite::Connection, gid: i64) -> Result<(), ApiError> {
    let ok: bool = conn
        .query_row("SELECT 1 FROM groups WHERE id = ?1", [gid], |_| Ok(true))
        .optional()
        .map_err(db_err)?
        .unwrap_or(false);
    if ok {
        Ok(())
    } else {
        Err(ApiError::bad_request("指定的分组不存在"))
    }
}

/// 收集某用户名下技能引用的全部非空 sha256（去重），供删除后 GC 判定。
fn collect_user_skill_blobs(conn: &rusqlite::Connection, uid: i64) -> Result<Vec<String>, ApiError> {
    let mut stmt = conn
        .prepare("SELECT DISTINCT archive_sha256 FROM skills WHERE owner_id = ?1 AND archive_sha256 != ''")
        .map_err(db_err)?;
    let rows = stmt
        .query_map([uid], |r| r.get::<_, String>(0))
        .map_err(db_err)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>().map_err(db_err)?)
}

// ---------------------------------------------------------------------------
// 市场资源（技能 / MCP）的可见性与分组配置 + 删除
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct ResourcePatch {
    /// private / public。
    #[serde(default)]
    visibility: Option<String>,
    /// 字符串 "all" 或分组 id 数组（如 [1,2]）。
    #[serde(default)]
    group_visibility: Option<serde_json::Value>,
    /// 提供则整体覆盖该资源的受管标签（空数组=清空）；缺省则不改动。
    #[serde(default)]
    label_ids: Option<Vec<i64>>,
}

/// 整体覆盖某资源的受管标签：先清空再按给定 id 集重建（去重、校验标签存在）。
/// `junction`/`fk` 为内部常量（skill_labels/skill_id 等）。
fn replace_resource_labels(
    conn: &rusqlite::Connection,
    junction: &str,
    fk: &str,
    rid: i64,
    label_ids: &[i64],
) -> Result<(), ApiError> {
    for lid in label_ids {
        let ok: bool = conn
            .query_row("SELECT 1 FROM labels WHERE id = ?1", [lid], |_| Ok(true))
            .optional()
            .map_err(db_err)?
            .unwrap_or(false);
        if !ok {
            return Err(ApiError::bad_request("指定的标签不存在"));
        }
    }
    conn.execute(
        &format!("DELETE FROM {junction} WHERE {fk} = ?1"),
        [rid],
    )
    .map_err(db_err)?;
    for lid in label_ids {
        conn.execute(
            &format!("INSERT OR IGNORE INTO {junction}({fk}, label_id) VALUES (?1, ?2)"),
            rusqlite::params![rid, lid],
        )
        .map_err(db_err)?;
    }
    Ok(())
}

/// 把前端传入的 group_visibility 归一化为存储字符串：'all' 或紧凑 JSON 数组。
fn normalize_group_vis(v: &serde_json::Value) -> Result<String, ApiError> {
    if let Some(s) = v.as_str() {
        if s == "all" {
            return Ok("all".to_string());
        }
        return Err(ApiError::bad_request("group_visibility 字符串只能是 \"all\""));
    }
    if let Some(arr) = v.as_array() {
        let mut ids: Vec<i64> = Vec::new();
        for it in arr {
            let id = it
                .as_i64()
                .ok_or_else(|| ApiError::bad_request("group_visibility 数组元素必须是分组 id"))?;
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        return serde_json::to_string(&ids).map_err(|e| ApiError::internal(e.to_string()));
    }
    Err(ApiError::bad_request(
        "group_visibility 只能是 \"all\" 或分组 id 数组",
    ))
}

fn check_visibility(v: &str) -> Result<(), ApiError> {
    if v == "private" || v == "public" {
        Ok(())
    } else {
        Err(ApiError::bad_request("visibility 只能是 private 或 public"))
    }
}

pub async fn skill_update(
    State(state): S,
    headers: HeaderMap,
    Path((owner, name)): Path<(String, String)>,
    Json(req): Json<ResourcePatch>,
) -> Result<StatusCode, ApiError> {
    require_admin(&state, &headers)?;
    let now = now_string();
    let conn = state.db.lock().unwrap();
    let oid: Option<i64> = conn
        .query_row(
            "SELECT id FROM users WHERE username = ?1",
            [&owner],
            |r| r.get(0),
        )
        .optional()
        .map_err(db_err)?;
    let oid = oid.ok_or_else(|| ApiError::not_found("未找到该技能"))?;
    if let Some(vis) = req.visibility.as_deref() {
        check_visibility(vis)?;
        conn.execute(
            "UPDATE skills SET visibility = ?1, updated_at = ?2 WHERE owner_id = ?3 AND name = ?4",
            rusqlite::params![vis, now, oid, name],
        )
        .map_err(db_err)?;
    }
    if let Some(gv) = &req.group_visibility {
        let stored = normalize_group_vis(gv)?;
        conn.execute(
            "UPDATE skills SET group_visibility = ?1, updated_at = ?2 WHERE owner_id = ?3 AND name = ?4",
            rusqlite::params![stored, now, oid, name],
        )
        .map_err(db_err)?;
    }
    if let Some(label_ids) = &req.label_ids {
        let sid: i64 = conn
            .query_row(
                "SELECT id FROM skills WHERE owner_id = ?1 AND name = ?2",
                rusqlite::params![oid, name],
                |r| r.get(0),
            )
            .optional()
            .map_err(db_err)?
            .ok_or_else(|| ApiError::not_found("未找到该技能"))?;
        replace_resource_labels(&conn, "skill_labels", "skill_id", sid, label_ids)?;
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn skill_delete(
    State(state): S,
    headers: HeaderMap,
    Path((owner, name)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    require_admin(&state, &headers)?;
    if skills::delete_skill_record(&state, &owner, &name)? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::not_found("未找到该技能"))
    }
}

pub async fn mcp_update(
    State(state): S,
    headers: HeaderMap,
    Path((owner, name)): Path<(String, String)>,
    Json(req): Json<ResourcePatch>,
) -> Result<StatusCode, ApiError> {
    require_admin(&state, &headers)?;
    let now = now_string();
    let conn = state.db.lock().unwrap();
    let oid: Option<i64> = conn
        .query_row(
            "SELECT id FROM users WHERE username = ?1",
            [&owner],
            |r| r.get(0),
        )
        .optional()
        .map_err(db_err)?;
    let oid = oid.ok_or_else(|| ApiError::not_found("未找到该 MCP"))?;
    if let Some(vis) = req.visibility.as_deref() {
        check_visibility(vis)?;
        conn.execute(
            "UPDATE mcps SET visibility = ?1, updated_at = ?2 WHERE owner_id = ?3 AND name = ?4",
            rusqlite::params![vis, now, oid, name],
        )
        .map_err(db_err)?;
    }
    if let Some(gv) = &req.group_visibility {
        let stored = normalize_group_vis(gv)?;
        conn.execute(
            "UPDATE mcps SET group_visibility = ?1, updated_at = ?2 WHERE owner_id = ?3 AND name = ?4",
            rusqlite::params![stored, now, oid, name],
        )
        .map_err(db_err)?;
    }
    if let Some(label_ids) = &req.label_ids {
        let mid: i64 = conn
            .query_row(
                "SELECT id FROM mcps WHERE owner_id = ?1 AND name = ?2",
                rusqlite::params![oid, name],
                |r| r.get(0),
            )
            .optional()
            .map_err(db_err)?
            .ok_or_else(|| ApiError::not_found("未找到该 MCP"))?;
        replace_resource_labels(&conn, "mcp_labels", "mcp_id", mid, label_ids)?;
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn mcp_delete(
    State(state): S,
    headers: HeaderMap,
    Path((owner, name)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    require_admin(&state, &headers)?;
    let conn = state.db.lock().unwrap();
    let n = conn
        .execute(
            "DELETE FROM mcps WHERE name = ?2 AND owner_id =
                (SELECT id FROM users WHERE username = ?1)",
            rusqlite::params![owner, name],
        )
        .map_err(db_err)?;
    if n == 0 {
        return Err(ApiError::not_found("未找到该 MCP"));
    }
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// 全量资源包：导入 / 导出
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
struct Pack {
    version: u32,
    exported_at: String,
    #[serde(default)]
    groups: Vec<PackGroup>,
    #[serde(default)]
    labels: Vec<PackLabel>,
    users: Vec<PackUser>,
    mcps: Vec<PackMcp>,
    skills: Vec<PackSkill>,
    secrets: Vec<PackSecret>,
    #[serde(default)]
    calls: Vec<PackCall>,
}

#[derive(Serialize, Deserialize)]
struct PackGroup {
    name: String,
    #[serde(default)]
    description: String,
    created_at: String,
}

#[derive(Serialize, Deserialize)]
struct PackLabel {
    name: String,
    #[serde(default)]
    created_at: String,
}

#[derive(Serialize, Deserialize)]
struct PackUser {
    username: String,
    password_hash: String,
    /// 所属分组名列表（多对多）。
    #[serde(default)]
    groups: Vec<String>,
    /// 兼容旧资源包的单分组字段（导入时并入 groups）。
    #[serde(default)]
    group: String,
    created_at: String,
}

#[derive(Serialize, Deserialize)]
struct PackMcp {
    owner: String,
    name: String,
    visibility: String,
    #[serde(default)]
    group_visibility: String,
    /// 已分配的受管标签名。
    #[serde(default)]
    labels: Vec<String>,
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
    #[serde(default)]
    group_visibility: String,
    /// 已分配的受管标签名。
    #[serde(default)]
    labels: Vec<String>,
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
    groups: usize,
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

    let groups = {
        let mut stmt = conn
            .prepare("SELECT name, description, created_at FROM groups ORDER BY id")
            .map_err(db_err)?;
        let rows = stmt
            .query_map([], |r| {
                Ok(PackGroup {
                    name: r.get(0)?,
                    description: r.get(1)?,
                    created_at: r.get(2)?,
                })
            })
            .map_err(db_err)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(db_err)?
    };

    let labels = {
        let mut stmt = conn
            .prepare("SELECT name, created_at FROM labels ORDER BY id")
            .map_err(db_err)?;
        let rows = stmt
            .query_map([], |r| {
                Ok(PackLabel {
                    name: r.get(0)?,
                    created_at: r.get(1)?,
                })
            })
            .map_err(db_err)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(db_err)?
    };

    // 资源 → 标签名映射，供 mcps/skills 装配。
    let mcp_label_map = super::routes::all_resource_labels(&conn, "mcp_labels", "mcp_id");
    let skill_label_map = super::routes::all_resource_labels(&conn, "skill_labels", "skill_id");

    let users = {
        // 先聚合每个用户名的分组列表（按用户名映射，便于装配 PackUser）。
        let mut by_user: HashMap<String, Vec<String>> = HashMap::new();
        {
            let mut stmt = conn
                .prepare(
                    "SELECT u.username, g.name FROM user_groups ug
                     JOIN users u ON u.id = ug.user_id
                     JOIN groups g ON g.id = ug.group_id ORDER BY g.name",
                )
                .map_err(db_err)?;
            let rows = stmt
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                .map_err(db_err)?;
            for row in rows {
                let (uname, gname) = row.map_err(db_err)?;
                by_user.entry(uname).or_default().push(gname);
            }
        }
        let mut stmt = conn
            .prepare("SELECT username, password_hash, created_at FROM users ORDER BY id")
            .map_err(db_err)?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .map_err(db_err)?;
        let mut out = Vec::new();
        for row in rows {
            let (username, password_hash, created_at) = row.map_err(db_err)?;
            out.push(PackUser {
                groups: by_user.remove(&username).unwrap_or_default(),
                group: String::new(),
                username,
                password_hash,
                created_at,
            });
        }
        out
    };

    let mcps = {
        let mut stmt = conn
            .prepare(
                "SELECT u.username, m.name, m.visibility, m.version, m.manifest, m.tools, m.updated_at,
                        m.group_visibility, m.id
                 FROM mcps m JOIN users u ON u.id = m.owner_id ORDER BY m.id",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map([], |r| {
                let manifest: String = r.get(4)?;
                let tools: String = r.get(5)?;
                let id: i64 = r.get(8)?;
                Ok(PackMcp {
                    owner: r.get(0)?,
                    name: r.get(1)?,
                    visibility: r.get(2)?,
                    version: r.get(3)?,
                    manifest: serde_json::from_str(&manifest).unwrap_or_default(),
                    tools: serde_json::from_str(&tools).unwrap_or(serde_json::json!([])),
                    updated_at: r.get(6)?,
                    group_visibility: r.get(7)?,
                    labels: mcp_label_map.get(&id).cloned().unwrap_or_default(),
                })
            })
            .map_err(db_err)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(db_err)?
    };

    let skills = {
        let mut stmt = conn
            .prepare(
                "SELECT u.username, s.name, s.category, s.visibility, s.version, s.description,
                        s.tags, s.skill_md, s.metadata, s.archive_sha256, s.archive_size, s.updated_at,
                        s.group_visibility, s.id
                 FROM skills s JOIN users u ON u.id = s.owner_id ORDER BY s.id",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map([], |r| {
                let tags: String = r.get(6)?;
                let metadata: String = r.get(8)?;
                let id: i64 = r.get(13)?;
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
                    group_visibility: r.get(12)?,
                    labels: skill_label_map.get(&id).cloned().unwrap_or_default(),
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
        groups,
        users,
        mcps,
        skills,
        secrets,
        calls,
        labels,
    })
}

/// 把资源包写入数据库（一个事务内 upsert）。
fn apply_pack(state: &AppState, pack: &Pack) -> Result<ImportSummary, ApiError> {
    let mut guard = state.db.lock().unwrap();
    let tx = guard.transaction().map_err(db_err)?;
    let mut skipped = Vec::new();

    // 分组先行 upsert（按名字自然键），并构建 名字 → 本地 id 映射（含目标既有分组）。
    for g in &pack.groups {
        tx.execute(
            "INSERT INTO groups(name, description, created_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(name) DO UPDATE SET description = excluded.description",
            rusqlite::params![g.name, g.description, g.created_at],
        )
        .map_err(db_err)?;
    }
    let mut gids: HashMap<String, i64> = HashMap::new();
    {
        let mut stmt = tx.prepare("SELECT name, id FROM groups").map_err(db_err)?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
            .map_err(db_err)?;
        for row in rows {
            let (name, id) = row.map_err(db_err)?;
            gids.insert(name, id);
        }
    }

    // 标签 upsert（按名字自然键），并构建 名字 → 本地 id 映射。
    for l in &pack.labels {
        tx.execute(
            "INSERT INTO labels(name, created_at) VALUES (?1, ?2)
             ON CONFLICT(name) DO NOTHING",
            rusqlite::params![l.name, l.created_at],
        )
        .map_err(db_err)?;
    }
    let mut lids: HashMap<String, i64> = HashMap::new();
    {
        let mut stmt = tx.prepare("SELECT name, id FROM labels").map_err(db_err)?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
            .map_err(db_err)?;
        for row in rows {
            let (name, id) = row.map_err(db_err)?;
            lids.insert(name, id);
        }
    }

    for u in &pack.users {
        tx.execute(
            "INSERT INTO users(username, password_hash, created_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(username) DO UPDATE SET password_hash = excluded.password_hash",
            rusqlite::params![u.username, u.password_hash, u.created_at],
        )
        .map_err(db_err)?;
        let uid: i64 = tx
            .query_row(
                "SELECT id FROM users WHERE username = ?1",
                [&u.username],
                |r| r.get(0),
            )
            .map_err(db_err)?;
        // 分组归属按名字解析并合并（不删除目标已有关联，符合 upsert 语义）。
        // 兼容旧资源包：单分组 group 字段并入。
        let mut names: Vec<&String> = u.groups.iter().collect();
        if !u.group.is_empty() && !names.iter().any(|n| *n == &u.group) {
            names.push(&u.group);
        }
        for name in names {
            if let Some(&gid) = gids.get(name) {
                tx.execute(
                    "INSERT OR IGNORE INTO user_groups(user_id, group_id) VALUES (?1, ?2)",
                    rusqlite::params![uid, gid],
                )
                .map_err(db_err)?;
            }
        }
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
        let gv = if m.group_visibility.is_empty() {
            "all"
        } else {
            m.group_visibility.as_str()
        };
        tx.execute(
            "INSERT INTO mcps(owner_id, name, visibility, group_visibility, version, manifest, tools, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(owner_id, name) DO UPDATE SET visibility=excluded.visibility,
                 group_visibility=excluded.group_visibility,
                 version=excluded.version, manifest=excluded.manifest, tools=excluded.tools,
                 updated_at=excluded.updated_at",
            rusqlite::params![oid, m.name, m.visibility, gv, m.version, manifest, tools, m.updated_at],
        )
        .map_err(db_err)?;
        // 标签关联（合并，不删除目标已有）。按名字解析为本地 label id。
        if !m.labels.is_empty() {
            let mid: i64 = tx
                .query_row(
                    "SELECT id FROM mcps WHERE owner_id = ?1 AND name = ?2",
                    rusqlite::params![oid, m.name],
                    |r| r.get(0),
                )
                .map_err(db_err)?;
            for lname in &m.labels {
                if let Some(&lid) = lids.get(lname) {
                    tx.execute(
                        "INSERT OR IGNORE INTO mcp_labels(mcp_id, label_id) VALUES (?1, ?2)",
                        rusqlite::params![mid, lid],
                    )
                    .map_err(db_err)?;
                }
            }
        }
        mcps += 1;
    }

    let mut skills = 0usize;
    for s in &pack.skills {
        let Some(&oid) = ids.get(&s.owner) else {
            skipped.push(format!("skill {}/{} (owner 缺失)", s.owner, s.name));
            continue;
        };
        tx.execute(
            "INSERT INTO skills(owner_id, name, category, visibility, group_visibility, version, description,
                                tags, skill_md, metadata, archive_sha256, archive_size, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
             ON CONFLICT(owner_id, name) DO UPDATE SET category=excluded.category,
                 visibility=excluded.visibility, group_visibility=excluded.group_visibility,
                 version=excluded.version,
                 description=excluded.description, tags=excluded.tags, skill_md=excluded.skill_md,
                 metadata=excluded.metadata, archive_sha256=excluded.archive_sha256,
                 archive_size=excluded.archive_size, updated_at=excluded.updated_at",
            rusqlite::params![
                oid,
                s.name,
                s.category,
                s.visibility,
                if s.group_visibility.is_empty() { "all" } else { s.group_visibility.as_str() },
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
        // 标签关联（合并，不删除目标已有）。
        if !s.labels.is_empty() {
            let sid: i64 = tx
                .query_row(
                    "SELECT id FROM skills WHERE owner_id = ?1 AND name = ?2",
                    rusqlite::params![oid, s.name],
                    |r| r.get(0),
                )
                .map_err(db_err)?;
            for lname in &s.labels {
                if let Some(&lid) = lids.get(lname) {
                    tx.execute(
                        "INSERT OR IGNORE INTO skill_labels(skill_id, label_id) VALUES (?1, ?2)",
                        rusqlite::params![sid, lid],
                    )
                    .map_err(db_err)?;
                }
            }
        }
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
        groups: pack.groups.len(),
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
