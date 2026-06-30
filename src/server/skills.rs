//! 技能市场（Skill marketplace）的路由与处理器。
//!
//! 万物皆 Skill：`category` 仅是逻辑分类标签（skill / kb / toolchain）。服务端只
//! 持有元数据 + SKILL.md 文本（「基础信息」），庞大的数据体由 `tsk build` 打包成
//! tar.gz 压缩体，按 sha256 内容寻址落盘于 `blobs/`，记录里仅存 sha256 与字节数。

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use rusqlite::OptionalExtension;
use sha2::{Digest, Sha256};

use crate::shared::{SkillInfo, SkillRenameReq, SkillUpsertReq, SKILL_CATEGORIES};

use super::auth;
use super::error::ApiError;
use super::routes::{db_err, now_string};
use super::AppState;

type S = State<Arc<AppState>>;

/// 元数据列里 JSON 承载的扩展字段（mcp 依赖、倾向工具）。
#[derive(serde::Serialize, serde::Deserialize, Default)]
struct SkillMeta {
    #[serde(default)]
    mcp_dependencies: Vec<String>,
    #[serde(default)]
    preferred_tools: Vec<String>,
}

#[derive(serde::Deserialize)]
pub struct ExploreQuery {
    q: Option<String>,
    category: Option<String>,
    tag: Option<String>,
}

/// 公开技能市场：列出所有 public 技能，可按 `q`（名称/描述/SKILL.md/标签模糊）、
/// `category`、`tag` 过滤。无需鉴权。
pub async fn explore(
    State(state): S,
    Query(query): Query<ExploreQuery>,
) -> Result<Json<Vec<SkillInfo>>, ApiError> {
    let pattern = match query.q.as_deref().map(str::trim) {
        Some(s) if !s.is_empty() => format!("%{}%", s.to_lowercase()),
        _ => "%".to_string(),
    };
    let category = query
        .category
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "all")
        .map(|s| s.to_string());
    let tag = query
        .tag
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| format!("%\"{}\"%", s.to_lowercase()));

    let conn = state.db.lock().unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT u.username, s.name, s.category, s.visibility, s.version, s.description,
                    s.tags, s.skill_md, s.metadata, s.archive_sha256, s.archive_size, s.updated_at
             FROM skills s JOIN users u ON u.id = s.owner_id
             WHERE s.visibility = 'public'
               AND (lower(s.name) LIKE ?1 OR lower(s.description) LIKE ?1
                    OR lower(s.skill_md) LIKE ?1 OR lower(s.tags) LIKE ?1)
               AND (?2 IS NULL OR s.category = ?2)
               AND (?3 IS NULL OR lower(s.tags) LIKE ?3)
             ORDER BY s.updated_at DESC, s.name",
        )
        .map_err(db_err)?;
    let rows = stmt
        .query_map(rusqlite::params![pattern, category, tag], row_to_tuple)
        .map_err(db_err)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(tuple_to_info(row.map_err(db_err)?)?);
    }
    Ok(Json(out))
}

/// 列出当前用户名下全部技能（含私有）。
pub async fn list_mine(State(state): S, headers: HeaderMap) -> Result<Json<Vec<SkillInfo>>, ApiError> {
    let claims = auth::authenticate(&state.jwt_secret, &headers)?;
    let conn = state.db.lock().unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT ?1, s.name, s.category, s.visibility, s.version, s.description,
                    s.tags, s.skill_md, s.metadata, s.archive_sha256, s.archive_size, s.updated_at
             FROM skills s WHERE s.owner_id = ?2 ORDER BY s.updated_at DESC, s.name",
        )
        .map_err(db_err)?;
    let rows = stmt
        .query_map(rusqlite::params![claims.username, claims.sub], row_to_tuple)
        .map_err(db_err)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(tuple_to_info(row.map_err(db_err)?)?);
    }
    Ok(Json(out))
}

/// 发布/更新一个技能的元数据（压缩体走 archive 接口单独上传）。
pub async fn upsert(
    State(state): S,
    headers: HeaderMap,
    Json(req): Json<SkillUpsertReq>,
) -> Result<Json<SkillInfo>, ApiError> {
    let claims = auth::authenticate(&state.jwt_secret, &headers)?;
    let m = req.manifest;
    if !valid_name(&m.name) {
        return Err(ApiError::bad_request(
            "技能名仅允许字母、数字、_、-、.，且长度 1..=128",
        ));
    }
    let category = if SKILL_CATEGORIES.contains(&m.category.as_str()) {
        m.category.clone()
    } else {
        return Err(ApiError::bad_request(format!(
            "category 只能是 {}",
            SKILL_CATEGORIES.join(" / ")
        )));
    };
    let visibility = match req.visibility.as_str() {
        "private" | "public" => req.visibility.clone(),
        _ => return Err(ApiError::bad_request("visibility 只能是 private 或 public")),
    };
    let tags_json = serde_json::to_string(&lower_tags(&m.tags))
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let meta = SkillMeta {
        mcp_dependencies: m.mcp_dependencies.clone(),
        preferred_tools: m.preferred_tools.clone(),
    };
    let meta_json = serde_json::to_string(&meta).map_err(|e| ApiError::internal(e.to_string()))?;
    let now = now_string();

    let conn = state.db.lock().unwrap();
    // 保留既有压缩体信息（upsert 元数据不应清空已上传的数据体）。
    let prev: Option<(String, i64)> = conn
        .query_row(
            "SELECT archive_sha256, archive_size FROM skills WHERE owner_id = ?1 AND name = ?2",
            rusqlite::params![claims.sub, m.name],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(db_err)?;
    let (archive_sha256, archive_size) = if req.archive_sha256.is_empty() {
        prev.unwrap_or_default()
    } else {
        (req.archive_sha256.clone(), req.archive_size as i64)
    };

    conn.execute(
        "INSERT INTO skills(owner_id, name, category, visibility, version, description,
                            tags, skill_md, metadata, archive_sha256, archive_size, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
         ON CONFLICT(owner_id, name) DO UPDATE SET
             category=excluded.category, visibility=excluded.visibility, version=excluded.version,
             description=excluded.description, tags=excluded.tags, skill_md=excluded.skill_md,
             metadata=excluded.metadata, archive_sha256=excluded.archive_sha256,
             archive_size=excluded.archive_size, updated_at=excluded.updated_at",
        rusqlite::params![
            claims.sub,
            m.name,
            category,
            visibility,
            m.version,
            m.description,
            tags_json,
            req.skill_md,
            meta_json,
            archive_sha256,
            archive_size,
            now
        ],
    )
    .map_err(db_err)?;
    drop(conn);

    Ok(Json(SkillInfo {
        owner: claims.username,
        name: m.name,
        category,
        visibility,
        version: m.version,
        description: m.description,
        tags: lower_tags(&m.tags),
        mcp_dependencies: m.mcp_dependencies,
        preferred_tools: m.preferred_tools,
        skill_md: req.skill_md,
        archive_sha256,
        archive_size: archive_size as u64,
        updated_at: now,
    }))
}

/// 技能详情。public 任何人可读；private 仅 owner（带有效 token）可读。
pub async fn get(
    State(state): S,
    headers: HeaderMap,
    Path((owner, name)): Path<(String, String)>,
) -> Result<Json<SkillInfo>, ApiError> {
    let info = load_skill(&state, &owner, &name)?;
    if info.visibility != "public" {
        let claims = auth::authenticate(&state.jwt_secret, &headers)?;
        if claims.username != owner {
            return Err(ApiError::not_found("未找到该技能（或为私有）"));
        }
    }
    Ok(Json(info))
}

pub async fn delete(
    State(state): S,
    headers: HeaderMap,
    Path((owner, name)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let claims = auth::authenticate(&state.jwt_secret, &headers)?;
    if claims.username != owner {
        return Err(ApiError::unauthorized("只能删除自己的技能"));
    }
    let conn = state.db.lock().unwrap();
    let sha: Option<String> = conn
        .query_row(
            "SELECT archive_sha256 FROM skills WHERE owner_id = ?1 AND name = ?2",
            rusqlite::params![claims.sub, name],
            |r| r.get(0),
        )
        .optional()
        .map_err(db_err)?;
    let n = conn
        .execute(
            "DELETE FROM skills WHERE owner_id = ?1 AND name = ?2",
            rusqlite::params![claims.sub, name],
        )
        .map_err(db_err)?;
    if n == 0 {
        return Err(ApiError::not_found("未找到该技能"));
    }
    // 若该压缩体已无人引用，顺带清理 blob（尽力而为）。
    if let Some(sha) = sha.filter(|s| !s.is_empty()) {
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
            let _ = std::fs::remove_file(blob_path(&state, &sha));
        }
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn rename(
    State(state): S,
    headers: HeaderMap,
    Path((owner, name)): Path<(String, String)>,
    Json(req): Json<SkillRenameReq>,
) -> Result<Json<SkillInfo>, ApiError> {
    let claims = auth::authenticate(&state.jwt_secret, &headers)?;
    if claims.username != owner {
        return Err(ApiError::unauthorized("只能重命名自己的技能"));
    }
    let new_name = req.new_name.trim().to_string();
    if !valid_name(&new_name) {
        return Err(ApiError::bad_request("新名称非法"));
    }
    let now = now_string();
    let conn = state.db.lock().unwrap();
    if new_name != name {
        let taken: bool = conn
            .query_row(
                "SELECT 1 FROM skills WHERE owner_id = ?1 AND name = ?2",
                rusqlite::params![claims.sub, new_name],
                |_| Ok(true),
            )
            .optional()
            .map_err(db_err)?
            .unwrap_or(false);
        if taken {
            return Err(ApiError::conflict("已存在同名技能"));
        }
    }
    let n = conn
        .execute(
            "UPDATE skills SET name = ?1, updated_at = ?2 WHERE owner_id = ?3 AND name = ?4",
            rusqlite::params![new_name, now, claims.sub, name],
        )
        .map_err(db_err)?;
    if n == 0 {
        return Err(ApiError::not_found("未找到该技能"));
    }
    drop(conn);
    Ok(Json(load_skill(&state, &owner, &new_name)?))
}

/// 上传技能压缩体（tar.gz 原始字节）。仅 owner。服务端计算 sha256 落盘并写回记录。
pub async fn archive_put(
    State(state): S,
    headers: HeaderMap,
    Path((owner, name)): Path<(String, String)>,
    body: Bytes,
) -> Result<Json<serde_json::Value>, ApiError> {
    let claims = auth::authenticate(&state.jwt_secret, &headers)?;
    if claims.username != owner {
        return Err(ApiError::unauthorized("只能上传自己的技能压缩体"));
    }
    if body.is_empty() {
        return Err(ApiError::bad_request("压缩体为空"));
    }
    let sha = sha256_hex(&body);
    let size = body.len() as i64;
    // 内容寻址落盘（若已存在同内容则复用）。
    let path = blob_path(&state, &sha);
    if !path.exists() {
        std::fs::write(&path, &body)
            .map_err(|e| ApiError::internal(format!("写入压缩体失败: {e}")))?;
    }
    let now = now_string();
    let conn = state.db.lock().unwrap();
    let n = conn
        .execute(
            "UPDATE skills SET archive_sha256 = ?1, archive_size = ?2, updated_at = ?3
             WHERE owner_id = ?4 AND name = ?5",
            rusqlite::params![sha, size, now, claims.sub, name],
        )
        .map_err(db_err)?;
    if n == 0 {
        return Err(ApiError::not_found("请先发布技能元数据，再上传压缩体"));
    }
    Ok(Json(serde_json::json!({
        "archive_sha256": sha,
        "archive_size": size,
    })))
}

/// 下载技能压缩体。public 任何人可下；private 仅 owner。
pub async fn archive_get(
    State(state): S,
    headers: HeaderMap,
    Path((owner, name)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let info = load_skill(&state, &owner, &name)?;
    if info.visibility != "public" {
        let claims = auth::authenticate(&state.jwt_secret, &headers)?;
        if claims.username != owner {
            return Err(ApiError::not_found("未找到该技能（或为私有）"));
        }
    }
    if info.archive_sha256.is_empty() {
        return Err(ApiError::not_found("该技能没有压缩体（纯文本裸说明书包）"));
    }
    let path = blob_path(&state, &info.archive_sha256);
    let bytes = std::fs::read(&path)
        .map_err(|e| ApiError::internal(format!("读取压缩体失败: {e}")))?;
    let filename = format!("{}-{}.tar.gz", name, info.version);
    Ok((
        [
            (header::CONTENT_TYPE, "application/gzip".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{filename}\""),
            ),
        ],
        bytes,
    )
        .into_response())
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn valid_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

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

fn blob_path(state: &AppState, sha: &str) -> std::path::PathBuf {
    state.blobs_dir.join(format!("{sha}.tar.gz"))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut s = String::with_capacity(64);
    for b in digest {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

type SkillRow = (
    String, // owner
    String, // name
    String, // category
    String, // visibility
    String, // version
    String, // description
    String, // tags json
    String, // skill_md
    String, // metadata json
    String, // archive_sha256
    i64,    // archive_size
    String, // updated_at
);

fn row_to_tuple(r: &rusqlite::Row<'_>) -> rusqlite::Result<SkillRow> {
    Ok((
        r.get(0)?,
        r.get(1)?,
        r.get(2)?,
        r.get(3)?,
        r.get(4)?,
        r.get(5)?,
        r.get(6)?,
        r.get(7)?,
        r.get(8)?,
        r.get(9)?,
        r.get(10)?,
        r.get(11)?,
    ))
}

fn tuple_to_info(t: SkillRow) -> Result<SkillInfo, ApiError> {
    let (owner, name, category, visibility, version, description, tags_json, skill_md, meta_json, sha, size, updated_at) =
        t;
    let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();
    let meta: SkillMeta = serde_json::from_str(&meta_json).unwrap_or_default();
    Ok(SkillInfo {
        owner,
        name,
        category,
        visibility,
        version,
        description,
        tags,
        mcp_dependencies: meta.mcp_dependencies,
        preferred_tools: meta.preferred_tools,
        skill_md,
        archive_sha256: sha,
        archive_size: size as u64,
        updated_at,
    })
}

fn load_skill(state: &AppState, owner: &str, name: &str) -> Result<SkillInfo, ApiError> {
    let conn = state.db.lock().unwrap();
    let row: Option<SkillRow> = conn
        .query_row(
            "SELECT u.username, s.name, s.category, s.visibility, s.version, s.description,
                    s.tags, s.skill_md, s.metadata, s.archive_sha256, s.archive_size, s.updated_at
             FROM skills s JOIN users u ON u.id = s.owner_id
             WHERE u.username = ?1 AND s.name = ?2",
            rusqlite::params![owner, name],
            row_to_tuple,
        )
        .optional()
        .map_err(db_err)?;
    drop(conn);
    let row = row.ok_or_else(|| ApiError::not_found(format!("未找到技能: {owner}/{name}")))?;
    tuple_to_info(row)
}
