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

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde::{Deserialize, Serialize};

use rand::TryRng;

use crate::archive::ZSTD_LEVEL;
use crate::shared::{McpManifest, Protocol, Runtime, SkillVersionInfo, SKILL_CATEGORIES, ToolMeta};

use super::auth;
use super::crypto;
use super::db::{db_params, Db, Value};
use super::error::ApiError;
use super::ldap;
use super::routes::{now_string, now_unix};
use super::settings;
use super::skills;
use super::AppState;

type S = State<Arc<AppState>>;

/// 资源包格式版本。
const PACK_VERSION: u32 = 2;

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

async fn count(db: &Db, sql: &str) -> Result<i64, ApiError> {
    Ok(db.query_row(sql, db_params![]).await?.get(0)?)
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

    let users = count(&state.db, "SELECT COUNT(*) FROM users").await?;
    let skills = count(&state.db, "SELECT COUNT(*) FROM skills").await?;
    let skills_public =
        count(&state.db, "SELECT COUNT(*) FROM skills WHERE visibility='public'").await?;
    let mcps = count(&state.db, "SELECT COUNT(*) FROM mcps").await?;
    let mcps_public =
        count(&state.db, "SELECT COUNT(*) FROM mcps WHERE visibility='public'").await?;
    let secrets = count(&state.db, "SELECT COUNT(*) FROM secrets").await?;
    let calls_total = count(&state.db, "SELECT COUNT(*) FROM tool_calls").await?;
    let calls_24h: i64 = state
        .db
        .query_row(
            "SELECT COUNT(*) FROM tool_calls WHERE created_ts >= ?1",
            db_params![since],
        )
        .await?
        .get(0)?;
    let calls_errors_24h: i64 = state
        .db
        .query_row(
            "SELECT COUNT(*) FROM tool_calls WHERE created_ts >= ?1 AND ok = 0",
            db_params![since],
        )
        .await?
        .get(0)?;

    let top_tools = state
        .db
        .query_map(
            "SELECT CONCAT(mcp_name, '/', tool) AS t, COUNT(*) AS c
                 FROM tool_calls WHERE created_ts >= ?1
                 GROUP BY t ORDER BY c DESC, t LIMIT 8",
            db_params![since],
            |r| {
                Ok(TopTool {
                    tool: r.get(0)?,
                    count: r.get(1)?,
                })
            },
        )
        .await?;

    let recent_errors = state
        .db
        .query_map(
            "SELECT CONCAT(mcp_name, '/', tool), caller, error, created_at
                 FROM tool_calls WHERE ok = 0 ORDER BY id DESC LIMIT 8",
            db_params![],
            |r| {
                Ok(RecentError {
                    tool: r.get(0)?,
                    caller: r.get(1)?,
                    error: r.get(2)?,
                    at: r.get(3)?,
                })
            },
        )
        .await?;

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
    /// 认证来源：'local' 本地口令 / 'ldap' 目录影子账号。
    auth_source: String,
    created_at: String,
    skills: i64,
    mcps: i64,
    secrets: i64,
}

pub async fn users(State(state): S, headers: HeaderMap) -> Result<Json<Vec<AdminUser>>, ApiError> {
    require_admin(&state, &headers)?;

    // 先聚合每个用户的分组（多对多），再装配用户行。
    let mut by_user: HashMap<i64, Vec<GroupBrief>> = HashMap::new();
    let rows = state
        .db
        .query_map(
            "SELECT ug.user_id, g.id, g.name FROM user_groups ug
                 JOIN groups g ON g.id = ug.group_id ORDER BY g.name",
            db_params![],
            |r| Ok((r.get::<i64>(0)?, r.get::<i64>(1)?, r.get::<String>(2)?)),
        )
        .await?;
    for (uid, gid, name) in rows {
        by_user.entry(uid).or_default().push(GroupBrief { id: gid, name });
    }

    let rows = state
        .db
        .query_map(
            "SELECT u.id, u.username, u.created_at, u.auth_source,
                    (SELECT COUNT(*) FROM skills s WHERE s.owner_id = u.id),
                    (SELECT COUNT(*) FROM mcps m WHERE m.owner_id = u.id),
                    (SELECT COUNT(*) FROM secrets x WHERE x.owner_id = u.id)
             FROM users u ORDER BY u.id",
            db_params![],
            |r| {
                Ok((
                    r.get::<i64>(0)?,
                    r.get::<String>(1)?,
                    r.get::<String>(2)?,
                    r.get::<String>(3)?,
                    r.get::<i64>(4)?,
                    r.get::<i64>(5)?,
                    r.get::<i64>(6)?,
                ))
            },
        )
        .await?;
    let mut out = Vec::new();
    for (id, username, created_at, auth_source, skills, mcps, secrets) in rows {
        out.push(AdminUser {
            groups: by_user.remove(&id).unwrap_or_default(),
            id,
            username,
            auth_source,
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
    tags: Vec<String>,
    skill_md: String,
    mcp_dependencies: Vec<String>,
    preferred_tools: Vec<String>,
    archive_size: i64,
    has_archive: bool,
    labels: Vec<LabelBrief>,
    likes: i64,
    favorites: i64,
    downloads: i64,
    updated_at: String,
}

/// 技能 `metadata` 列里 JSON 承载的扩展字段（与 [`super::skills`] 中的同名结构对齐）。
#[derive(Serialize, Deserialize, Default)]
struct SkillMeta {
    #[serde(default)]
    mcp_dependencies: Vec<String>,
    #[serde(default)]
    preferred_tools: Vec<String>,
}

/// 标签归一化：去空白、转小写、去重（与 [`super::skills`] 的 `lower_tags` 对齐）。
fn lower_tags(tags: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in tags {
        let t = t.trim().to_lowercase();
        if !t.is_empty() && !out.contains(&t) {
            out.push(t);
        }
    }
    out
}

#[derive(Serialize, Clone)]
pub struct LabelBrief {
    id: i64,
    name: String,
}

/// 全量映射：资源 id → 已分配标签（id+name）。供管理列表/详情一次性装配。
async fn all_label_briefs(db: &Db, junction: &str, fk: &str) -> HashMap<i64, Vec<LabelBrief>> {
    let mut map: HashMap<i64, Vec<LabelBrief>> = HashMap::new();
    let sql = format!(
        "SELECT j.{fk}, l.id, l.name FROM {junction} j JOIN labels l ON l.id = j.label_id
         ORDER BY l.name"
    );
    if let Ok(rows) = db
        .query_map(&sql, db_params![], |r| {
            Ok((r.get::<i64>(0)?, r.get::<i64>(1)?, r.get::<String>(2)?))
        })
        .await
    {
        for (rid, lid, name) in rows {
            map.entry(rid).or_default().push(LabelBrief { id: lid, name });
        }
    }
    map
}

pub async fn skills_all(
    State(state): S,
    headers: HeaderMap,
) -> Result<Json<Vec<AdminSkill>>, ApiError> {
    require_admin(&state, &headers)?;
    let mut label_map = all_label_briefs(&state.db, "skill_labels", "skill_id").await;
    let count_map =
        super::routes::all_reaction_counts(&state.db, "skill_reactions", "skill_id").await;
    let rows = state
        .db
        .query_map(
            "SELECT u.username, s.name, s.category, s.visibility, s.version, s.description,
                    s.archive_size, s.archive_sha256, s.updated_at, s.group_visibility, s.id,
                    s.tags, s.skill_md, s.metadata, s.downloads
             FROM skills s JOIN users u ON u.id = s.owner_id
             ORDER BY s.updated_at DESC, s.name",
            db_params![],
            |r| {
                let sha: String = r.get(7)?;
                let tags_json: String = r.get(11)?;
                let meta_json: String = r.get(13)?;
                let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();
                let meta: SkillMeta = serde_json::from_str(&meta_json).unwrap_or_default();
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
                        tags,
                        skill_md: r.get(12)?,
                        mcp_dependencies: meta.mcp_dependencies,
                        preferred_tools: meta.preferred_tools,
                        labels: Vec::new(),
                        likes: 0,
                        favorites: 0,
                        downloads: r.get(14)?,
                    },
                    r.get::<i64>(10)?,
                ))
            },
        )
        .await?;
    let mut out = Vec::new();
    for (mut s, id) in rows {
        s.labels = label_map.remove(&id).unwrap_or_default();
        (s.likes, s.favorites) = count_map.get(&id).copied().unwrap_or_default();
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
    manifest: McpManifest,
    labels: Vec<LabelBrief>,
    likes: i64,
    favorites: i64,
    updated_at: String,
}

pub async fn mcps_all(State(state): S, headers: HeaderMap) -> Result<Json<Vec<AdminMcp>>, ApiError> {
    require_admin(&state, &headers)?;
    let mut label_map = all_label_briefs(&state.db, "mcp_labels", "mcp_id").await;
    let count_map = super::routes::all_reaction_counts(&state.db, "mcp_reactions", "mcp_id").await;
    let rows = state
        .db
        .query_map(
            "SELECT u.username, m.name, m.visibility, m.version, m.manifest, m.updated_at,
                    m.group_visibility, m.id
             FROM mcps m JOIN users u ON u.id = m.owner_id
             ORDER BY m.updated_at DESC, m.name",
            db_params![],
            |r| {
                Ok((
                    r.get::<String>(0)?,
                    r.get::<String>(1)?,
                    r.get::<String>(2)?,
                    r.get::<String>(3)?,
                    r.get::<String>(4)?,
                    r.get::<String>(5)?,
                    r.get::<String>(6)?,
                    r.get::<i64>(7)?,
                ))
            },
        )
        .await?;
    let mut out = Vec::new();
    for (owner, name, visibility, version, manifest_json, updated_at, group_visibility, id) in rows
    {
        // 解析完整 manifest；损坏时回退到一个由已知列拼出的最小可编辑 manifest，
        // 避免单条坏数据拖垮整表，也保证前端编辑表单总有结构可填。
        let manifest: McpManifest = serde_json::from_str(&manifest_json).unwrap_or_else(|_| {
            McpManifest {
                resource_type: "mcp".into(),
                name: name.clone(),
                description: String::new(),
                version: version.clone(),
                runtime: Runtime::Remote,
                protocol: Protocol::Streamable,
                url: None,
                command: None,
                env: BTreeMap::new(),
                headers: BTreeMap::new(),
            }
        });
        out.push(AdminMcp {
            owner,
            name,
            visibility,
            group_visibility,
            version,
            runtime: manifest.runtime.as_str().to_string(),
            protocol: manifest.protocol.as_str().to_string(),
            manifest,
            labels: label_map.remove(&id).unwrap_or_default(),
            likes: count_map.get(&id).copied().unwrap_or_default().0,
            favorites: count_map.get(&id).copied().unwrap_or_default().1,
            updated_at,
        });
    }
    Ok(Json(out))
}

#[derive(Serialize)]
pub struct CallLog {
    caller: String,
    /// 发起者用户 id（按用户名快照关联，用户已删除则为 null）。
    caller_id: Option<i64>,
    owner: String,
    mcp_name: String,
    tool: String,
    ok: bool,
    error: String,
    /// 结果摘要（成功调用的结果概要；失败时为空，错误见 error）。
    result: String,
    ms: i64,
    created_at: String,
}

/// 调用日志查询参数：服务 / 工具 / 发起者 / 时间窗口（小时）/ 仅错误 + 分页。
#[derive(Deserialize)]
pub struct CallsQuery {
    #[serde(default)]
    service: Option<String>,
    #[serde(default)]
    tool: Option<String>,
    #[serde(default)]
    caller: Option<String>,
    /// 时间窗口（小时）；缺省或 0 表示不限。
    #[serde(default)]
    window: Option<i64>,
    #[serde(default)]
    errors_only: Option<bool>,
    #[serde(default)]
    limit: Option<i64>,
    #[serde(default)]
    offset: Option<i64>,
}

#[derive(Serialize)]
pub struct CallsResp {
    /// 命中过滤条件的总条数（用于分页）。
    total: i64,
    rows: Vec<CallLog>,
    /// 下拉候选：去重后的全部服务名与工具名（不随当前过滤变化）。
    services: Vec<String>,
    tools: Vec<String>,
}

pub async fn calls(
    State(state): S,
    headers: HeaderMap,
    Query(q): Query<CallsQuery>,
) -> Result<Json<CallsResp>, ApiError> {
    require_admin(&state, &headers)?;

    // 动态拼装过滤条件，参数按出现顺序绑定（占位符仅来自计数，无注入风险）。
    let mut where_sql = String::from(" WHERE 1=1");
    let mut params: Vec<Value> = Vec::new();
    if let Some(s) = q.service.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        params.push(Value::Text(s.to_string()));
        where_sql.push_str(&format!(" AND mcp_name = ?{}", params.len()));
    }
    if let Some(t) = q.tool.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        params.push(Value::Text(t.to_string()));
        where_sql.push_str(&format!(" AND tool = ?{}", params.len()));
    }
    if let Some(c) = q.caller.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        params.push(Value::Text(format!("%{}%", c.to_lowercase())));
        where_sql.push_str(&format!(" AND lower(caller) LIKE ?{}", params.len()));
    }
    if let Some(w) = q.window.filter(|w| *w > 0) {
        params.push(Value::Int(now_unix() - w * 3600));
        where_sql.push_str(&format!(" AND created_ts >= ?{}", params.len()));
    }
    if q.errors_only.unwrap_or(false) {
        where_sql.push_str(" AND ok = 0");
    }

    let total: i64 = state
        .db
        .query_row(
            &format!("SELECT COUNT(*) FROM tool_calls{where_sql}"),
            params.clone(),
        )
        .await?
        .get(0)?;

    let limit = q.limit.unwrap_or(20).clamp(1, 200);
    let offset = q.offset.unwrap_or(0).max(0);

    let rows = {
        let sql = format!(
            "SELECT tc.caller, u.id, tc.owner, tc.mcp_name, tc.tool, tc.ok, tc.error, tc.result, tc.ms, tc.created_at
             FROM tool_calls tc LEFT JOIN users u ON u.username = tc.caller
             {where_sql} ORDER BY tc.id DESC LIMIT ?{} OFFSET ?{}",
            params.len() + 1,
            params.len() + 2,
        );
        let mut p = params;
        p.push(Value::Int(limit));
        p.push(Value::Int(offset));
        state
            .db
            .query_map(&sql, p, |r| {
                Ok(CallLog {
                    caller: r.get(0)?,
                    caller_id: r.get(1)?,
                    owner: r.get(2)?,
                    mcp_name: r.get(3)?,
                    tool: r.get(4)?,
                    ok: r.get::<i64>(5)? != 0,
                    error: r.get(6)?,
                    result: r.get(7)?,
                    ms: r.get(8)?,
                    created_at: r.get(9)?,
                })
            })
            .await?
    };

    let services = distinct_calls_column(&state.db, "mcp_name").await?;
    let tools = distinct_calls_column(&state.db, "tool").await?;

    Ok(Json(CallsResp { total, rows, services, tools }))
}

/// 取 tool_calls 某列去重非空值（升序），供过滤下拉。`col` 仅来自固定字面量。
async fn distinct_calls_column(db: &Db, col: &str) -> Result<Vec<String>, ApiError> {
    let sql = format!("SELECT DISTINCT {col} FROM tool_calls WHERE {col} <> '' ORDER BY {col}");
    Ok(db
        .query_map(&sql, db_params![], |r| r.get::<String>(0))
        .await?)
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
    let out = state
        .db
        .query_map(
            "SELECT g.id, g.name, g.description, g.created_at,
                    (SELECT COUNT(*) FROM user_groups ug WHERE ug.group_id = g.id)
             FROM groups g ORDER BY g.name",
            db_params![],
            |r| {
                Ok(AdminGroup {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    description: r.get(2)?,
                    created_at: r.get(3)?,
                    users: r.get(4)?,
                })
            },
        )
        .await?;
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
    let exists = state
        .db
        .query_opt("SELECT 1 FROM groups WHERE name = ?1", db_params![name])
        .await?
        .is_some();
    if exists {
        return Err(ApiError::conflict("分组名已存在"));
    }
    let id = state
        .db
        .insert_id(
            "INSERT INTO groups(name, description, created_at) VALUES (?1, ?2, ?3)",
            db_params![name, req.description.trim(), now],
        )
        .await?;
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
    if let Some(name) = req.name.as_deref().map(str::trim) {
        if !valid_group_name(name) {
            return Err(ApiError::bad_request("分组名不能为空且长度 ≤64"));
        }
        let clash = state
            .db
            .query_opt(
                "SELECT 1 FROM groups WHERE name = ?1 AND id != ?2",
                db_params![name, id],
            )
            .await?
            .is_some();
        if clash {
            return Err(ApiError::conflict("分组名已存在"));
        }
        state
            .db
            .execute(
                "UPDATE groups SET name = ?1 WHERE id = ?2",
                db_params![name, id],
            )
            .await?;
    }
    if let Some(desc) = req.description.as_deref() {
        state
            .db
            .execute(
                "UPDATE groups SET description = ?1 WHERE id = ?2",
                db_params![desc.trim(), id],
            )
            .await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn group_delete(
    State(state): S,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    require_admin(&state, &headers)?;
    // 解除该分组的全部成员关联（FK 亦会级联，这里显式清理以防 FK 未启用）。
    state
        .db
        .execute("DELETE FROM user_groups WHERE group_id = ?1", db_params![id])
        .await?;
    let n = state
        .db
        .execute("DELETE FROM groups WHERE id = ?1", db_params![id])
        .await?;
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
    let out = state
        .db
        .query_map(
            "SELECT l.id, l.name, l.created_at,
                    (SELECT COUNT(*) FROM skill_labels sl WHERE sl.label_id = l.id),
                    (SELECT COUNT(*) FROM mcp_labels ml WHERE ml.label_id = l.id)
             FROM labels l ORDER BY l.name",
            db_params![],
            |r| {
                Ok(AdminLabel {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    created_at: r.get(2)?,
                    skills: r.get(3)?,
                    mcps: r.get(4)?,
                })
            },
        )
        .await?;
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
    let exists = state
        .db
        .query_opt("SELECT 1 FROM labels WHERE name = ?1", db_params![name])
        .await?
        .is_some();
    if exists {
        return Err(ApiError::conflict("标签名已存在"));
    }
    let id = state
        .db
        .insert_id(
            "INSERT INTO labels(name, created_at) VALUES (?1, ?2)",
            db_params![name, now],
        )
        .await?;
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
    let clash = state
        .db
        .query_opt(
            "SELECT 1 FROM labels WHERE name = ?1 AND id != ?2",
            db_params![name, id],
        )
        .await?
        .is_some();
    if clash {
        return Err(ApiError::conflict("标签名已存在"));
    }
    let n = state
        .db
        .execute(
            "UPDATE labels SET name = ?1 WHERE id = ?2",
            db_params![name, id],
        )
        .await?;
    // MySQL 仅统计实际变更行：重命名为同名时返回 0，须复核存在性再判 404。
    if n == 0 {
        let exists: bool = state
            .db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM labels WHERE id = ?1)",
                db_params![id],
            )
            .await?
            .get(0)?;
        if !exists {
            return Err(ApiError::not_found("未找到该标签"));
        }
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn label_delete(
    State(state): S,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    require_admin(&state, &headers)?;
    // 解除该标签在各资源上的关联（FK 亦会级联，这里显式清理以防 FK 未启用）。
    state
        .db
        .execute("DELETE FROM skill_labels WHERE label_id = ?1", db_params![id])
        .await?;
    state
        .db
        .execute("DELETE FROM mcp_labels WHERE label_id = ?1", db_params![id])
        .await?;
    let n = state
        .db
        .execute("DELETE FROM labels WHERE id = ?1", db_params![id])
        .await?;
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
    for gid in &req.group_ids {
        ensure_group_exists(&state.db, *gid).await?;
    }
    let exists = state
        .db
        .query_opt("SELECT 1 FROM users WHERE username = ?1", db_params![username])
        .await?
        .is_some();
    if exists {
        return Err(ApiError::conflict("用户名已存在"));
    }
    let uid = state
        .db
        .insert_id(
            "INSERT INTO users(username, password_hash, created_at) VALUES (?1, ?2, ?3)",
            db_params![username, hash, now],
        )
        .await?;
    replace_user_groups(&state.db, uid, &req.group_ids).await?;
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
    let exists = state
        .db
        .query_opt("SELECT 1 FROM users WHERE id = ?1", db_params![id])
        .await?
        .is_some();
    if !exists {
        return Err(ApiError::not_found("未找到该用户"));
    }
    if let Some(gids) = &req.group_ids {
        for gid in gids {
            ensure_group_exists(&state.db, *gid).await?;
        }
        replace_user_groups(&state.db, id, gids).await?;
    }
    if let Some(pw) = req.password.as_deref().filter(|p| !p.is_empty()) {
        if pw.len() < 6 {
            return Err(ApiError::bad_request("密码至少 6 位"));
        }
        let hash = auth::hash_password(pw).map_err(|e| ApiError::internal(e.to_string()))?;
        state
            .db
            .execute(
                "UPDATE users SET password_hash = ?1 WHERE id = ?2",
                db_params![hash, id],
            )
            .await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// 整体覆盖某用户的分组归属：先清空再按给定 id 集重建（去重）。
async fn replace_user_groups(db: &Db, uid: i64, gids: &[i64]) -> Result<(), ApiError> {
    db.execute("DELETE FROM user_groups WHERE user_id = ?1", db_params![uid])
        .await?;
    for gid in gids {
        db.execute(
            "INSERT INTO user_groups(user_id, group_id) VALUES (?1, ?2) ON CONFLICT DO NOTHING",
            db_params![uid, gid],
        )
        .await?;
    }
    Ok(())
}

pub async fn user_delete(
    State(state): S,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    require_admin(&state, &headers)?;
    // 技能压缩体 blob 的清理：先收集该用户引用且无人共享的 sha，删除后 GC。
    let orphan_shas = collect_user_skill_blobs(&state.db, id).await?;
    let n = state
        .db
        .execute("DELETE FROM users WHERE id = ?1", db_params![id])
        .await?;
    if n == 0 {
        return Err(ApiError::not_found("未找到该用户"));
    }
    // 删除用户级联清掉其 skills（外键 ON DELETE CASCADE），此后无人引用的 blob 落盘清理。
    for sha in orphan_shas {
        let still = state
            .db
            .query_opt(
                "SELECT 1 FROM skills WHERE archive_sha256 = ?1 LIMIT 1",
                db_params![sha],
            )
            .await?
            .is_some();
        if !still {
            if let Some(p) = skills::find_blob(&state, &sha) {
                let _ = std::fs::remove_file(p);
            }
        }
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn ensure_group_exists(db: &Db, gid: i64) -> Result<(), ApiError> {
    let ok = db
        .query_opt("SELECT 1 FROM groups WHERE id = ?1", db_params![gid])
        .await?
        .is_some();
    if ok {
        Ok(())
    } else {
        Err(ApiError::bad_request("指定的分组不存在"))
    }
}

/// 收集某用户名下技能引用的全部非空 sha256（去重），供删除后 GC 判定。
async fn collect_user_skill_blobs(db: &Db, uid: i64) -> Result<Vec<String>, ApiError> {
    Ok(db
        .query_map(
            "SELECT DISTINCT archive_sha256 FROM skills WHERE owner_id = ?1 AND archive_sha256 != ''",
            db_params![uid],
            |r| r.get::<String>(0),
        )
        .await?)
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
    // --- 内容编辑（管理员）。每个字段缺省则不改动对应内容 ---
    /// 版本号（技能 / MCP 通用）。
    #[serde(default)]
    version: Option<String>,
    /// 技能：逻辑分类 skill / kb / toolchain。
    #[serde(default)]
    category: Option<String>,
    /// 技能：一句话描述。
    #[serde(default)]
    description: Option<String>,
    /// 技能：自由标签。
    #[serde(default)]
    tags: Option<Vec<String>>,
    /// 技能：SKILL.md 正文。
    #[serde(default)]
    skill_md: Option<String>,
    /// 技能：依赖的底层 MCP。
    #[serde(default)]
    mcp_dependencies: Option<Vec<String>>,
    /// 技能：倾向优先使用的工具。
    #[serde(default)]
    preferred_tools: Option<Vec<String>>,
    /// MCP：完整运行清单（覆盖式更新；name 始终锁定为路径名，不在此重命名）。
    #[serde(default)]
    manifest: Option<McpManifest>,
}

/// 整体覆盖某资源的受管标签：先清空再按给定 id 集重建（去重、校验标签存在）。
/// `junction`/`fk` 为内部常量（skill_labels/skill_id 等）。
async fn replace_resource_labels(
    db: &Db,
    junction: &str,
    fk: &str,
    rid: i64,
    label_ids: &[i64],
) -> Result<(), ApiError> {
    for lid in label_ids {
        let ok = db
            .query_opt("SELECT 1 FROM labels WHERE id = ?1", db_params![lid])
            .await?
            .is_some();
        if !ok {
            return Err(ApiError::bad_request("指定的标签不存在"));
        }
    }
    db.execute(
        &format!("DELETE FROM {junction} WHERE {fk} = ?1"),
        db_params![rid],
    )
    .await?;
    for lid in label_ids {
        db.execute(
            &format!("INSERT INTO {junction}({fk}, label_id) VALUES (?1, ?2) ON CONFLICT DO NOTHING"),
            db_params![rid, lid],
        )
        .await?;
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
    let oid: Option<i64> = match state
        .db
        .query_opt("SELECT id FROM users WHERE username = ?1", db_params![owner])
        .await?
    {
        Some(r) => Some(r.get(0)?),
        None => None,
    };
    let oid = oid.ok_or_else(|| ApiError::not_found("未找到该技能"))?;
    if let Some(vis) = req.visibility.as_deref() {
        check_visibility(vis)?;
        state
            .db
            .execute(
                "UPDATE skills SET visibility = ?1, updated_at = ?2 WHERE owner_id = ?3 AND name = ?4",
                db_params![vis, now, oid, name],
            )
            .await?;
    }
    // 内容编辑：分类 / 版本 / 描述 / 标签 / SKILL.md / 依赖与倾向工具。
    if let Some(cat) = req.category.as_deref() {
        if !SKILL_CATEGORIES.contains(&cat) {
            return Err(ApiError::bad_request(format!(
                "category 只能是 {}",
                SKILL_CATEGORIES.join(" / ")
            )));
        }
        state
            .db
            .execute(
                "UPDATE skills SET category = ?1, updated_at = ?2 WHERE owner_id = ?3 AND name = ?4",
                db_params![cat, now, oid, name],
            )
            .await?;
    }
    if let Some(ver) = req.version.as_deref() {
        state
            .db
            .execute(
                "UPDATE skills SET version = ?1, updated_at = ?2 WHERE owner_id = ?3 AND name = ?4",
                db_params![ver, now, oid, name],
            )
            .await?;
        // 保持版本历史一致：改版本号视作以当前快照内容发布该版本（同号覆盖，旧版本保留）。
        state
            .db
            .execute(
                "INSERT INTO skill_versions(skill_id, version, skill_md, metadata,
                                        archive_sha256, archive_size, created_at)
             SELECT id, ?1, skill_md, metadata, archive_sha256, archive_size, ?2
             FROM skills WHERE owner_id = ?3 AND name = ?4
             ON CONFLICT(skill_id, version) DO UPDATE SET
                 skill_md=excluded.skill_md, metadata=excluded.metadata,
                 archive_sha256=excluded.archive_sha256, archive_size=excluded.archive_size,
                 created_at=excluded.created_at",
                db_params![ver, now, oid, name],
            )
            .await?;
    }
    if let Some(desc) = req.description.as_deref() {
        state
            .db
            .execute(
                "UPDATE skills SET description = ?1, updated_at = ?2 WHERE owner_id = ?3 AND name = ?4",
                db_params![desc, now, oid, name],
            )
            .await?;
    }
    if let Some(tags) = &req.tags {
        let tags_json = serde_json::to_string(&lower_tags(tags))
            .map_err(|e| ApiError::internal(e.to_string()))?;
        state
            .db
            .execute(
                "UPDATE skills SET tags = ?1, updated_at = ?2 WHERE owner_id = ?3 AND name = ?4",
                db_params![tags_json, now, oid, name],
            )
            .await?;
    }
    if let Some(md) = req.skill_md.as_deref() {
        state
            .db
            .execute(
                "UPDATE skills SET skill_md = ?1, updated_at = ?2 WHERE owner_id = ?3 AND name = ?4",
                db_params![md, now, oid, name],
            )
            .await?;
    }
    // metadata 列同时承载 mcp_dependencies 与 preferred_tools：读-改-写，缺省字段保留旧值。
    if req.mcp_dependencies.is_some() || req.preferred_tools.is_some() {
        let meta_json: Option<String> = match state
            .db
            .query_opt(
                "SELECT metadata FROM skills WHERE owner_id = ?1 AND name = ?2",
                db_params![oid, name],
            )
            .await?
        {
            Some(r) => Some(r.get(0)?),
            None => None,
        };
        let mut meta: SkillMeta = meta_json
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default();
        if let Some(deps) = &req.mcp_dependencies {
            meta.mcp_dependencies = deps.clone();
        }
        if let Some(tools) = &req.preferred_tools {
            meta.preferred_tools = tools.clone();
        }
        let new_meta = serde_json::to_string(&meta).map_err(|e| ApiError::internal(e.to_string()))?;
        state
            .db
            .execute(
                "UPDATE skills SET metadata = ?1, updated_at = ?2 WHERE owner_id = ?3 AND name = ?4",
                db_params![new_meta, now, oid, name],
            )
            .await?;
    }
    if let Some(gv) = &req.group_visibility {
        let stored = normalize_group_vis(gv)?;
        state
            .db
            .execute(
                "UPDATE skills SET group_visibility = ?1, updated_at = ?2 WHERE owner_id = ?3 AND name = ?4",
                db_params![stored, now, oid, name],
            )
            .await?;
    }
    if let Some(label_ids) = &req.label_ids {
        let sid: i64 = match state
            .db
            .query_opt(
                "SELECT id FROM skills WHERE owner_id = ?1 AND name = ?2",
                db_params![oid, name],
            )
            .await?
        {
            Some(r) => r.get(0)?,
            None => return Err(ApiError::not_found("未找到该技能")),
        };
        replace_resource_labels(&state.db, "skill_labels", "skill_id", sid, label_ids).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn skill_delete(
    State(state): S,
    headers: HeaderMap,
    Path((owner, name)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    require_admin(&state, &headers)?;
    if skills::delete_skill_record(&state, &owner, &name).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::not_found("未找到该技能"))
    }
}

/// 管理后台：列出某技能已发布的全部版本副本（新→旧）。
pub async fn skill_versions(
    State(state): S,
    headers: HeaderMap,
    Path((owner, name)): Path<(String, String)>,
) -> Result<Json<Vec<SkillVersionInfo>>, ApiError> {
    require_admin(&state, &headers)?;
    let (sid, _) = admin_skill_head(&state.db, &owner, &name).await?;
    Ok(Json(skills::versions_of(&state.db, sid).await))
}

/// 管理后台：删除技能的指定版本副本（含失引用 blob 的清理）。
///
/// - 删除的是最新版时，自动把剩余最高版本提升为最新版（`skills` 快照随之恢复该版本内容）；
/// - 仅剩最后一个版本时拒绝——如需整体下架请直接删除技能。
///
/// 返回删除后的最新版本号与版本列表，供管理界面就地刷新。
pub async fn skill_version_delete(
    State(state): S,
    headers: HeaderMap,
    Path((owner, name, version)): Path<(String, String, String)>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_admin(&state, &headers)?;
    let now = now_string();
    let (sid, head) = admin_skill_head(&state.db, &owner, &name).await?;
    let sha: Option<String> = match state
        .db
        .query_opt(
            "SELECT archive_sha256 FROM skill_versions WHERE skill_id = ?1 AND version = ?2",
            db_params![sid, version],
        )
        .await?
    {
        Some(r) => Some(r.get(0)?),
        None => None,
    };
    let Some(sha) = sha else {
        return Err(ApiError::not_found(format!("未找到版本 {version}")));
    };
    let count: i64 = state
        .db
        .query_row(
            "SELECT COUNT(*) FROM skill_versions WHERE skill_id = ?1",
            db_params![sid],
        )
        .await?
        .get(0)?;
    if count <= 1 {
        return Err(ApiError::bad_request(
            "仅剩最后一个版本，不能单独删除；如需下架请直接删除整个技能",
        ));
    }
    state
        .db
        .execute(
            "DELETE FROM skill_versions WHERE skill_id = ?1 AND version = ?2",
            db_params![sid, version],
        )
        .await?;

    // 删除的是最新版 → 把剩余最高版本的副本内容恢复为 head 快照。
    if version == head {
        let remaining = skills::versions_of(&state.db, sid).await;
        if let Some(top) = remaining.first() {
            // MySQL 无 UPDATE…FROM：先取版本副本再普通 UPDATE（无并发风险，管理接口 + 唯一键定位）。
            let row = state
                .db
                .query_row(
                    "SELECT skill_md, metadata, archive_sha256, archive_size
                     FROM skill_versions WHERE skill_id = ?1 AND version = ?2",
                    db_params![sid, top.version],
                )
                .await?;
            let (v_md, v_meta, v_sha, v_size): (String, String, String, i64) =
                (row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?);
            state
                .db
                .execute(
                    "UPDATE skills SET version = ?1, skill_md = ?2, metadata = ?3,
                        archive_sha256 = ?4, archive_size = ?5, updated_at = ?6
                 WHERE id = ?7",
                    db_params![top.version, v_md, v_meta, v_sha, v_size, now, sid],
                )
                .await?;
        }
    }

    // GC 必须在 head 回退之后：删除最新版时，回退前 skills 快照仍引用被删版本的
    // blob，先 GC 会误判「仍被引用」而漏清。
    if !sha.is_empty() {
        skills::gc_blob_if_unreferenced(&state, &sha).await;
    }

    let head_now: String = state
        .db
        .query_row("SELECT version FROM skills WHERE id = ?1", db_params![sid])
        .await?
        .get(0)?;
    Ok(Json(serde_json::json!({
        "head": head_now,
        "versions": skills::versions_of(&state.db, sid).await,
    })))
}

/// 按 owner 用户名 + 技能名取 (skill_id, 最新版本号)。
async fn admin_skill_head(db: &Db, owner: &str, name: &str) -> Result<(i64, String), ApiError> {
    match db
        .query_opt(
            "SELECT s.id, s.version FROM skills s JOIN users u ON u.id = s.owner_id
         WHERE u.username = ?1 AND s.name = ?2",
            db_params![owner, name],
        )
        .await?
    {
        Some(r) => Ok((r.get(0)?, r.get(1)?)),
        None => Err(ApiError::not_found("未找到该技能")),
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
    let oid: Option<i64> = match state
        .db
        .query_opt("SELECT id FROM users WHERE username = ?1", db_params![owner])
        .await?
    {
        Some(r) => Some(r.get(0)?),
        None => None,
    };
    let oid = oid.ok_or_else(|| ApiError::not_found("未找到该 MCP"))?;
    if let Some(vis) = req.visibility.as_deref() {
        check_visibility(vis)?;
        state
            .db
            .execute(
                "UPDATE mcps SET visibility = ?1, updated_at = ?2 WHERE owner_id = ?3 AND name = ?4",
                db_params![vis, now, oid, name],
            )
            .await?;
    }
    // 内容编辑：覆盖式更新运行清单。name 锁定为路径名（管理面板不在此重命名，
    // 以免 owner/name 引用失效），version 随 manifest 同步。
    if let Some(mut manifest) = req.manifest.clone() {
        manifest.name = name.clone();
        match manifest.runtime {
            Runtime::Remote if manifest.url.as_deref().unwrap_or("").is_empty() => {
                return Err(ApiError::bad_request("remote 运行时必须提供 url"));
            }
            Runtime::Local if manifest.command.as_deref().unwrap_or("").is_empty() => {
                return Err(ApiError::bad_request("local 运行时必须提供 command"));
            }
            _ => {}
        }
        let manifest_json =
            serde_json::to_string(&manifest).map_err(|e| ApiError::internal(e.to_string()))?;
        state
            .db
            .execute(
                "UPDATE mcps SET manifest = ?1, version = ?2, updated_at = ?3
             WHERE owner_id = ?4 AND name = ?5",
                db_params![manifest_json, manifest.version, now, oid, name],
            )
            .await?;
    }
    if let Some(gv) = &req.group_visibility {
        let stored = normalize_group_vis(gv)?;
        state
            .db
            .execute(
                "UPDATE mcps SET group_visibility = ?1, updated_at = ?2 WHERE owner_id = ?3 AND name = ?4",
                db_params![stored, now, oid, name],
            )
            .await?;
    }
    if let Some(label_ids) = &req.label_ids {
        let mid: i64 = match state
            .db
            .query_opt(
                "SELECT id FROM mcps WHERE owner_id = ?1 AND name = ?2",
                db_params![oid, name],
            )
            .await?
        {
            Some(r) => r.get(0)?,
            None => return Err(ApiError::not_found("未找到该 MCP")),
        };
        replace_resource_labels(&state.db, "mcp_labels", "mcp_id", mid, label_ids).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn mcp_delete(
    State(state): S,
    headers: HeaderMap,
    Path((owner, name)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    require_admin(&state, &headers)?;
    let n = state
        .db
        .execute(
            "DELETE FROM mcps WHERE name = ?2 AND owner_id =
                (SELECT id FROM users WHERE username = ?1)",
            db_params![owner, name],
        )
        .await?;
    if n == 0 {
        return Err(ApiError::not_found("未找到该 MCP"));
    }
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// 批量配置：对一批技能 / MCP 一次性改可见性 / 可见分组 / 增删受管标签
// ---------------------------------------------------------------------------

/// 批量操作的资源定位：owner + name（与前端列表行一致，无需暴露内部 id）。
#[derive(Deserialize)]
pub struct BatchTarget {
    owner: String,
    name: String,
}

#[derive(Deserialize)]
pub struct BatchPatch {
    /// 资源类型：skill / mcp。决定操作哪张表。
    kind: String,
    /// 目标资源列表（owner/name）。
    targets: Vec<BatchTarget>,
    /// private / public；缺省则不改动可见性。
    #[serde(default)]
    visibility: Option<String>,
    /// "all" 或分组 id 数组；缺省则不改动可见分组。
    #[serde(default)]
    group_visibility: Option<serde_json::Value>,
    /// 追加的受管标签 id（合并式，去重，不影响未列出的标签）。
    #[serde(default)]
    add_label_ids: Vec<i64>,
    /// 移除的受管标签 id。
    #[serde(default)]
    remove_label_ids: Vec<i64>,
}

#[derive(Serialize)]
pub struct BatchFailure {
    owner: String,
    name: String,
    error: String,
}

#[derive(Serialize)]
pub struct BatchResult {
    updated: usize,
    failed: Vec<BatchFailure>,
}

/// 校验一组标签 id 均存在；任一不存在即报错。
async fn ensure_labels_exist(db: &Db, ids: &[i64]) -> Result<(), ApiError> {
    for lid in ids {
        let ok = db
            .query_opt("SELECT 1 FROM labels WHERE id = ?1", db_params![lid])
            .await?
            .is_some();
        if !ok {
            return Err(ApiError::bad_request("指定的标签不存在"));
        }
    }
    Ok(())
}

/// 批量配置技能 / MCP 的可见性、可见分组与受管标签。
/// 标签为「增删式」（add/remove），可见性 / 可见分组为「设置式」，均可单独或组合下发。
/// 逐条应用，单条失败记入 `failed` 不阻断其余（除非是标签不存在这类前置校验错误）。
pub async fn batch_update(
    State(state): S,
    headers: HeaderMap,
    Json(req): Json<BatchPatch>,
) -> Result<Json<BatchResult>, ApiError> {
    require_admin(&state, &headers)?;
    let (table, junction, fk) = match req.kind.as_str() {
        "skill" => ("skills", "skill_labels", "skill_id"),
        "mcp" => ("mcps", "mcp_labels", "mcp_id"),
        _ => return Err(ApiError::bad_request("kind 只能是 skill 或 mcp")),
    };
    if req.targets.is_empty() {
        return Err(ApiError::bad_request("targets 不能为空"));
    }
    if let Some(vis) = req.visibility.as_deref() {
        check_visibility(vis)?;
    }
    // 归一化可见分组一次，供所有目标复用。
    let group_vis = match &req.group_visibility {
        Some(gv) => Some(normalize_group_vis(gv)?),
        None => None,
    };
    let now = now_string();
    // 标签存在性前置校验（一次性），避免逐条重复查询与部分写入不一致。
    ensure_labels_exist(&state.db, &req.add_label_ids).await?;
    ensure_labels_exist(&state.db, &req.remove_label_ids).await?;

    let mut updated = 0usize;
    let mut failed: Vec<BatchFailure> = Vec::new();
    for t in &req.targets {
        match batch_apply_one(
            &state.db, table, junction, fk, t, req.visibility.as_deref(), group_vis.as_deref(),
            &req.add_label_ids, &req.remove_label_ids, &now,
        )
        .await
        {
            Ok(()) => updated += 1,
            Err(e) => failed.push(BatchFailure {
                owner: t.owner.clone(),
                name: t.name.clone(),
                error: e.message,
            }),
        }
    }
    Ok(Json(BatchResult { updated, failed }))
}

/// 对单个资源应用批量补丁：解析 id → 改可见性/分组 → 增删标签。
#[allow(clippy::too_many_arguments)]
async fn batch_apply_one(
    db: &Db,
    table: &str,
    junction: &str,
    fk: &str,
    target: &BatchTarget,
    visibility: Option<&str>,
    group_vis: Option<&str>,
    add_label_ids: &[i64],
    remove_label_ids: &[i64],
    now: &str,
) -> Result<(), ApiError> {
    // 解析 owner/name → 资源 id（junction 关联用）。
    let rid: Option<i64> = match db
        .query_opt(
            &format!(
                "SELECT r.id FROM {table} r JOIN users u ON u.id = r.owner_id
                 WHERE u.username = ?1 AND r.name = ?2"
            ),
            db_params![target.owner, target.name],
        )
        .await?
    {
        Some(r) => Some(r.get(0)?),
        None => None,
    };
    let rid = rid.ok_or_else(|| ApiError::not_found("资源不存在"))?;

    if let Some(vis) = visibility {
        db.execute(
            &format!("UPDATE {table} SET visibility = ?1, updated_at = ?2 WHERE id = ?3"),
            db_params![vis, now, rid],
        )
        .await?;
    }
    if let Some(gv) = group_vis {
        db.execute(
            &format!("UPDATE {table} SET group_visibility = ?1, updated_at = ?2 WHERE id = ?3"),
            db_params![gv, now, rid],
        )
        .await?;
    }
    for lid in add_label_ids {
        db.execute(
            &format!("INSERT INTO {junction}({fk}, label_id) VALUES (?1, ?2) ON CONFLICT DO NOTHING"),
            db_params![rid, lid],
        )
        .await?;
    }
    for lid in remove_label_ids {
        db.execute(
            &format!("DELETE FROM {junction} WHERE {fk} = ?1 AND label_id = ?2"),
            db_params![rid, lid],
        )
        .await?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 资源转移：批量转移选中资源 / 整户转移（用户注销时把名下资源转给他人）
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct AdminTransferReq {
    /// 资源类型：skill / mcp。
    kind: String,
    /// 目标资源列表（owner/name）。
    targets: Vec<BatchTarget>,
    /// 接收方用户名（必须已存在）。
    to_username: String,
}

/// 批量把选中的技能 / MCP 转移给另一个用户。逐条应用：与接收方既有资源重名、
/// 或已属于接收方的记入 `failed`，不阻断其余。
pub async fn transfer_resources(
    State(state): S,
    headers: HeaderMap,
    Json(req): Json<AdminTransferReq>,
) -> Result<Json<BatchResult>, ApiError> {
    require_admin(&state, &headers)?;
    let table = match req.kind.as_str() {
        "skill" => "skills",
        "mcp" => "mcps",
        _ => return Err(ApiError::bad_request("kind 只能是 skill 或 mcp")),
    };
    if req.targets.is_empty() {
        return Err(ApiError::bad_request("targets 不能为空"));
    }
    let to_username = req.to_username.trim().to_string();
    let now = now_string();
    let target_uid = super::routes::user_id_by_name(&state.db, &to_username)
        .await?
        .ok_or_else(|| ApiError::not_found("目标用户不存在"))?;

    let mut updated = 0usize;
    let mut failed: Vec<BatchFailure> = Vec::new();
    for t in &req.targets {
        match transfer_one(&state.db, table, t, target_uid, &now).await {
            Ok(()) => updated += 1,
            Err(e) => failed.push(BatchFailure {
                owner: t.owner.clone(),
                name: t.name.clone(),
                error: e.message,
            }),
        }
    }
    Ok(Json(BatchResult { updated, failed }))
}

/// 把单个资源转给 target_uid：解析当前 owner → 查重名 → 改 owner_id。
async fn transfer_one(
    db: &Db,
    table: &str,
    target: &BatchTarget,
    target_uid: i64,
    now: &str,
) -> Result<(), ApiError> {
    let from_uid: Option<i64> = match db
        .query_opt(
            "SELECT id FROM users WHERE username = ?1",
            db_params![target.owner],
        )
        .await?
    {
        Some(r) => Some(r.get(0)?),
        None => None,
    };
    let from_uid = from_uid.ok_or_else(|| ApiError::not_found("资源归属用户不存在"))?;
    if from_uid == target_uid {
        return Err(ApiError::bad_request("已属于该用户"));
    }
    let taken = db
        .query_opt(
            &format!("SELECT 1 FROM {table} WHERE owner_id = ?1 AND name = ?2"),
            db_params![target_uid, target.name],
        )
        .await?
        .is_some();
    if taken {
        return Err(ApiError::conflict("接收方已有同名资源"));
    }
    let n = db
        .execute(
            &format!("UPDATE {table} SET owner_id = ?1, updated_at = ?2 WHERE owner_id = ?3 AND name = ?4"),
            db_params![target_uid, now, from_uid, target.name],
        )
        .await?;
    // MySQL 仅统计实际变更行：无变化的 UPDATE（如转让给自己）返回 0，须复核存在性再判 404。
    if n == 0 {
        let exists: bool = db
            .query_row(
                &format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE owner_id = ?1 AND name = ?2)"),
                db_params![from_uid, target.name],
            )
            .await?
            .get(0)?;
        if !exists {
            return Err(ApiError::not_found("资源不存在"));
        }
    }
    Ok(())
}

#[derive(Deserialize)]
pub struct UserTransferReq {
    /// 接收方用户名（必须已存在）。
    to_username: String,
}

#[derive(Serialize)]
pub struct UserTransferResult {
    skills_moved: usize,
    mcps_moved: usize,
    /// 因与接收方重名而跳过的资源（留在原账号名下）。
    skipped: Vec<String>,
}

/// 整户转移：把某用户名下全部技能与 MCP 转给另一个用户（用户注销前的资产交接）。
/// 与接收方重名的资源跳过并记入 `skipped`；加密凭据不随迁（属个人机密）。
pub async fn user_transfer(
    State(state): S,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Json(req): Json<UserTransferReq>,
) -> Result<Json<UserTransferResult>, ApiError> {
    require_admin(&state, &headers)?;
    let to_username = req.to_username.trim().to_string();
    let now = now_string();
    let exists = state
        .db
        .query_opt("SELECT 1 FROM users WHERE id = ?1", db_params![id])
        .await?
        .is_some();
    if !exists {
        return Err(ApiError::not_found("未找到该用户"));
    }
    let target_uid = super::routes::user_id_by_name(&state.db, &to_username)
        .await?
        .ok_or_else(|| ApiError::not_found("目标用户不存在"))?;
    if target_uid == id {
        return Err(ApiError::bad_request("不能转移给该用户自己"));
    }

    // 原为闭包；async 调用不能进闭包，改为嵌套 async fn（环境经参数显式传入）。
    async fn move_all(
        db: &Db,
        table: &str,
        kind: &str,
        from_uid: i64,
        target_uid: i64,
        now: &str,
        skipped: &mut Vec<String>,
    ) -> Result<usize, ApiError> {
        let names: Vec<String> = db
            .query_map(
                &format!("SELECT name FROM {table} WHERE owner_id = ?1 ORDER BY name"),
                db_params![from_uid],
                |r| r.get::<String>(0),
            )
            .await?;
        let mut moved = 0usize;
        for name in names {
            let taken = db
                .query_opt(
                    &format!("SELECT 1 FROM {table} WHERE owner_id = ?1 AND name = ?2"),
                    db_params![target_uid, name],
                )
                .await?
                .is_some();
            if taken {
                skipped.push(format!("{kind} {name}（接收方已有同名资源）"));
                continue;
            }
            db.execute(
                &format!("UPDATE {table} SET owner_id = ?1, updated_at = ?2 WHERE owner_id = ?3 AND name = ?4"),
                db_params![target_uid, now, from_uid, name],
            )
            .await?;
            moved += 1;
        }
        Ok(moved)
    }
    let mut skipped = Vec::new();
    let skills_moved =
        move_all(&state.db, "skills", "技能", id, target_uid, &now, &mut skipped).await?;
    let mcps_moved = move_all(&state.db, "mcps", "MCP", id, target_uid, &now, &mut skipped).await?;

    Ok(Json(UserTransferResult {
        skills_moved,
        mcps_moved,
        skipped,
    }))
}

// ---------------------------------------------------------------------------
// 系统设置：注册开关 + LDAP 认证配置 / 连通测试 / 用户同步
// ---------------------------------------------------------------------------

/// 读取认证配置（LDAP 绑定密码脱敏，仅返回是否已设置）。
pub async fn auth_settings_get(
    State(state): S,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_admin(&state, &headers)?;
    let s = settings::load(&state.db, &state.master_key).await;
    Ok(Json(settings::masked_json(&s)))
}

#[derive(Deserialize)]
pub struct AuthSettingsPatch {
    #[serde(default)]
    registration_enabled: Option<bool>,
    #[serde(default)]
    ldap: Option<LdapSettingsPatch>,
}

/// LDAP 配置增量：字段缺省表示保持不变；bind_password 传空串表示清除。
#[derive(Deserialize)]
pub struct LdapSettingsPatch {
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    start_tls: Option<bool>,
    #[serde(default)]
    no_tls_verify: Option<bool>,
    #[serde(default)]
    bind_dn: Option<String>,
    #[serde(default)]
    bind_password: Option<String>,
    #[serde(default)]
    user_base_dn: Option<String>,
    #[serde(default)]
    user_filter: Option<String>,
    #[serde(default)]
    username_attr: Option<String>,
    #[serde(default)]
    sync_base_dn: Option<String>,
}

fn apply_ldap_patch(cur: &mut settings::LdapSettings, p: LdapSettingsPatch) {
    if let Some(v) = p.enabled {
        cur.enabled = v;
    }
    if let Some(v) = p.url {
        cur.url = v.trim().to_string();
    }
    if let Some(v) = p.start_tls {
        cur.start_tls = v;
    }
    if let Some(v) = p.no_tls_verify {
        cur.no_tls_verify = v;
    }
    if let Some(v) = p.bind_dn {
        cur.bind_dn = v.trim().to_string();
    }
    if let Some(v) = p.bind_password {
        cur.bind_password = v;
    }
    if let Some(v) = p.user_base_dn {
        cur.user_base_dn = v.trim().to_string();
    }
    if let Some(v) = p.user_filter {
        cur.user_filter = v.trim().to_string();
    }
    if let Some(v) = p.username_attr {
        cur.username_attr = v.trim().to_string();
    }
    if let Some(v) = p.sync_base_dn {
        cur.sync_base_dn = v.trim().to_string();
    }
}

/// 更新认证配置（增量合并后整体落库）。启用 LDAP 时校验配置完整性。
pub async fn auth_settings_update(
    State(state): S,
    headers: HeaderMap,
    Json(req): Json<AuthSettingsPatch>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_admin(&state, &headers)?;
    let mut s = settings::load(&state.db, &state.master_key).await;
    if let Some(v) = req.registration_enabled {
        s.registration_enabled = v;
    }
    if let Some(p) = req.ldap {
        apply_ldap_patch(&mut s.ldap, p);
    }
    if s.ldap.enabled {
        ldap::validate(&s.ldap).map_err(ApiError::bad_request)?;
    }
    settings::save(&state.db, &state.master_key, &s).await?;
    Ok(Json(settings::masked_json(&s)))
}

#[derive(Deserialize)]
pub struct LdapTestReq {
    /// 可选：草稿配置（未保存也能试）；密码缺省回落到已保存的。
    #[serde(default)]
    ldap: Option<LdapSettingsPatch>,
    /// 可选：给一个真实账号跑完整认证链路（搜索 + 绑定）。
    #[serde(default)]
    username: Option<String>,
    #[serde(default)]
    password: Option<String>,
}

/// LDAP 连通性测试。始终返回 200，结论在 `{ ok, message }` 里，便于 UI 展示。
pub async fn ldap_test(
    State(state): S,
    headers: HeaderMap,
    Json(req): Json<LdapTestReq>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_admin(&state, &headers)?;
    let cfg = {
        let mut cfg = settings::load(&state.db, &state.master_key).await.ldap;
        if let Some(p) = req.ldap {
            apply_ldap_patch(&mut cfg, p);
        }
        cfg
    };
    let username = req.username.as_deref().unwrap_or("").trim().to_string();
    let result = if username.is_empty() {
        ldap::test_connection(&cfg).await
    } else {
        match ldap::authenticate(&cfg, &username, req.password.as_deref().unwrap_or("")).await {
            Ok(canonical) => Ok(format!("认证成功：目录用户 {canonical}")),
            Err(ldap::LdapAuthError::NotFound) => {
                Err("目录中搜不到该用户：请检查 user_base_dn / user_filter".into())
            }
            Err(ldap::LdapAuthError::BadCredentials) => {
                Err("找到了用户，但口令校验未通过".into())
            }
            Err(ldap::LdapAuthError::Other(e)) => Err(e),
        }
    };
    Ok(Json(match result {
        Ok(m) => serde_json::json!({ "ok": true, "message": m }),
        Err(m) => serde_json::json!({ "ok": false, "message": m }),
    }))
}

#[derive(Deserialize)]
pub struct LdapSyncReq {
    /// 同时以 `{ARGON2}<PHC>` 方案写入 userPassword（需目录支持 ARGON2，
    /// 如 OpenLDAP 2.5+ argon2 模块）；否则仅建条目，口令需在目录侧另行设置。
    #[serde(default)]
    sync_password_hashes: bool,
}

/// 把全部本地口令账号同步进 LDAP 目录（LDAP 影子账号本就来自目录，跳过）。
pub async fn ldap_sync(
    State(state): S,
    headers: HeaderMap,
    Json(req): Json<LdapSyncReq>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_admin(&state, &headers)?;
    let cfg = settings::load(&state.db, &state.master_key).await.ldap;
    let users: Vec<(String, String)> = state
        .db
        .query_map(
            "SELECT username, password_hash FROM users
                 WHERE auth_source != 'ldap' ORDER BY username",
            db_params![],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .await?;
    let report = ldap::sync_users(&cfg, &users, req.sync_password_hashes)
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, format!("LDAP 同步失败：{e}")))?;
    let pick = |action: &str| -> Vec<&String> {
        report
            .iter()
            .filter(|i| i.action == action)
            .map(|i| &i.username)
            .collect()
    };
    let failed: Vec<serde_json::Value> = report
        .iter()
        .filter(|i| i.action == "failed")
        .map(|i| serde_json::json!({ "username": i.username, "error": i.error }))
        .collect();
    Ok(Json(serde_json::json!({
        "total": report.len(),
        "created": pick("created"),
        "updated": pick("updated"),
        "skipped": pick("skipped"),
        "failed": failed,
    })))
}

// ---------------------------------------------------------------------------
// 外部系统注入：注册 MCP / 批量分发用户变量（供 aiko_hub 等上游推送）
// ---------------------------------------------------------------------------

/// 生成一个随机不可登录的口令哈希，供自动创建的占位用户使用
/// （外部系统 ensure_user / LDAP 影子账号共用）。
pub(super) fn random_password_hash() -> Result<String, ApiError> {
    let mut buf = [0u8; 32];
    rand::rngs::SysRng
        .try_fill_bytes(&mut buf)
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let hex: String = buf.iter().map(|b| format!("{b:02x}")).collect();
    auth::hash_password(&hex).map_err(|e| ApiError::internal(e.to_string()))
}

/// 取用户 id；不存在则按用户名自动创建（随机不可登录口令）。
/// 调用方须先校验用户名合法。
async fn ensure_user(db: &Db, username: &str) -> Result<i64, ApiError> {
    if let Some(row) = db
        .query_opt("SELECT id FROM users WHERE username = ?1", db_params![username])
        .await?
    {
        return Ok(row.get(0)?);
    }
    let hash = random_password_hash()?;
    Ok(db
        .insert_id(
            "INSERT INTO users(username, password_hash, created_at) VALUES (?1, ?2, ?3)",
            db_params![username, hash, now_string()],
        )
        .await?)
}

#[derive(Deserialize)]
pub struct AdminMcpRegisterReq {
    /// MCP 归属账号名（不存在则自动创建）。
    owner: String,
    manifest: McpManifest,
    /// private / public；默认 public。
    #[serde(default)]
    visibility: Option<String>,
    /// "all" 或分组 id 数组；默认 all。
    #[serde(default)]
    group_visibility: Option<serde_json::Value>,
    /// 可选工具清单，供市场检索 / 展示。
    #[serde(default)]
    tools: Option<Vec<ToolMeta>>,
}

/// 管理员注册 / 覆盖一条 MCP（按 (owner, name) 自然键 upsert）。供外部系统统一注入。
pub async fn mcp_register(
    State(state): S,
    headers: HeaderMap,
    Json(req): Json<AdminMcpRegisterReq>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_admin(&state, &headers)?;
    let owner = req.owner.trim().to_string();
    if !super::routes::is_valid_username(&owner) {
        return Err(ApiError::bad_request(
            "owner 用户名仅允许字母、数字、_、-，且长度 1..=64",
        ));
    }
    let manifest = req.manifest;
    if manifest.name.trim().is_empty() {
        return Err(ApiError::bad_request("manifest.name 不能为空"));
    }
    match manifest.runtime {
        Runtime::Remote if manifest.url.as_deref().unwrap_or("").is_empty() => {
            return Err(ApiError::bad_request("remote 运行时必须提供 url"));
        }
        Runtime::Local if manifest.command.as_deref().unwrap_or("").is_empty() => {
            return Err(ApiError::bad_request("local 运行时必须提供 command"));
        }
        _ => {}
    }
    let visibility = match req.visibility.as_deref().unwrap_or("public") {
        v @ ("private" | "public") => v.to_string(),
        _ => return Err(ApiError::bad_request("visibility 只能是 private 或 public")),
    };
    let group_vis = match &req.group_visibility {
        Some(v) => normalize_group_vis(v)?,
        None => "all".to_string(),
    };
    let manifest_json =
        serde_json::to_string(&manifest).map_err(|e| ApiError::internal(e.to_string()))?;
    let tools_json = match &req.tools {
        Some(tools) => {
            Some(serde_json::to_string(tools).map_err(|e| ApiError::internal(e.to_string()))?)
        }
        None => None,
    };
    let now = now_string();
    let oid = ensure_user(&state.db, &owner).await?;
    state
        .db
        .execute(
            "INSERT INTO mcps(owner_id, name, visibility, group_visibility, version, manifest, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(owner_id, name)
         DO UPDATE SET visibility=excluded.visibility, group_visibility=excluded.group_visibility,
                       version=excluded.version, manifest=excluded.manifest,
                       updated_at=excluded.updated_at",
            db_params![
                oid,
                manifest.name,
                visibility,
                group_vis,
                manifest.version,
                manifest_json,
                now
            ],
        )
        .await?;
    // tools 单独维护：仅当显式传入时覆盖（插入默认 '[]'，更新时不动旧值）。
    if let Some(tools_json) = tools_json {
        state
            .db
            .execute(
                "UPDATE mcps SET tools = ?1 WHERE owner_id = ?2 AND name = ?3",
                db_params![tools_json, oid, manifest.name],
            )
            .await?;
    }
    Ok(Json(serde_json::json!({
        "owner": owner,
        "name": manifest.name,
        "visibility": visibility,
        "updated_at": now,
    })))
}

#[derive(Deserialize)]
pub struct SecretEntry {
    username: String,
    value: String,
}

#[derive(Deserialize)]
pub struct SecretDistributeReq {
    key: String,
    entries: Vec<SecretEntry>,
}

#[derive(Serialize)]
pub struct SecretDistributeResp {
    applied: usize,
    skipped: Vec<String>,
}

/// 批量为多个用户写同一变量（按 (owner, key) upsert）。供外部系统分发用户 KEY。
/// 不存在的同名用户**跳过**（不自动创建），计入 `skipped`。
pub async fn secrets_distribute(
    State(state): S,
    headers: HeaderMap,
    Json(req): Json<SecretDistributeReq>,
) -> Result<Json<SecretDistributeResp>, ApiError> {
    require_admin(&state, &headers)?;
    let key = req.key.trim().to_string();
    if key.is_empty() {
        return Err(ApiError::bad_request("变量名不能为空"));
    }
    let now = now_string();
    let mut applied = 0usize;
    let mut skipped = Vec::new();
    for entry in &req.entries {
        let username = entry.username.trim();
        if !super::routes::is_valid_username(username) {
            skipped.push(format!("{username}（用户名非法）"));
            continue;
        }
        let oid: Option<i64> = match state
            .db
            .query_opt(
                "SELECT id FROM users WHERE username = ?1",
                db_params![username],
            )
            .await?
        {
            Some(r) => Some(r.get(0)?),
            None => None,
        };
        let Some(oid) = oid else {
            skipped.push(format!("{username}（用户不存在）"));
            continue;
        };
        let (nonce, ct) = crypto::encrypt(&state.master_key, &entry.value)?;
        state
            .db
            .execute(
                "INSERT INTO secrets(owner_id, key, nonce, ciphertext, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(owner_id, key)
             DO UPDATE SET nonce=excluded.nonce, ciphertext=excluded.ciphertext,
                           updated_at=excluded.updated_at",
                db_params![oid, key, nonce, ct, now],
            )
            .await?;
        applied += 1;
    }
    Ok(Json(SecretDistributeResp { applied, skipped }))
}

#[derive(Deserialize)]
pub struct UserProvisionReq {
    username: String,
    password: String,
    /// 可选：随账号一并注入的变量名（如 "AIKO_HUB_KEY"）。
    #[serde(default)]
    key: Option<String>,
    /// 可选：上述变量的值。
    #[serde(default)]
    value: Option<String>,
}

#[derive(Serialize)]
pub struct UserProvisionResp {
    username: String,
    created: bool,
    secret_set: bool,
}

/// 配给一个用户账号（外部系统如 aiko_hub 在登录 / 注册时调用）：按用户名 create-or-update，
/// 始终同步口令以保持一致；可选随账号注入一个变量。
pub async fn user_provision(
    State(state): S,
    headers: HeaderMap,
    Json(req): Json<UserProvisionReq>,
) -> Result<Json<UserProvisionResp>, ApiError> {
    require_admin(&state, &headers)?;
    let username = req.username.trim().to_string();
    if !super::routes::is_valid_username(&username) {
        return Err(ApiError::bad_request(
            "用户名仅允许字母、数字、_、-，且长度 1..=64",
        ));
    }
    if req.password.is_empty() {
        return Err(ApiError::bad_request("密码不能为空"));
    }
    let hash = auth::hash_password(&req.password).map_err(|e| ApiError::internal(e.to_string()))?;
    let now = now_string();
    let existing: Option<i64> = match state
        .db
        .query_opt(
            "SELECT id FROM users WHERE username = ?1",
            db_params![username],
        )
        .await?
    {
        Some(r) => Some(r.get(0)?),
        None => None,
    };
    let (uid, created) = match existing {
        Some(id) => {
            // 同步口令保持一致。
            state
                .db
                .execute(
                    "UPDATE users SET password_hash = ?1 WHERE id = ?2",
                    db_params![hash, id],
                )
                .await?;
            (id, false)
        }
        None => {
            let id = state
                .db
                .insert_id(
                    "INSERT INTO users(username, password_hash, created_at) VALUES (?1, ?2, ?3)",
                    db_params![username, hash, now],
                )
                .await?;
            (id, true)
        }
    };
    let mut secret_set = false;
    if let (Some(key), Some(value)) = (req.key.as_deref(), req.value.as_deref()) {
        let key = key.trim();
        if !key.is_empty() {
            let (nonce, ct) = crypto::encrypt(&state.master_key, value)?;
            state
                .db
                .execute(
                    "INSERT INTO secrets(owner_id, key, nonce, ciphertext, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(owner_id, key)
                 DO UPDATE SET nonce=excluded.nonce, ciphertext=excluded.ciphertext,
                               updated_at=excluded.updated_at",
                    db_params![uid, key, nonce, ct, now],
                )
                .await?;
            secret_set = true;
        }
    }
    Ok(Json(UserProvisionResp {
        username,
        created,
        secret_set,
    }))
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
    /// v2：技能版本历史（含各版本压缩体指针）。旧包缺省为空，导入时以 head 快照补录。
    #[serde(default)]
    versions: Vec<PackSkillVersion>,
    /// v2：点赞/收藏互动。
    #[serde(default)]
    reactions: Vec<PackReaction>,
    /// v2：系统设置（settings 表原文；敏感字段本就以 master key 加密，跨实例导入须共用 master.key）。
    #[serde(default)]
    settings: Vec<PackSetting>,
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
    /// 认证来源（旧资源包缺省视为 local）。
    #[serde(default = "default_auth_source")]
    auth_source: String,
    created_at: String,
}

fn default_auth_source() -> String {
    "local".into()
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
    /// 压缩体累计下载次数（旧资源包缺省为 0）。
    #[serde(default)]
    downloads: i64,
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
    #[serde(default)]
    result: String,
    ms: i64,
    created_at: String,
    created_ts: i64,
}

#[derive(Serialize, Deserialize)]
struct PackSkillVersion {
    owner: String,
    skill: String,
    version: String,
    skill_md: String,
    metadata: serde_json::Value,
    archive_sha256: String,
    archive_size: i64,
    created_at: String,
}

#[derive(Serialize, Deserialize)]
struct PackReaction {
    user: String,
    /// 资源类型："skill" / "mcp"。
    resource: String,
    owner: String,
    name: String,
    /// 'like' / 'favorite'。
    kind: String,
    created_at: String,
}

#[derive(Serialize, Deserialize)]
struct PackSetting {
    key: String,
    value: String,
}

/// 导出全量资源包：`GET /v1/admin/export` → `.tskpack`（tar + zstd）下载。
pub async fn export(State(state): S, headers: HeaderMap) -> Result<Response, ApiError> {
    require_admin(&state, &headers)?;
    let pack = collect_pack(&state).await?;
    let json = serde_json::to_vec_pretty(&pack)
        .map_err(|e| ApiError::internal(format!("序列化资源包失败: {e}")))?;

    // 收集被引用的 blob（去重）及其落盘路径：head 快照 + 全部版本副本。
    let mut blob_files: Vec<(String, PathBuf)> = Vec::new();
    let mut seen = BTreeSet::new();
    let head_shas = pack.skills.iter().map(|s| &s.archive_sha256);
    let version_shas = pack.versions.iter().map(|v| &v.archive_sha256);
    for sha in head_shas.chain(version_shas) {
        if sha.is_empty() || !seen.insert(sha.clone()) {
            continue;
        }
        if let Some(path) = skills::find_blob(&state, sha) {
            blob_files.push((sha.clone(), path));
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
    /// v2 新增三节的导入计数（导入 v1 旧包时为 0）。
    versions: usize,
    reactions: usize,
    settings: usize,
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

    let mut summary = apply_pack(&state, &pack).await?;
    summary.blobs = blob_n;
    Ok(Json(summary))
}

/// 读取全部数据库行，组装资源包（不含 blob 字节；blob 由调用方按 sha 落盘进 tar）。
async fn collect_pack(state: &AppState) -> Result<Pack, ApiError> {
    let groups = state
        .db
        .query_map(
            "SELECT name, description, created_at FROM groups ORDER BY id",
            db_params![],
            |r| {
                Ok(PackGroup {
                    name: r.get(0)?,
                    description: r.get(1)?,
                    created_at: r.get(2)?,
                })
            },
        )
        .await?;

    let labels = state
        .db
        .query_map(
            "SELECT name, created_at FROM labels ORDER BY id",
            db_params![],
            |r| {
                Ok(PackLabel {
                    name: r.get(0)?,
                    created_at: r.get(1)?,
                })
            },
        )
        .await?;

    // 资源 → 标签名映射，供 mcps/skills 装配。
    let mcp_label_map =
        super::routes::all_resource_labels(&state.db, "mcp_labels", "mcp_id").await;
    let skill_label_map =
        super::routes::all_resource_labels(&state.db, "skill_labels", "skill_id").await;

    let users = {
        // 先聚合每个用户名的分组列表（按用户名映射，便于装配 PackUser）。
        let mut by_user: HashMap<String, Vec<String>> = HashMap::new();
        let rows = state
            .db
            .query_map(
                "SELECT u.username, g.name FROM user_groups ug
                     JOIN users u ON u.id = ug.user_id
                     JOIN groups g ON g.id = ug.group_id ORDER BY g.name",
                db_params![],
                |r| Ok((r.get::<String>(0)?, r.get::<String>(1)?)),
            )
            .await?;
        for (uname, gname) in rows {
            by_user.entry(uname).or_default().push(gname);
        }
        let rows = state
            .db
            .query_map(
                "SELECT username, password_hash, auth_source, created_at FROM users ORDER BY id",
                db_params![],
                |r| {
                    Ok((
                        r.get::<String>(0)?,
                        r.get::<String>(1)?,
                        r.get::<String>(2)?,
                        r.get::<String>(3)?,
                    ))
                },
            )
            .await?;
        let mut out = Vec::new();
        for (username, password_hash, auth_source, created_at) in rows {
            out.push(PackUser {
                groups: by_user.remove(&username).unwrap_or_default(),
                group: String::new(),
                username,
                password_hash,
                auth_source,
                created_at,
            });
        }
        out
    };

    let mcps = state
        .db
        .query_map(
            "SELECT u.username, m.name, m.visibility, m.version, m.manifest, m.tools, m.updated_at,
                        m.group_visibility, m.id
                 FROM mcps m JOIN users u ON u.id = m.owner_id ORDER BY m.id",
            db_params![],
            |r| {
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
            },
        )
        .await?;

    let skills = state
        .db
        .query_map(
            "SELECT u.username, s.name, s.category, s.visibility, s.version, s.description,
                        s.tags, s.skill_md, s.metadata, s.archive_sha256, s.archive_size, s.updated_at,
                        s.group_visibility, s.id, s.downloads
                 FROM skills s JOIN users u ON u.id = s.owner_id ORDER BY s.id",
            db_params![],
            |r| {
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
                    downloads: r.get(14)?,
                    updated_at: r.get(11)?,
                    group_visibility: r.get(12)?,
                    labels: skill_label_map.get(&id).cloned().unwrap_or_default(),
                })
            },
        )
        .await?;

    let secrets = state
        .db
        .query_map(
            "SELECT u.username, s.key, s.nonce, s.ciphertext, s.updated_at
                 FROM secrets s JOIN users u ON u.id = s.owner_id ORDER BY s.id",
            db_params![],
            |r| {
                let nonce: Vec<u8> = r.get(2)?;
                let ct: Vec<u8> = r.get(3)?;
                Ok(PackSecret {
                    owner: r.get(0)?,
                    key: r.get(1)?,
                    nonce_b64: STANDARD.encode(nonce),
                    ciphertext_b64: STANDARD.encode(ct),
                    updated_at: r.get(4)?,
                })
            },
        )
        .await?;

    let calls = state
        .db
        .query_map(
            "SELECT caller, owner, mcp_name, tool, ok, error, result, ms, created_at, created_ts
                 FROM tool_calls ORDER BY id",
            db_params![],
            |r| {
                Ok(PackCall {
                    caller: r.get(0)?,
                    owner: r.get(1)?,
                    mcp_name: r.get(2)?,
                    tool: r.get(3)?,
                    ok: r.get(4)?,
                    error: r.get(5)?,
                    result: r.get(6)?,
                    ms: r.get(7)?,
                    created_at: r.get(8)?,
                    created_ts: r.get(9)?,
                })
            },
        )
        .await?;

    // v2：技能版本历史（owner/skill 用自然键，导入侧按名字回解）。
    let versions = state
        .db
        .query_map(
            "SELECT u.username, s.name, v.version, v.skill_md, v.metadata,
                    v.archive_sha256, v.archive_size, v.created_at
                 FROM skill_versions v
                 JOIN skills s ON s.id = v.skill_id
                 JOIN users u ON u.id = s.owner_id ORDER BY v.id",
            db_params![],
            |r| {
                let metadata: String = r.get(4)?;
                Ok(PackSkillVersion {
                    owner: r.get(0)?,
                    skill: r.get(1)?,
                    version: r.get(2)?,
                    skill_md: r.get(3)?,
                    metadata: serde_json::from_str(&metadata).unwrap_or(serde_json::json!({})),
                    archive_sha256: r.get(5)?,
                    archive_size: r.get(6)?,
                    created_at: r.get(7)?,
                })
            },
        )
        .await?;

    // v2：点赞/收藏（技能与 MCP 两张联结表合并为一节）。
    let mut reactions = state
        .db
        .query_map(
            "SELECT ru.username, ou.username, s.name, r.kind, r.created_at
                 FROM skill_reactions r
                 JOIN users ru ON ru.id = r.user_id
                 JOIN skills s ON s.id = r.skill_id
                 JOIN users ou ON ou.id = s.owner_id",
            db_params![],
            |r| {
                Ok(PackReaction {
                    user: r.get(0)?,
                    resource: "skill".into(),
                    owner: r.get(1)?,
                    name: r.get(2)?,
                    kind: r.get(3)?,
                    created_at: r.get(4)?,
                })
            },
        )
        .await?;
    reactions.extend(
        state
            .db
            .query_map(
                "SELECT ru.username, ou.username, m.name, r.kind, r.created_at
                 FROM mcp_reactions r
                 JOIN users ru ON ru.id = r.user_id
                 JOIN mcps m ON m.id = r.mcp_id
                 JOIN users ou ON ou.id = m.owner_id",
                db_params![],
                |r| {
                    Ok(PackReaction {
                        user: r.get(0)?,
                        resource: "mcp".into(),
                        owner: r.get(1)?,
                        name: r.get(2)?,
                        kind: r.get(3)?,
                        created_at: r.get(4)?,
                    })
                },
            )
            .await?,
    );

    // v2：系统设置（注册开关、LDAP 配置等；敏感字段入库前已加密，原样携带）。
    let settings = state
        .db
        .query_map(
            "SELECT key, value FROM settings",
            db_params![],
            |r| {
                Ok(PackSetting {
                    key: r.get(0)?,
                    value: r.get(1)?,
                })
            },
        )
        .await?;

    Ok(Pack {
        version: PACK_VERSION,
        exported_at: now_string(),
        groups,
        users,
        mcps,
        skills,
        secrets,
        calls,
        versions,
        reactions,
        settings,
        labels,
    })
}

/// 把资源包写入数据库（一个事务内 upsert）。
async fn apply_pack(state: &AppState, pack: &Pack) -> Result<ImportSummary, ApiError> {
    let mut tx = state.db.begin().await?;
    let mut skipped = Vec::new();

    // 分组先行 upsert（按名字自然键），并构建 名字 → 本地 id 映射（含目标既有分组）。
    for g in &pack.groups {
        tx.execute(
            "INSERT INTO groups(name, description, created_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(name) DO UPDATE SET description = excluded.description",
            db_params![g.name, g.description, g.created_at],
        )
        .await?;
    }
    let mut gids: HashMap<String, i64> = HashMap::new();
    let rows = tx
        .query_map("SELECT name, id FROM groups", db_params![], |r| {
            Ok((r.get::<String>(0)?, r.get::<i64>(1)?))
        })
        .await?;
    for (name, id) in rows {
        gids.insert(name, id);
    }

    // 标签 upsert（按名字自然键），并构建 名字 → 本地 id 映射。
    for l in &pack.labels {
        tx.execute(
            "INSERT INTO labels(name, created_at) VALUES (?1, ?2)
             ON CONFLICT(name) DO NOTHING",
            db_params![l.name, l.created_at],
        )
        .await?;
    }
    let mut lids: HashMap<String, i64> = HashMap::new();
    let rows = tx
        .query_map("SELECT name, id FROM labels", db_params![], |r| {
            Ok((r.get::<String>(0)?, r.get::<i64>(1)?))
        })
        .await?;
    for (name, id) in rows {
        lids.insert(name, id);
    }

    for u in &pack.users {
        tx.execute(
            "INSERT INTO users(username, password_hash, auth_source, created_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(username) DO UPDATE SET password_hash = excluded.password_hash,
                                                 auth_source = excluded.auth_source",
            db_params![u.username, u.password_hash, u.auth_source, u.created_at],
        )
        .await?;
        let uid: i64 = tx
            .query_row(
                "SELECT id FROM users WHERE username = ?1",
                db_params![u.username],
            )
            .await?
            .get(0)?;
        // 分组归属按名字解析并合并（不删除目标已有关联，符合 upsert 语义）。
        // 兼容旧资源包：单分组 group 字段并入。
        let mut names: Vec<&String> = u.groups.iter().collect();
        if !u.group.is_empty() && !names.iter().any(|n| *n == &u.group) {
            names.push(&u.group);
        }
        for name in names {
            if let Some(&gid) = gids.get(name) {
                tx.execute(
                    "INSERT INTO user_groups(user_id, group_id) VALUES (?1, ?2) ON CONFLICT DO NOTHING",
                    db_params![uid, gid],
                )
                .await?;
            }
        }
    }

    // 用户名 → 本地 id 映射（含目标实例既有用户）。
    let mut ids: HashMap<String, i64> = HashMap::new();
    let rows = tx
        .query_map("SELECT username, id FROM users", db_params![], |r| {
            Ok((r.get::<String>(0)?, r.get::<i64>(1)?))
        })
        .await?;
    for (name, id) in rows {
        ids.insert(name, id);
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
            db_params![oid, m.name, m.visibility, gv, m.version, manifest, tools, m.updated_at],
        )
        .await?;
        // 标签关联（合并，不删除目标已有）。按名字解析为本地 label id。
        if !m.labels.is_empty() {
            let mid: i64 = tx
                .query_row(
                    "SELECT id FROM mcps WHERE owner_id = ?1 AND name = ?2",
                    db_params![oid, m.name],
                )
                .await?
                .get(0)?;
            for lname in &m.labels {
                if let Some(&lid) = lids.get(lname) {
                    tx.execute(
                        "INSERT INTO mcp_labels(mcp_id, label_id) VALUES (?1, ?2) ON CONFLICT DO NOTHING",
                        db_params![mid, lid],
                    )
                    .await?;
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
                                tags, skill_md, metadata, archive_sha256, archive_size, downloads, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
             ON CONFLICT(owner_id, name) DO UPDATE SET category=excluded.category,
                 visibility=excluded.visibility, group_visibility=excluded.group_visibility,
                 version=excluded.version,
                 description=excluded.description, tags=excluded.tags, skill_md=excluded.skill_md,
                 metadata=excluded.metadata, archive_sha256=excluded.archive_sha256,
                 archive_size=excluded.archive_size, downloads=excluded.downloads,
                 updated_at=excluded.updated_at",
            db_params![
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
                s.downloads,
                s.updated_at
            ],
        )
        .await?;
        // 标签关联（合并，不删除目标已有）。
        if !s.labels.is_empty() {
            let sid: i64 = tx
                .query_row(
                    "SELECT id FROM skills WHERE owner_id = ?1 AND name = ?2",
                    db_params![oid, s.name],
                )
                .await?
                .get(0)?;
            for lname in &s.labels {
                if let Some(&lid) = lids.get(lname) {
                    tx.execute(
                        "INSERT INTO skill_labels(skill_id, label_id) VALUES (?1, ?2) ON CONFLICT DO NOTHING",
                        db_params![sid, lid],
                    )
                    .await?;
                }
            }
        }
        skills += 1;
    }

    // v2：技能版本历史（按 (owner, skill, version) 自然键 upsert；owner/技能缺失则跳过）。
    let mut versions_n = 0usize;
    for v in &pack.versions {
        let Some(&oid) = ids.get(&v.owner) else {
            skipped.push(format!("version {}/{}@{} (owner 缺失)", v.owner, v.skill, v.version));
            continue;
        };
        let sid: Option<i64> = match tx
            .query_opt(
                "SELECT id FROM skills WHERE owner_id = ?1 AND name = ?2",
                db_params![oid, v.skill],
            )
            .await?
        {
            Some(r) => Some(r.get(0)?),
            None => None,
        };
        let Some(sid) = sid else {
            skipped.push(format!("version {}/{}@{} (技能缺失)", v.owner, v.skill, v.version));
            continue;
        };
        tx.execute(
            "INSERT INTO skill_versions(skill_id, version, skill_md, metadata,
                                        archive_sha256, archive_size, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(skill_id, version) DO UPDATE SET skill_md=excluded.skill_md,
                 metadata=excluded.metadata, archive_sha256=excluded.archive_sha256,
                 archive_size=excluded.archive_size, created_at=excluded.created_at",
            db_params![
                sid,
                v.version,
                v.skill_md,
                v.metadata.to_string(),
                v.archive_sha256,
                v.archive_size,
                v.created_at
            ],
        )
        .await?;
        versions_n += 1;
    }
    // 旧版（v1）资源包不含版本历史：把导入技能的 head 快照补录为一个版本副本（幂等，
    // 与 schema::init 的回填同语义）。
    if pack.version < 2 {
        tx.execute(
            "INSERT INTO skill_versions(skill_id, version, skill_md, metadata,
                                        archive_sha256, archive_size, created_at)
             SELECT id, version, skill_md, metadata, archive_sha256, archive_size, updated_at
             FROM skills WHERE true
             ON CONFLICT DO NOTHING",
            db_params![],
        )
        .await?;
    }

    // v2：点赞/收藏（自然键解析；用户或资源缺失跳过；合并语义 DO NOTHING）。
    let mut reactions_n = 0usize;
    for re in &pack.reactions {
        let (Some(&ruid), Some(&oid)) = (ids.get(&re.user), ids.get(&re.owner)) else {
            skipped.push(format!("reaction {} → {}/{} (用户缺失)", re.user, re.owner, re.name));
            continue;
        };
        let (table, res_table, fk) = match re.resource.as_str() {
            "skill" => ("skill_reactions", "skills", "skill_id"),
            "mcp" => ("mcp_reactions", "mcps", "mcp_id"),
            other => {
                skipped.push(format!("reaction 未知资源类型: {other}"));
                continue;
            }
        };
        let rid: Option<i64> = match tx
            .query_opt(
                &format!("SELECT id FROM {res_table} WHERE owner_id = ?1 AND name = ?2"),
                db_params![oid, re.name],
            )
            .await?
        {
            Some(r) => Some(r.get(0)?),
            None => None,
        };
        let Some(rid) = rid else {
            skipped.push(format!("reaction {} → {}/{} (资源缺失)", re.user, re.owner, re.name));
            continue;
        };
        tx.execute(
            &format!(
                "INSERT INTO {table}(user_id, {fk}, kind, created_at) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT DO NOTHING"
            ),
            db_params![ruid, rid, re.kind, re.created_at],
        )
        .await?;
        reactions_n += 1;
    }

    // v2：系统设置（key 级覆盖；值内敏感字段以 master key 加密，跨实例须共用 master.key）。
    let mut settings_n = 0usize;
    for s in &pack.settings {
        tx.execute(
            "INSERT INTO settings(key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            db_params![s.key, s.value],
        )
        .await?;
        settings_n += 1;
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
            db_params![oid, sec.key, nonce, ct, sec.updated_at],
        )
        .await?;
        secrets += 1;
    }

    // 调用日志：仅当目标为空表时导入，避免重复导入造成统计翻倍。
    let mut calls = 0usize;
    let existing: i64 = tx
        .query_row("SELECT COUNT(*) FROM tool_calls", db_params![])
        .await?
        .get(0)?;
    if existing == 0 {
        for c in &pack.calls {
            tx.execute(
                "INSERT INTO tool_calls(caller, owner, mcp_name, tool, ok, error, result, ms, created_at, created_ts)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                db_params![
                    c.caller, c.owner, c.mcp_name, c.tool, c.ok, c.error, c.result, c.ms,
                    c.created_at, c.created_ts
                ],
            )
            .await?;
            calls += 1;
        }
    } else if !pack.calls.is_empty() {
        skipped.push(format!("调用日志 {} 条（目标已有日志，跳过以免重复）", pack.calls.len()));
    }

    tx.commit().await?;
    Ok(ImportSummary {
        groups: pack.groups.len(),
        users: pack.users.len(),
        mcps,
        skills,
        secrets,
        calls,
        blobs: 0,
        versions: versions_n,
        reactions: reactions_n,
        settings: settings_n,
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
