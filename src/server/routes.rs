//! Hub 开放 API 路由与处理器。

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{delete, get, patch, post};
use axum::{Json, Router};
use rusqlite::OptionalExtension;

use crate::shared::{
    AuthReq, AuthResp, CallReq, McpInfo, McpManifest, McpRenameReq, McpUpsertReq, ReportCallReq,
    ResolveResp, SecretInfo, SecretSetReq, SetToolsReq, ToolMeta, stitch,
};

use super::auth;
use super::admin;
use super::crypto;
use super::error::ApiError;
use super::skills;
use super::web;
use super::AppState;

type S = State<Arc<AppState>>;

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(web::index))
        .route("/healthz", get(health))
        .route("/v1/auth/register", post(register))
        .route("/v1/auth/login", post(login))
        .route("/v1/whoami", get(whoami))
        .route("/v1/explore", get(explore))
        .route("/v1/labels", get(label_names))
        .route("/v1/mcp", get(mcp_list).post(mcp_upsert))
        .route("/v1/mcp/:name", delete(mcp_delete))
        .route("/v1/mcp/:name/rename", post(mcp_rename))
        .route("/v1/mcp/:name/tools", post(mcp_set_tools))
        .route("/v1/mcp/:owner/:name", get(mcp_get))
        // 技能市场
        .route("/v1/skill/explore", get(skills::explore))
        .route("/v1/skill", get(skills::list_mine).post(skills::upsert))
        .route(
            "/v1/skill/:owner/:name",
            get(skills::get).delete(skills::delete),
        )
        .route("/v1/skill/:owner/:name/rename", post(skills::rename))
        .route(
            "/v1/skill/:owner/:name/archive",
            get(skills::archive_get).put(skills::archive_put),
        )
        .route("/v1/secret", get(secret_list).put(secret_set))
        .route("/v1/secret/:key", delete(secret_delete))
        .route("/v1/run/:owner/:name/resolve", post(run_resolve))
        .route("/v1/run/:owner/:name/call", post(run_call))
        .route("/v1/run/:owner/:name/report", post(run_report))
        // 管理后台（需 ADMIN_TOKEN）
        .route("/v1/admin/stats", get(admin::stats))
        .route("/v1/admin/users", get(admin::users).post(admin::user_create))
        .route(
            "/v1/admin/users/:id",
            patch(admin::user_update).delete(admin::user_delete),
        )
        .route(
            "/v1/admin/groups",
            get(admin::groups).post(admin::group_create),
        )
        .route(
            "/v1/admin/groups/:id",
            patch(admin::group_update).delete(admin::group_delete),
        )
        .route(
            "/v1/admin/labels",
            get(admin::labels).post(admin::label_create),
        )
        .route(
            "/v1/admin/labels/:id",
            patch(admin::label_update).delete(admin::label_delete),
        )
        .route("/v1/admin/skills", get(admin::skills_all))
        .route(
            "/v1/admin/skills/:owner/:name",
            patch(admin::skill_update).delete(admin::skill_delete),
        )
        .route("/v1/admin/mcps", get(admin::mcps_all))
        .route(
            "/v1/admin/mcps/:owner/:name",
            patch(admin::mcp_update).delete(admin::mcp_delete),
        )
        .route("/v1/admin/calls", get(admin::calls))
        .route("/v1/admin/export", get(admin::export))
        .route("/v1/admin/import", post(admin::import))
        // 技能压缩体可能较大，放宽请求体上限至 512 MiB
        .layer(axum::extract::DefaultBodyLimit::max(512 * 1024 * 1024))
        // 调试期：放行跨源前端（Tauri webview / 浏览器）直连本 Hub。
        // 反射任意 Origin/Method/Header，便于本地联调；生产部署应收紧。
        .layer(tower_http::cors::CorsLayer::very_permissive())
        // 内置 Web UI 静态资源 + SPA 回退
        .fallback(web::static_handler)
        .with_state(state)
}

async fn health() -> &'static str {
    "triskelion hub ok"
}

/// 公开标签清单：列出全部受管标签名（供市场筛选）。无需鉴权。
async fn label_names(State(state): S) -> Result<Json<Vec<String>>, ApiError> {
    let conn = state.db.lock().unwrap();
    let mut stmt = conn
        .prepare("SELECT name FROM labels ORDER BY name")
        .map_err(db_err)?;
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(db_err)?;
    Ok(Json(rows.filter_map(|r| r.ok()).collect()))
}

// ---------------------------------------------------------------------------
// auth
// ---------------------------------------------------------------------------

fn valid_username(u: &str) -> bool {
    !u.is_empty()
        && u.len() <= 64
        && u.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// 供管理后台创建用户时复用的用户名校验。
pub(super) fn is_valid_username(u: &str) -> bool {
    valid_username(u)
}

async fn register(
    State(state): S,
    Json(req): Json<AuthReq>,
) -> Result<Json<AuthResp>, ApiError> {
    if !valid_username(&req.username) {
        return Err(ApiError::bad_request(
            "用户名仅允许字母、数字、_、-，且长度 1..=64",
        ));
    }
    if req.password.len() < 6 {
        return Err(ApiError::bad_request("密码至少 6 位"));
    }
    let hash = auth::hash_password(&req.password)?;
    let now = now_string();
    let conn = state.db.lock().unwrap();
    let exists: bool = conn
        .query_row(
            "SELECT 1 FROM users WHERE username = ?1",
            [&req.username],
            |_| Ok(true),
        )
        .optional()
        .map_err(db_err)?
        .unwrap_or(false);
    if exists {
        return Err(ApiError::conflict("用户名已存在"));
    }
    conn.execute(
        "INSERT INTO users(username, password_hash, created_at) VALUES (?1, ?2, ?3)",
        rusqlite::params![req.username, hash, now],
    )
    .map_err(db_err)?;
    let id = conn.last_insert_rowid();
    drop(conn);
    let token = auth::issue_token(&state.jwt_secret, id, &req.username)?;
    Ok(Json(AuthResp {
        token,
        username: req.username,
    }))
}

async fn login(State(state): S, Json(req): Json<AuthReq>) -> Result<Json<AuthResp>, ApiError> {
    let conn = state.db.lock().unwrap();
    let row: Option<(i64, String)> = conn
        .query_row(
            "SELECT id, password_hash FROM users WHERE username = ?1",
            [&req.username],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(db_err)?;
    drop(conn);
    let (id, hash) = row.ok_or_else(|| ApiError::not_found("用户不存在"))?;
    if !auth::verify_password(&req.password, &hash) {
        return Err(ApiError::unauthorized("密码错误"));
    }
    let token = auth::issue_token(&state.jwt_secret, id, &req.username)?;
    Ok(Json(AuthResp {
        token,
        username: req.username,
    }))
}

async fn whoami(State(state): S, headers: HeaderMap) -> Result<Json<serde_json::Value>, ApiError> {
    let claims = auth::authenticate(&state.jwt_secret, &headers)?;
    let conn = state.db.lock().unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT g.id, g.name FROM user_groups ug JOIN groups g ON g.id = ug.group_id
             WHERE ug.user_id = ?1 ORDER BY g.name",
        )
        .map_err(db_err)?;
    let groups: Vec<serde_json::Value> = stmt
        .query_map([claims.sub], |r| {
            Ok(serde_json::json!({ "id": r.get::<_, i64>(0)?, "name": r.get::<_, String>(1)? }))
        })
        .map_err(db_err)?
        .filter_map(|r| r.ok())
        .collect();
    drop(stmt);
    drop(conn);
    Ok(Json(serde_json::json!({
        "username": claims.username,
        "user_id": claims.sub,
        "groups": groups,
    })))
}

#[derive(serde::Deserialize)]
struct ExploreQuery {
    q: Option<String>,
    /// 按受管标签名筛选（如「官方」「社区」）。
    label: Option<String>,
}

/// 公开市场：列出所有 visibility=public 的 MCP，可选 `?q=` 模糊匹配名称/清单、`?label=` 标签筛选。
/// 鉴权可选——匿名访客只看「所有分组可见」的；登录用户额外看到其分组可见的与自己的。
async fn explore(
    State(state): S,
    headers: HeaderMap,
    Query(query): Query<ExploreQuery>,
) -> Result<Json<Vec<McpInfo>>, ApiError> {
    let pattern = match query.q.as_deref().map(str::trim) {
        Some(s) if !s.is_empty() => format!("%{}%", s.to_lowercase()),
        _ => "%".to_string(),
    };
    let label_filter = query
        .label
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "all")
        .map(|s| s.to_string());
    let conn = state.db.lock().unwrap();
    let viewer = auth::authenticate_opt(&state.jwt_secret, &headers);
    let viewer_groups = viewer
        .as_ref()
        .map(|c| groups_of_user(&conn, c.sub))
        .unwrap_or_default();
    let viewer_name = viewer.as_ref().map(|c| c.username.clone());
    let label_map = all_resource_labels(&conn, "mcp_labels", "mcp_id");
    let mut stmt = conn
        .prepare(
            "SELECT m.id, u.username, m.name, m.visibility, m.version, m.manifest, m.tools,
                    m.updated_at, m.group_visibility
             FROM mcps m JOIN users u ON u.id = m.owner_id
             WHERE m.visibility = 'public'
               AND (lower(m.name) LIKE ?1 OR lower(m.manifest) LIKE ?1 OR lower(m.tools) LIKE ?1)
             ORDER BY m.updated_at DESC, m.name",
        )
        .map_err(db_err)?;
    let rows = stmt
        .query_map([pattern], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?,
                r.get::<_, String>(8)?,
            ))
        })
        .map_err(db_err)?;
    let mut out = Vec::new();
    for row in rows {
        let (id, owner, name, visibility, version, manifest_json, tools_json, updated_at, group_vis) =
            row.map_err(db_err)?;
        let is_owner = viewer_name.as_deref() == Some(owner.as_str());
        if !group_can_see(&group_vis, &viewer_groups, is_owner) {
            continue;
        }
        let labels = label_map.get(&id).cloned().unwrap_or_default();
        if let Some(want) = &label_filter {
            if !labels.iter().any(|l| l == want) {
                continue;
            }
        }
        let manifest: McpManifest = serde_json::from_str(&manifest_json)
            .map_err(|e| ApiError::internal(format!("manifest 解析失败: {e}")))?;
        out.push(McpInfo {
            owner,
            name,
            visibility,
            version,
            manifest,
            tools: parse_tools(&tools_json),
            labels,
            updated_at,
        });
    }
    Ok(Json(out))
}

// ---------------------------------------------------------------------------
// mcp 注册表
// ---------------------------------------------------------------------------

async fn mcp_upsert(
    State(state): S,
    headers: HeaderMap,
    Json(req): Json<McpUpsertReq>,
) -> Result<Json<McpInfo>, ApiError> {
    let claims = auth::authenticate(&state.jwt_secret, &headers)?;
    let manifest = req.manifest;
    if manifest.name.is_empty() {
        return Err(ApiError::bad_request("manifest.name 不能为空"));
    }
    match manifest.runtime {
        crate::shared::Runtime::Remote if manifest.url.is_none() => {
            return Err(ApiError::bad_request("remote 运行时必须提供 url"));
        }
        crate::shared::Runtime::Local if manifest.command.is_none() => {
            return Err(ApiError::bad_request("local 运行时必须提供 command"));
        }
        _ => {}
    }
    let visibility = match req.visibility.as_str() {
        "private" | "public" => req.visibility.clone(),
        _ => return Err(ApiError::bad_request("visibility 只能是 private 或 public")),
    };
    let manifest_json = serde_json::to_string(&manifest).map_err(|e| ApiError::internal(e.to_string()))?;
    let now = now_string();
    let conn = state.db.lock().unwrap();
    conn.execute(
        "INSERT INTO mcps(owner_id, name, visibility, version, manifest, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(owner_id, name)
         DO UPDATE SET visibility=excluded.visibility, version=excluded.version,
                       manifest=excluded.manifest, updated_at=excluded.updated_at",
        rusqlite::params![
            claims.sub,
            manifest.name,
            visibility,
            manifest.version,
            manifest_json,
            now
        ],
    )
    .map_err(db_err)?;
    // tools 不随 manifest 改动（插入默认 '[]'，更新时保留旧值），单独读出用于响应。
    let tools_json: String = conn
        .query_row(
            "SELECT tools FROM mcps WHERE owner_id = ?1 AND name = ?2",
            rusqlite::params![claims.sub, manifest.name],
            |r| r.get(0),
        )
        .map_err(db_err)?;
    drop(conn);
    Ok(Json(McpInfo {
        owner: claims.username,
        name: manifest.name.clone(),
        visibility,
        version: manifest.version.clone(),
        manifest,
        tools: parse_tools(&tools_json),
        labels: Vec::new(),
        updated_at: now,
    }))
}

async fn mcp_list(State(state): S, headers: HeaderMap) -> Result<Json<Vec<McpInfo>>, ApiError> {
    let claims = auth::authenticate(&state.jwt_secret, &headers)?;
    let conn = state.db.lock().unwrap();
    let label_map = all_resource_labels(&conn, "mcp_labels", "mcp_id");
    let mut stmt = conn
        .prepare(
            "SELECT id, name, visibility, version, manifest, tools, updated_at
             FROM mcps WHERE owner_id = ?1 ORDER BY name",
        )
        .map_err(db_err)?;
    let rows = stmt
        .query_map([claims.sub], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
            ))
        })
        .map_err(db_err)?;
    let mut out = Vec::new();
    for row in rows {
        let (id, name, visibility, version, manifest_json, tools_json, updated_at) =
            row.map_err(db_err)?;
        let manifest: McpManifest = serde_json::from_str(&manifest_json)
            .map_err(|e| ApiError::internal(format!("manifest 解析失败: {e}")))?;
        out.push(McpInfo {
            owner: claims.username.clone(),
            name,
            visibility,
            version,
            manifest,
            tools: parse_tools(&tools_json),
            labels: label_map.get(&id).cloned().unwrap_or_default(),
            updated_at,
        });
    }
    Ok(Json(out))
}

async fn mcp_delete(
    State(state): S,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> Result<StatusCode, ApiError> {
    let claims = auth::authenticate(&state.jwt_secret, &headers)?;
    let conn = state.db.lock().unwrap();
    let n = conn
        .execute(
            "DELETE FROM mcps WHERE owner_id = ?1 AND name = ?2",
            rusqlite::params![claims.sub, name],
        )
        .map_err(db_err)?;
    if n == 0 {
        return Err(ApiError::not_found("未找到该 MCP"));
    }
    Ok(StatusCode::NO_CONTENT)
}

/// 重命名 MCP（仅限本人）。同步更新 manifest.name，并校验新名未被占用。
async fn mcp_rename(
    State(state): S,
    headers: HeaderMap,
    Path(name): Path<String>,
    Json(req): Json<McpRenameReq>,
) -> Result<Json<McpInfo>, ApiError> {
    let claims = auth::authenticate(&state.jwt_secret, &headers)?;
    let new_name = req.new_name.trim().to_string();
    if new_name.is_empty() || new_name.contains('/') || new_name.len() > 128 {
        return Err(ApiError::bad_request("新名称非法（不能为空、含 '/'，且 ≤128 字符）"));
    }
    let now = now_string();
    let conn = state.db.lock().unwrap();
    let row: Option<(String, String, String, String)> = conn
        .query_row(
            "SELECT visibility, version, manifest, tools FROM mcps WHERE owner_id = ?1 AND name = ?2",
            rusqlite::params![claims.sub, name],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()
        .map_err(db_err)?;
    let (visibility, version, manifest_json, tools_json) =
        row.ok_or_else(|| ApiError::not_found("未找到该 MCP"))?;

    if new_name != name {
        let taken: bool = conn
            .query_row(
                "SELECT 1 FROM mcps WHERE owner_id = ?1 AND name = ?2",
                rusqlite::params![claims.sub, new_name],
                |_| Ok(true),
            )
            .optional()
            .map_err(db_err)?
            .unwrap_or(false);
        if taken {
            return Err(ApiError::conflict("已存在同名 MCP"));
        }
    }

    let mut manifest: McpManifest = serde_json::from_str(&manifest_json)
        .map_err(|e| ApiError::internal(format!("manifest 解析失败: {e}")))?;
    manifest.name = new_name.clone();
    let new_json =
        serde_json::to_string(&manifest).map_err(|e| ApiError::internal(e.to_string()))?;
    conn.execute(
        "UPDATE mcps SET name = ?1, manifest = ?2, updated_at = ?3
         WHERE owner_id = ?4 AND name = ?5",
        rusqlite::params![new_name, new_json, now, claims.sub, name],
    )
    .map_err(db_err)?;
    drop(conn);

    Ok(Json(McpInfo {
        owner: claims.username,
        name: new_name,
        visibility,
        version,
        manifest,
        tools: parse_tools(&tools_json),
        labels: Vec::new(),
        updated_at: now,
    }))
}

/// 上报某 MCP 的工具清单（仅限本人）。客户端连接 MCP 列出工具后写入检索索引。
async fn mcp_set_tools(
    State(state): S,
    headers: HeaderMap,
    Path(name): Path<String>,
    Json(req): Json<SetToolsReq>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let claims = auth::authenticate(&state.jwt_secret, &headers)?;
    let tools_json =
        serde_json::to_string(&req.tools).map_err(|e| ApiError::internal(e.to_string()))?;
    let now = now_string();
    let conn = state.db.lock().unwrap();
    let n = conn
        .execute(
            "UPDATE mcps SET tools = ?1, updated_at = ?2 WHERE owner_id = ?3 AND name = ?4",
            rusqlite::params![tools_json, now, claims.sub, name],
        )
        .map_err(db_err)?;
    if n == 0 {
        return Err(ApiError::not_found("未找到该 MCP"));
    }
    Ok(Json(serde_json::json!({ "indexed": req.tools.len() })))
}

async fn mcp_get(
    State(state): S,
    headers: HeaderMap,
    Path((owner, name)): Path<(String, String)>,
) -> Result<Json<McpInfo>, ApiError> {
    let claims = auth::authenticate(&state.jwt_secret, &headers)?;
    let (info, group_vis) = load_mcp(&state, &owner, &name)?;
    ensure_mcp_access(&state, &info, &group_vis, &claims)?;
    Ok(Json(info))
}

// ---------------------------------------------------------------------------
// 加密凭据池
// ---------------------------------------------------------------------------

async fn secret_set(
    State(state): S,
    headers: HeaderMap,
    Json(req): Json<SecretSetReq>,
) -> Result<Json<SecretInfo>, ApiError> {
    let claims = auth::authenticate(&state.jwt_secret, &headers)?;
    if req.key.is_empty() {
        return Err(ApiError::bad_request("变量名不能为空"));
    }
    let (nonce, ct) = crypto::encrypt(&state.master_key, &req.value)?;
    let now = now_string();
    let conn = state.db.lock().unwrap();
    conn.execute(
        "INSERT INTO secrets(owner_id, key, nonce, ciphertext, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(owner_id, key)
         DO UPDATE SET nonce=excluded.nonce, ciphertext=excluded.ciphertext,
                       updated_at=excluded.updated_at",
        rusqlite::params![claims.sub, req.key, nonce, ct, now],
    )
    .map_err(db_err)?;
    drop(conn);
    Ok(Json(SecretInfo {
        key: req.key,
        updated_at: now,
    }))
}

async fn secret_list(
    State(state): S,
    headers: HeaderMap,
) -> Result<Json<Vec<SecretInfo>>, ApiError> {
    let claims = auth::authenticate(&state.jwt_secret, &headers)?;
    let conn = state.db.lock().unwrap();
    let mut stmt = conn
        .prepare("SELECT key, updated_at FROM secrets WHERE owner_id = ?1 ORDER BY key")
        .map_err(db_err)?;
    let rows = stmt
        .query_map([claims.sub], |r| {
            Ok(SecretInfo {
                key: r.get(0)?,
                updated_at: r.get(1)?,
            })
        })
        .map_err(db_err)?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(db_err)?);
    }
    Ok(Json(out))
}

async fn secret_delete(
    State(state): S,
    headers: HeaderMap,
    Path(key): Path<String>,
) -> Result<StatusCode, ApiError> {
    let claims = auth::authenticate(&state.jwt_secret, &headers)?;
    let conn = state.db.lock().unwrap();
    let n = conn
        .execute(
            "DELETE FROM secrets WHERE owner_id = ?1 AND key = ?2",
            rusqlite::params![claims.sub, key],
        )
        .map_err(db_err)?;
    if n == 0 {
        return Err(ApiError::not_found("未找到该变量"));
    }
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// run 解析：凭据缝合
// ---------------------------------------------------------------------------

async fn run_resolve(
    State(state): S,
    headers: HeaderMap,
    Path((owner, name)): Path<(String, String)>,
) -> Result<Json<ResolveResp>, ApiError> {
    // 鉴权可选：未登录也能解析公开 MCP（返回原始 manifest，线上变量为空）。
    let viewer = auth::authenticate_opt(&state.jwt_secret, &headers);
    let (info, group_vis) = load_mcp(&state, &owner, &name)?;
    let is_owner = viewer.as_ref().map(|c| c.username == owner).unwrap_or(false);
    if !is_owner {
        if info.visibility != "public" {
            return Err(ApiError::not_found("未找到该 MCP（或为私有）"));
        }
        let viewer_groups = viewer
            .as_ref()
            .map(|c| {
                let conn = state.db.lock().unwrap();
                groups_of_user(&conn, c.sub)
            })
            .unwrap_or_default();
        if !group_can_see(&group_vis, &viewer_groups, false) {
            return Err(ApiError::not_found("未找到该 MCP（或所属分组不可见）"));
        }
    }
    let required = info.manifest.required_vars();
    // 仅返回调用者本人线上已设置、且被该 manifest 引用的变量值。
    let vars = if let Some(claims) = &viewer {
        let all = decrypt_user_secrets(&state, claims.sub)?;
        required
            .iter()
            .filter_map(|k| all.get(k).map(|v| (k.clone(), v.clone())))
            .collect()
    } else {
        std::collections::BTreeMap::new()
    };
    Ok(Json(ResolveResp {
        manifest: info.manifest,
        required,
        vars,
    }))
}

/// Hub 作为网关代调用某工具：解析凭据 → 服务端连接 MCP → tools/call → 回吐结果。
async fn run_call(
    State(state): S,
    headers: HeaderMap,
    Path((owner, name)): Path<(String, String)>,
    Json(req): Json<CallReq>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let claims = auth::authenticate(&state.jwt_secret, &headers)?;
    let (info, group_vis) = load_mcp(&state, &owner, &name)?;
    ensure_mcp_access(&state, &info, &group_vis, &claims)?;
    if req.tool.is_empty() {
        return Err(ApiError::bad_request("缺少 tool 字段"));
    }
    let (resolved, _required, missing) = stitch_for_user(&state, claims.sub, &info.manifest)?;
    if !missing.is_empty() {
        return Err(ApiError::bad_request(format!(
            "缺少变量：{}。请在「我的变量」设置后重试",
            missing.join(", ")
        )));
    }
    let tool = req.tool.clone();
    let call_tool = tool.clone();
    let arguments = if req.arguments.is_null() {
        serde_json::json!({})
    } else {
        req.arguments
    };
    // MCP 连接是阻塞 IO（子进程 / 阻塞 HTTP），放到 blocking 线程。
    let started = std::time::Instant::now();
    let outcome = tokio::task::spawn_blocking(move || -> anyhow::Result<serde_json::Value> {
        let mut mcp = crate::mcp::McpClient::connect(&resolved)?;
        mcp.call_tool(&call_tool, arguments)
    })
    .await
    .map_err(|e| ApiError::internal(format!("任务执行失败: {e}")))?;
    let ms = started.elapsed().as_millis() as i64;
    match outcome {
        Ok(result) => {
            let summary = summarize_result(&result);
            log_tool_call(&state, &claims.username, &owner, &name, &tool, true, "", &summary, ms);
            Ok(Json(result))
        }
        Err(e) => {
            let msg = format!("MCP 调用失败: {e}");
            log_tool_call(&state, &claims.username, &owner, &name, &tool, false, &msg, "", ms);
            Err(ApiError::new(StatusCode::BAD_GATEWAY, msg))
        }
    }
}

/// CLI 调用回传：`tsk run` 在本地直连 MCP 完成 `tools/call` 后，把调用结果上报 Hub
/// 作审计统计（尽力而为）。仅记录元信息（工具名、成败、耗时），不经手任何凭据或参数。
async fn run_report(
    State(state): S,
    headers: HeaderMap,
    Path((owner, name)): Path<(String, String)>,
    Json(req): Json<ReportCallReq>,
) -> Result<StatusCode, ApiError> {
    let claims = auth::authenticate(&state.jwt_secret, &headers)?;
    if req.tool.is_empty() {
        return Err(ApiError::bad_request("缺少 tool 字段"));
    }
    // 校验该 MCP 确实存在且调用者可见，避免被用来伪造任意审计行。
    let (info, group_vis) = load_mcp(&state, &owner, &name)?;
    ensure_mcp_access(&state, &info, &group_vis, &claims)?;
    log_tool_call(
        &state,
        &claims.username,
        &owner,
        &name,
        &req.tool,
        req.ok,
        &req.error,
        &req.summary,
        req.ms.max(0),
    );
    Ok(StatusCode::NO_CONTENT)
}

/// 取用户名下凭据并缝合进清单，返回 (resolved, required, missing)。
/// 解密某用户名下全部线上变量为明文 map（变量名→值）。
fn decrypt_user_secrets(
    state: &AppState,
    user_id: i64,
) -> Result<std::collections::BTreeMap<String, String>, ApiError> {
    let conn = state.db.lock().unwrap();
    let mut stmt = conn
        .prepare("SELECT key, nonce, ciphertext FROM secrets WHERE owner_id = ?1")
        .map_err(db_err)?;
    let rows = stmt
        .query_map([user_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Vec<u8>>(1)?,
                r.get::<_, Vec<u8>>(2)?,
            ))
        })
        .map_err(db_err)?;
    let mut vars = std::collections::BTreeMap::new();
    for row in rows {
        let (key, nonce, ct) = row.map_err(db_err)?;
        let val = crypto::decrypt(&state.master_key, &nonce, &ct)?;
        vars.insert(key, val);
    }
    Ok(vars)
}

fn stitch_for_user(
    state: &AppState,
    user_id: i64,
    manifest: &McpManifest,
) -> Result<(McpManifest, Vec<String>, Vec<String>), ApiError> {
    let vars = decrypt_user_secrets(state, user_id)?;
    let required = manifest.required_vars();
    let (resolved, missing) = stitch(manifest, &vars);
    Ok((resolved, required, missing))
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn load_mcp(state: &AppState, owner: &str, name: &str) -> Result<(McpInfo, String), ApiError> {
    let conn = state.db.lock().unwrap();
    let row: Option<(i64, String, String, String, String, String, String)> = conn
        .query_row(
            "SELECT m.id, m.visibility, m.version, m.manifest, m.tools, m.updated_at, m.group_visibility
             FROM mcps m JOIN users u ON u.id = m.owner_id
             WHERE u.username = ?1 AND m.name = ?2",
            rusqlite::params![owner, name],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)),
        )
        .optional()
        .map_err(db_err)?;
    let (id, visibility, version, manifest_json, tools_json, updated_at, group_vis) =
        row.ok_or_else(|| ApiError::not_found(format!("未找到 MCP: {owner}/{name}")))?;
    let labels = labels_of(&conn, "mcp_labels", "mcp_id", id);
    drop(conn);
    let manifest: McpManifest = serde_json::from_str(&manifest_json)
        .map_err(|e| ApiError::internal(format!("manifest 解析失败: {e}")))?;
    Ok((
        McpInfo {
            owner: owner.to_string(),
            name: name.to_string(),
            visibility,
            version,
            manifest,
            tools: parse_tools(&tools_json),
            labels,
            updated_at,
        },
        group_vis,
    ))
}

/// 非 owner 访问公开 MCP 时的分组可见性校验：不可见时统一报 not_found（避免泄漏存在性）。
fn ensure_mcp_access(
    state: &AppState,
    info: &McpInfo,
    group_vis: &str,
    claims: &auth::Claims,
) -> Result<(), ApiError> {
    if info.owner == claims.username {
        return Ok(());
    }
    if info.visibility != "public" {
        return Err(ApiError::not_found("未找到该 MCP（或为私有）"));
    }
    let viewer_groups = {
        let conn = state.db.lock().unwrap();
        groups_of_user(&conn, claims.sub)
    };
    if !group_can_see(group_vis, &viewer_groups, false) {
        return Err(ApiError::not_found("未找到该 MCP（或所属分组不可见）"));
    }
    Ok(())
}

/// 解析存储的工具索引 JSON，损坏则当作空。
fn parse_tools(s: &str) -> Vec<ToolMeta> {
    serde_json::from_str(s).unwrap_or_default()
}

/// 分组可见性判定：owner 永远可见；'all'（或空）对所有人可见；否则只要访客所属分组
/// 与白名单有交集即可见。匿名访客（viewer_groups 为空）只能看到 'all' 资源。
pub(super) fn group_can_see(group_visibility: &str, viewer_groups: &[i64], is_owner: bool) -> bool {
    if is_owner {
        return true;
    }
    let gv = group_visibility.trim();
    if gv.is_empty() || gv == "all" {
        return true;
    }
    let allowed: Vec<i64> = serde_json::from_str(gv).unwrap_or_default();
    viewer_groups.iter().any(|g| allowed.contains(g))
}

/// 查询某用户所属的全部分组 id（多对多）。
pub(super) fn groups_of_user(conn: &rusqlite::Connection, user_id: i64) -> Vec<i64> {
    let mut stmt = match conn.prepare("SELECT group_id FROM user_groups WHERE user_id = ?1") {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let rows = match stmt.query_map([user_id], |r| r.get::<_, i64>(0)) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    rows.filter_map(|r| r.ok()).collect()
}

/// 取某资源（skill / mcp）已分配的受管标签名（按名排序）。
/// `junction`/`fk` 为内部常量（skill_labels/skill_id 等），非用户输入，无注入风险。
pub(super) fn labels_of(conn: &rusqlite::Connection, junction: &str, fk: &str, id: i64) -> Vec<String> {
    let sql = format!(
        "SELECT l.name FROM {junction} j JOIN labels l ON l.id = j.label_id
         WHERE j.{fk} = ?1 ORDER BY l.name"
    );
    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let rows = match stmt.query_map([id], |r| r.get::<_, String>(0)) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    rows.filter_map(|r| r.ok()).collect()
}

/// 全量映射：资源 id → 受管标签名列表。供列表接口一次性装配，避免 N+1。
pub(super) fn all_resource_labels(
    conn: &rusqlite::Connection,
    junction: &str,
    fk: &str,
) -> std::collections::HashMap<i64, Vec<String>> {
    let mut map: std::collections::HashMap<i64, Vec<String>> = std::collections::HashMap::new();
    let sql = format!(
        "SELECT j.{fk}, l.name FROM {junction} j JOIN labels l ON l.id = j.label_id ORDER BY l.name"
    );
    if let Ok(mut stmt) = conn.prepare(&sql) {
        if let Ok(rows) = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))) {
            for (id, name) in rows.flatten() {
                map.entry(id).or_default().push(name);
            }
        }
    }
    map
}

pub(super) fn db_err(e: rusqlite::Error) -> ApiError {
    eprintln!("db error: {e}");
    ApiError::internal(format!("数据库错误: {e}"))
}

/// 当前 Unix 时间戳（秒）。用于审计表的时间窗过滤（24h 等）。
pub(super) fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// 记录一次工具调用审计（尽力而为，落库失败不影响调用结果）。
pub(super) fn log_tool_call(
    state: &AppState,
    caller: &str,
    owner: &str,
    mcp_name: &str,
    tool: &str,
    ok: bool,
    error: &str,
    result: &str,
    ms: i64,
) {
    let conn = state.db.lock().unwrap();
    let _ = conn.execute(
        "INSERT INTO tool_calls(caller, owner, mcp_name, tool, ok, error, result, ms, created_at, created_ts)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        rusqlite::params![
            caller,
            owner,
            mcp_name,
            tool,
            ok as i64,
            error,
            result,
            ms,
            now_string(),
            now_unix()
        ],
    );
}

/// 把一次 MCP 工具调用结果压缩成一行可读摘要，供审计面板「结果摘要」列展示。
/// 优先拼接 `content[].text`，否则回退到紧凑 JSON；统一截断到 240 字符。
pub(super) fn summarize_result(result: &serde_json::Value) -> String {
    let mut s = String::new();
    if let Some(items) = result.get("content").and_then(|c| c.as_array()) {
        for item in items {
            if let Some(t) = item.get("text").and_then(|t| t.as_str()) {
                if !s.is_empty() {
                    s.push(' ');
                }
                s.push_str(t);
            }
        }
    }
    if s.trim().is_empty() {
        s = result.to_string();
    }
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = s.trim();
    if trimmed.chars().count() <= 240 {
        trimmed.to_string()
    } else {
        let mut out: String = trimmed.chars().take(240).collect();
        out.push('…');
        out
    }
}

/// 当前时间 `YYYY-MM-DD HH:MM:SS UTC`（不引入 chrono）。
pub(super) fn now_string() -> String {
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
