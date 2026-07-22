//! 技能市场（Skill marketplace）的路由与处理器。
//!
//! 万物皆 Skill：`category` 仅是逻辑分类标签（skill / kb / toolchain）。服务端只
//! 持有元数据 + SKILL.md 文本（「基础信息」），庞大的数据体由 `tsk build` 打包成
//! tar.zst 压缩体（zstd，向后兼容旧版 gzip），按 sha256 内容寻址落盘于 `blobs/`，记录里仅存 sha256 与字节数。

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use sha2::{Digest, Sha256};

use crate::shared::{
    compare_versions, ReactReq, ReactResp, SkillInfo, SkillInspectResp, SkillRenameReq,
    SkillUpsertReq, SkillVersionInfo, TransferReq, SKILL_CATEGORIES,
};

use super::auth;
use super::db::{db_params, Db};
use super::error::ApiError;
use super::routes::now_string;
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
    /// 按受管标签名筛选（如「官方」「社区」）。
    label: Option<String>,
}

/// 公开技能市场：列出所有 public 技能，可按 `q`（名称/描述/SKILL.md/标签模糊）、
/// `category`、`tag`（自由标签）、`label`（受管标签）过滤。鉴权可选——匿名访客只看
/// 「所有分组可见」的，登录用户额外看到其分组可见的与自己的。
pub async fn explore(
    State(state): S,
    headers: HeaderMap,
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
    let label_filter = query
        .label
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "all")
        .map(|s| s.to_string());

    let viewer = auth::require_user_opt(&state, &headers).await;
    let viewer_groups = match viewer.as_ref() {
        Some(c) => super::routes::groups_of_user(&state.db, c.sub).await,
        None => Vec::new(),
    };
    let viewer_name = viewer.as_ref().map(|c| c.username.clone());
    let label_map = super::routes::all_resource_labels(&state.db, "skill_labels", "skill_id").await;
    let count_map = super::routes::all_reaction_counts(&state.db, "skill_reactions", "skill_id").await;
    let mine_map = match viewer.as_ref() {
        Some(c) => {
            super::routes::user_reaction_map(&state.db, "skill_reactions", "skill_id", c.sub).await
        }
        None => Default::default(),
    };
    let rows = state
        .db
        .query_map(
            "SELECT u.username, s.name, s.category, s.visibility, s.version, s.description,
                    s.tags, s.skill_md, s.metadata, s.archive_sha256, s.archive_size, s.updated_at,
                    s.downloads, s.group_visibility, s.id
             FROM skills s JOIN users u ON u.id = s.owner_id
             WHERE s.visibility = 'public'
               AND (lower(s.name) LIKE ?1 OR lower(s.description) LIKE ?1
                    OR lower(s.skill_md) LIKE ?1 OR lower(s.tags) LIKE ?1)
               AND (?2 IS NULL OR s.category = ?2)
               AND (?3 IS NULL OR lower(s.tags) LIKE ?3)
             ORDER BY s.updated_at DESC, s.name",
            db_params![pattern, category, tag],
            |r| Ok((row_to_tuple(r)?, r.get::<String>(13)?, r.get::<i64>(14)?)),
        )
        .await?;
    let mut out = Vec::new();
    for (tuple, group_vis, id) in rows {
        let is_owner = viewer_name.as_deref() == Some(tuple.0.as_str());
        if !super::routes::group_can_see(&group_vis, &viewer_groups, is_owner) {
            continue;
        }
        let labels = label_map.get(&id).cloned().unwrap_or_default();
        if let Some(want) = &label_filter {
            if !labels.iter().any(|l| l == want) {
                continue;
            }
        }
        let mut info = tuple_to_info(tuple)?;
        info.labels = labels;
        (info.likes, info.favorites) = count_map.get(&id).copied().unwrap_or_default();
        (info.liked, info.favorited) = mine_map.get(&id).copied().unwrap_or_default();
        out.push(info);
    }
    Ok(Json(out))
}

/// 列出当前用户名下全部技能（含私有）。
pub async fn list_mine(State(state): S, headers: HeaderMap) -> Result<Json<Vec<SkillInfo>>, ApiError> {
    let claims = auth::require_user(&state, &headers).await?;
    let label_map = super::routes::all_resource_labels(&state.db, "skill_labels", "skill_id").await;
    let count_map = super::routes::all_reaction_counts(&state.db, "skill_reactions", "skill_id").await;
    let mine_map =
        super::routes::user_reaction_map(&state.db, "skill_reactions", "skill_id", claims.sub).await;
    let rows = state
        .db
        .query_map(
            "SELECT CAST(?1 AS TEXT), s.name, s.category, s.visibility, s.version, s.description,
                    s.tags, s.skill_md, s.metadata, s.archive_sha256, s.archive_size, s.updated_at,
                    s.downloads, s.id
             FROM skills s WHERE s.owner_id = ?2 ORDER BY s.updated_at DESC, s.name",
            db_params![claims.username, claims.sub],
            |r| Ok((row_to_tuple(r)?, r.get::<i64>(13)?)),
        )
        .await?;
    let mut out = Vec::new();
    for (tuple, id) in rows {
        let mut info = tuple_to_info(tuple)?;
        info.labels = label_map.get(&id).cloned().unwrap_or_default();
        (info.likes, info.favorites) = count_map.get(&id).copied().unwrap_or_default();
        (info.liked, info.favorited) = mine_map.get(&id).copied().unwrap_or_default();
        out.push(info);
    }
    Ok(Json(out))
}

/// 发布/更新一个技能的元数据（压缩体走 archive 接口单独上传）。
///
/// 版本语义：每次发布都会把该版本的完整副本（说明书 + 元数据 + 压缩体指针）写入
/// `skill_versions`——同版本号重复发布即**覆盖该版本**；`skills` 表始终持有「最新版」
/// 快照（按版本号序），发布不高于现有最新版的旧版本号只更新版本副本，不回退最新版。
pub async fn upsert(
    State(state): S,
    headers: HeaderMap,
    Json(req): Json<SkillUpsertReq>,
) -> Result<Json<SkillInfo>, ApiError> {
    let claims = auth::require_user(&state, &headers).await?;
    let m = req.manifest;
    if !valid_name(&m.name) {
        return Err(ApiError::bad_request(
            "技能名仅允许字母、数字、_、-、.，且长度 1..=128",
        ));
    }
    let version = m.version.trim().to_string();
    if !valid_version(&version) {
        return Err(ApiError::bad_request(
            "版本号非法：仅允许字母、数字、.、_、-、+，长度 1..=64",
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

    // 既有记录：head 版本号 + 该版本已有副本的压缩体信息（upsert 元数据不应清空已上传的数据体）。
    let prev: Option<(i64, String, String, i64)> = match state
        .db
        .query_opt(
            "SELECT id, version, archive_sha256, archive_size FROM skills
             WHERE owner_id = ?1 AND name = ?2",
            db_params![claims.sub, m.name],
        )
        .await?
    {
        Some(r) => Some((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        None => None,
    };
    // 压缩体指针兜底链：请求携带 > 本版本既有副本 > head 快照（保持旧客户端行为）。
    let (archive_sha256, archive_size) = if req.archive_sha256.is_empty() {
        let from_version: Option<(String, i64)> = match &prev {
            Some((sid, ..)) => {
                let found: Option<(String, i64)> = match state
                    .db
                    .query_opt(
                        "SELECT archive_sha256, archive_size FROM skill_versions
                     WHERE skill_id = ?1 AND version = ?2",
                        db_params![sid, version],
                    )
                    .await?
                {
                    Some(r) => Some((r.get(0)?, r.get(1)?)),
                    None => None,
                };
                found.filter(|(sha, _)| !sha.is_empty())
            }
            None => None,
        };
        from_version.or_else(|| prev.as_ref().map(|(_, _, sha, size)| (sha.clone(), *size))).unwrap_or_default()
    } else {
        (req.archive_sha256.clone(), req.archive_size as i64)
    };

    // 发布旧版本号（低于现有最新版）时只写版本副本，不回退 head 快照。
    let is_head = match &prev {
        Some((_, head_ver, ..)) => compare_versions(&version, head_ver) != std::cmp::Ordering::Less,
        None => true,
    };
    if is_head {
        state
            .db
            .execute(
                "INSERT INTO skills(owner_id, name, category, visibility, version, description,
                                tags, skill_md, metadata, archive_sha256, archive_size, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
             ON CONFLICT(owner_id, name) DO UPDATE SET
                 category=excluded.category, visibility=excluded.visibility, version=excluded.version,
                 description=excluded.description, tags=excluded.tags, skill_md=excluded.skill_md,
                 metadata=excluded.metadata, archive_sha256=excluded.archive_sha256,
                 archive_size=excluded.archive_size, updated_at=excluded.updated_at",
                db_params![
                    claims.sub,
                    m.name,
                    category,
                    visibility,
                    version,
                    m.description,
                    tags_json,
                    req.skill_md,
                    meta_json,
                    archive_sha256,
                    archive_size,
                    now
                ],
            )
            .await?;
    }

    let row = state
        .db
        .query_row(
            "SELECT id, downloads FROM skills WHERE owner_id = ?1 AND name = ?2",
            db_params![claims.sub, m.name],
        )
        .await?;
    let (skill_id, downloads): (i64, i64) = (row.get(0)?, row.get(1)?);

    // 版本副本：同版本号重复发布即覆盖。覆盖若替换了压缩体，顺带 GC 失引用的旧 blob。
    let old_sha: Option<String> = match state
        .db
        .query_opt(
            "SELECT archive_sha256 FROM skill_versions WHERE skill_id = ?1 AND version = ?2",
            db_params![skill_id, version],
        )
        .await?
    {
        Some(r) => Some(r.get(0)?),
        None => None,
    };
    state
        .db
        .execute(
            "INSERT INTO skill_versions(skill_id, version, skill_md, metadata,
                                    archive_sha256, archive_size, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(skill_id, version) DO UPDATE SET
             skill_md=excluded.skill_md, metadata=excluded.metadata,
             archive_sha256=excluded.archive_sha256, archive_size=excluded.archive_size,
             created_at=excluded.created_at",
            db_params![skill_id, version, req.skill_md, meta_json, archive_sha256, archive_size, now],
        )
        .await?;
    if let Some(old) = old_sha.filter(|s| !s.is_empty() && *s != archive_sha256) {
        gc_blob_if_unreferenced(&state, &old).await;
    }

    // 受管标签（labels）：合并式关联（仅新增、去重），须为后台已存在的标签。
    // 空则不动既有关联，避免客户端重发布覆盖掉后台分配的标签。
    let labels = if m.labels.is_empty() {
        super::routes::labels_of(&state.db, "skill_labels", "skill_id", skill_id).await
    } else {
        super::routes::merge_resource_labels_by_name(
            &state.db,
            "skill_labels",
            "skill_id",
            skill_id,
            &m.labels,
        )
        .await?
    };
    let (likes, favorites, liked, favorited) = super::routes::reaction_summary(
        &state.db,
        "skill_reactions",
        "skill_id",
        skill_id,
        Some(claims.sub),
    )
    .await;
    let versions = versions_of(&state.db, skill_id).await;

    Ok(Json(SkillInfo {
        owner: claims.username,
        name: m.name,
        category,
        visibility,
        version,
        description: m.description,
        tags: lower_tags(&m.tags),
        mcp_dependencies: m.mcp_dependencies,
        preferred_tools: m.preferred_tools,
        skill_md: req.skill_md,
        archive_sha256,
        archive_size: archive_size as u64,
        labels,
        likes,
        favorites,
        downloads,
        liked,
        favorited,
        versions: versions.into_iter().map(|v| v.version).collect(),
        updated_at: now,
    }))
}

/// 详情/下载接口的版本选择参数：`?version=1.2.0`，缺省为最新版。
#[derive(serde::Deserialize)]
pub struct VersionQuery {
    version: Option<String>,
}

/// 技能详情。public 受分组可见性约束；private 仅 owner（带有效 token）可读。
/// 带 `?version=` 时返回该历史版本的副本内容（说明书/依赖/压缩体指针），缺省为最新版；
/// 响应的 `versions` 字段列出全部已发布版本（新→旧）。
pub async fn get(
    State(state): S,
    headers: HeaderMap,
    Path((owner, name)): Path<(String, String)>,
    Query(q): Query<VersionQuery>,
) -> Result<Json<SkillInfo>, ApiError> {
    let (mut info, group_vis, id) = load_skill(&state, &owner, &name).await?;
    ensure_skill_access(&state, &headers, &info, &group_vis).await?;
    info.versions = versions_of(&state.db, id).await.into_iter().map(|v| v.version).collect();
    if let Some(ver) = q.version.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        apply_version(&state.db, id, ver, &mut info).await?;
    }
    if let Some(claims) = auth::require_user_opt(&state, &headers).await {
        let (_, _, liked, favorited) = super::routes::reaction_summary(
            &state.db,
            "skill_reactions",
            "skill_id",
            id,
            Some(claims.sub),
        )
        .await;
        info.liked = liked;
        info.favorited = favorited;
    }
    Ok(Json(info))
}

/// 列出某技能已发布的全部版本副本（新→旧）。可见性校验同详情接口。
pub async fn versions(
    State(state): S,
    headers: HeaderMap,
    Path((owner, name)): Path<(String, String)>,
) -> Result<Json<Vec<SkillVersionInfo>>, ApiError> {
    let (info, group_vis, id) = load_skill(&state, &owner, &name).await?;
    ensure_skill_access(&state, &headers, &info, &group_vis).await?;
    Ok(Json(versions_of(&state.db, id).await))
}

/// 点赞 / 收藏一个技能（或取消）。资源须对当前用户可见。
pub async fn react(
    State(state): S,
    headers: HeaderMap,
    Path((owner, name)): Path<(String, String)>,
    Json(req): Json<ReactReq>,
) -> Result<Json<ReactResp>, ApiError> {
    let claims = auth::require_user(&state, &headers).await?;
    let (info, group_vis, id) = load_skill(&state, &owner, &name).await?;
    ensure_skill_access(&state, &headers, &info, &group_vis).await?;
    let resp = super::routes::set_reaction(
        &state.db,
        "skill_reactions",
        "skill_id",
        id,
        claims.sub,
        &req.kind,
        req.on,
    )
    .await?;
    Ok(Json(resp))
}

/// 把自己的技能转移给另一个用户。目标账号必须存在，且不能与其既有技能重名。
pub async fn transfer(
    State(state): S,
    headers: HeaderMap,
    Path((owner, name)): Path<(String, String)>,
    Json(req): Json<TransferReq>,
) -> Result<StatusCode, ApiError> {
    let claims = auth::require_user(&state, &headers).await?;
    if claims.username != owner {
        return Err(ApiError::unauthorized("只能转移自己的技能"));
    }
    let new_owner = req.new_owner.trim().to_string();
    let now = now_string();
    let target = super::routes::user_id_by_name(&state.db, &new_owner)
        .await?
        .ok_or_else(|| ApiError::not_found("目标用户不存在"))?;
    if target == claims.sub {
        return Err(ApiError::bad_request("不能转移给自己"));
    }
    let taken: bool = state
        .db
        .query_opt(
            "SELECT 1 FROM skills WHERE owner_id = ?1 AND name = ?2",
            db_params![target, name],
        )
        .await?
        .is_some();
    if taken {
        return Err(ApiError::conflict("对方已有同名技能，请先重命名"));
    }
    let n = state
        .db
        .execute(
            "UPDATE skills SET owner_id = ?1, updated_at = ?2 WHERE owner_id = ?3 AND name = ?4",
            db_params![target, now, claims.sub, name],
        )
        .await?;
    // MySQL 仅统计实际变更行：无变化的 UPDATE（如转让给自己）返回 0，须复核存在性再判 404。
    if n == 0 {
        let exists: bool = state
            .db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM skills WHERE owner_id = ?1 AND name = ?2)",
                db_params![claims.sub, name],
            )
            .await?
            .get(0)?;
        if !exists {
            return Err(ApiError::not_found("未找到该技能"));
        }
    }
    Ok(StatusCode::NO_CONTENT)
}

/// 当前用户收藏的技能（供 `/v1/favorites` 汇总），按收藏时间倒序。
/// 已失去可见性的（对方转私有 / 分组收紧）自动过滤。
pub(super) async fn favorites_of(
    state: &AppState,
    claims: &auth::Claims,
) -> Result<Vec<SkillInfo>, ApiError> {
    let viewer_groups = super::routes::groups_of_user(&state.db, claims.sub).await;
    let label_map = super::routes::all_resource_labels(&state.db, "skill_labels", "skill_id").await;
    let count_map = super::routes::all_reaction_counts(&state.db, "skill_reactions", "skill_id").await;
    let mine_map =
        super::routes::user_reaction_map(&state.db, "skill_reactions", "skill_id", claims.sub).await;
    let rows = state
        .db
        .query_map(
            "SELECT u.username, s.name, s.category, s.visibility, s.version, s.description,
                    s.tags, s.skill_md, s.metadata, s.archive_sha256, s.archive_size, s.updated_at,
                    s.downloads, s.group_visibility, s.id
             FROM skill_reactions r
             JOIN skills s ON s.id = r.skill_id
             JOIN users u ON u.id = s.owner_id
             WHERE r.user_id = ?1 AND r.kind = 'favorite'
             ORDER BY r.created_at DESC, s.name",
            db_params![claims.sub],
            |r| Ok((row_to_tuple(r)?, r.get::<String>(13)?, r.get::<i64>(14)?)),
        )
        .await?;
    let mut out = Vec::new();
    for (tuple, group_vis, id) in rows {
        let is_owner = tuple.0 == claims.username;
        if !is_owner
            && (tuple.3 != "public"
                || !super::routes::group_can_see(&group_vis, &viewer_groups, false))
        {
            continue;
        }
        let mut info = tuple_to_info(tuple)?;
        info.labels = label_map.get(&id).cloned().unwrap_or_default();
        (info.likes, info.favorites) = count_map.get(&id).copied().unwrap_or_default();
        (info.liked, info.favorited) = mine_map.get(&id).copied().unwrap_or_default();
        out.push(info);
    }
    Ok(out)
}

pub async fn delete(
    State(state): S,
    headers: HeaderMap,
    Path((owner, name)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let claims = auth::require_user(&state, &headers).await?;
    if claims.username != owner {
        return Err(ApiError::unauthorized("只能删除自己的技能"));
    }
    // 收集该技能全部版本引用的 blob（head 快照 + 版本副本），删除后逐个 GC。
    let shas: Vec<String> = collect_skill_shas(&state.db, claims.sub, &name).await?;
    let n = state
        .db
        .execute(
            "DELETE FROM skills WHERE owner_id = ?1 AND name = ?2",
            db_params![claims.sub, name],
        )
        .await?;
    if n == 0 {
        return Err(ApiError::not_found("未找到该技能"));
    }
    // 版本副本随 FK 级联删除；失引用的 blob 顺带清理（尽力而为）。
    for sha in shas {
        gc_blob_if_unreferenced(&state, &sha).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn rename(
    State(state): S,
    headers: HeaderMap,
    Path((owner, name)): Path<(String, String)>,
    Json(req): Json<SkillRenameReq>,
) -> Result<Json<SkillInfo>, ApiError> {
    let claims = auth::require_user(&state, &headers).await?;
    if claims.username != owner {
        return Err(ApiError::unauthorized("只能重命名自己的技能"));
    }
    let new_name = req.new_name.trim().to_string();
    if !valid_name(&new_name) {
        return Err(ApiError::bad_request("新名称非法"));
    }
    let now = now_string();
    if new_name != name {
        let taken: bool = state
            .db
            .query_opt(
                "SELECT 1 FROM skills WHERE owner_id = ?1 AND name = ?2",
                db_params![claims.sub, new_name],
            )
            .await?
            .is_some();
        if taken {
            return Err(ApiError::conflict("已存在同名技能"));
        }
    }
    let n = state
        .db
        .execute(
            "UPDATE skills SET name = ?1, updated_at = ?2 WHERE owner_id = ?3 AND name = ?4",
            db_params![new_name, now, claims.sub, name],
        )
        .await?;
    // MySQL 仅统计实际变更行：重命名为同名时返回 0，须复核存在性再判 404。
    if n == 0 {
        let exists: bool = state
            .db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM skills WHERE owner_id = ?1 AND name = ?2)",
                db_params![claims.sub, name],
            )
            .await?
            .get(0)?;
        if !exists {
            return Err(ApiError::not_found("未找到该技能"));
        }
    }
    Ok(Json(load_skill(&state, &owner, &new_name).await?.0))
}

/// 拖入压缩包创建技能——解析预览。仅需登录（不校验 owner，创建走 upsert）。
///
/// 接收原始压缩包字节（zip / tar.zst / tar.gz / 裸 tar），在阻塞线程里解包并归一化：
/// 剥离单层根目录、读取 tsk-skill.json 与说明书、重打成平台原生 tar.zst 按 sha256 落盘。
/// 回吐解析出的清单 + 说明书 + 压缩体 sha256/size，供 Web 端预填表单确认后再 upsert。
pub async fn inspect(
    State(state): S,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<SkillInspectResp>, ApiError> {
    auth::require_user(&state, &headers).await?;
    if body.is_empty() {
        return Err(ApiError::bad_request("压缩包为空"));
    }
    // 解包 + 重压缩是 CPU 密集型，移出异步线程。
    let extracted = tokio::task::spawn_blocking(move || super::skillpack::extract_skill(&body))
        .await
        .map_err(|e| ApiError::internal(format!("解包任务失败: {e}")))??;

    // 归一化后的 tar.zst 按内容寻址落盘（同内容复用；后续 upsert 以 sha 关联）。
    let sha = sha256_hex(&extracted.archive);
    let size = extracted.archive.len() as u64;
    if find_blob(&state, &sha).is_none() {
        let path = blob_write_path(&state, &sha, &extracted.archive);
        std::fs::write(&path, &extracted.archive)
            .map_err(|e| ApiError::internal(format!("写入压缩体失败: {e}")))?;
    }

    Ok(Json(SkillInspectResp {
        manifest: extracted.manifest,
        skill_md: extracted.skill_md,
        archive_sha256: sha,
        archive_size: size,
        file_count: extracted.file_count,
    }))
}

/// 上传技能压缩体（tar.zst 原始字节，亦兼容旧版 gzip）。仅 owner。服务端计算 sha256 落盘并写回记录。
/// 带 `?version=` 时挂到该版本副本（须先发布该版本元数据），缺省挂到最新版；
/// 目标版本恰为最新版时同步刷新 `skills` 快照。
pub async fn archive_put(
    State(state): S,
    headers: HeaderMap,
    Path((owner, name)): Path<(String, String)>,
    Query(q): Query<VersionQuery>,
    body: Bytes,
) -> Result<Json<serde_json::Value>, ApiError> {
    let claims = auth::require_user(&state, &headers).await?;
    if claims.username != owner {
        return Err(ApiError::unauthorized("只能上传自己的技能压缩体"));
    }
    if body.is_empty() {
        return Err(ApiError::bad_request("压缩体为空"));
    }
    let sha = sha256_hex(&body);
    let size = body.len() as i64;
    // 内容寻址落盘（若已存在同内容则复用）。扩展名随压缩格式（zstd/gzip）而定。
    if find_blob(&state, &sha).is_none() {
        let path = blob_write_path(&state, &sha, &body);
        std::fs::write(&path, &body)
            .map_err(|e| ApiError::internal(format!("写入压缩体失败: {e}")))?;
    }
    let now = now_string();
    let head: Option<(i64, String)> = match state
        .db
        .query_opt(
            "SELECT id, version FROM skills WHERE owner_id = ?1 AND name = ?2",
            db_params![claims.sub, name],
        )
        .await?
    {
        Some(r) => Some((r.get(0)?, r.get(1)?)),
        None => None,
    };
    let Some((skill_id, head_ver)) = head else {
        return Err(ApiError::not_found("请先发布技能元数据，再上传压缩体"));
    };
    let target_ver = q
        .version
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| head_ver.clone());

    // 覆盖版本副本的压缩体指针；被替换的旧 blob 失引用则 GC。
    let old_sha: Option<String> = match state
        .db
        .query_opt(
            "SELECT archive_sha256 FROM skill_versions WHERE skill_id = ?1 AND version = ?2",
            db_params![skill_id, target_ver],
        )
        .await?
    {
        Some(r) => Some(r.get(0)?),
        None => None,
    };
    if old_sha.is_none() {
        return Err(ApiError::not_found(format!(
            "版本 {target_ver} 不存在，请先发布该版本元数据"
        )));
    }
    state
        .db
        .execute(
            "UPDATE skill_versions SET archive_sha256 = ?1, archive_size = ?2, created_at = ?3
         WHERE skill_id = ?4 AND version = ?5",
            db_params![sha, size, now, skill_id, target_ver],
        )
        .await?;
    if target_ver == head_ver {
        state
            .db
            .execute(
                "UPDATE skills SET archive_sha256 = ?1, archive_size = ?2, updated_at = ?3
             WHERE id = ?4",
                db_params![sha, size, now, skill_id],
            )
            .await?;
    }
    if let Some(old) = old_sha.filter(|s| !s.is_empty() && *s != sha) {
        gc_blob_if_unreferenced(&state, &old).await;
    }
    Ok(Json(serde_json::json!({
        "archive_sha256": sha,
        "archive_size": size,
        "version": target_ver,
    })))
}

/// 下载技能压缩体。public 受分组可见性约束；private 仅 owner。
/// 带 `?version=` 时下载该历史版本的压缩体，缺省为最新版。
pub async fn archive_get(
    State(state): S,
    headers: HeaderMap,
    Path((owner, name)): Path<(String, String)>,
    Query(q): Query<VersionQuery>,
) -> Result<Response, ApiError> {
    let (mut info, group_vis, id) = load_skill(&state, &owner, &name).await?;
    ensure_skill_access(&state, &headers, &info, &group_vis).await?;
    if let Some(ver) = q.version.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        apply_version(&state.db, id, ver, &mut info).await?;
    }
    if info.archive_sha256.is_empty() {
        return Err(ApiError::not_found("该技能没有压缩体（纯文本裸说明书包）"));
    }
    let path = find_blob(&state, &info.archive_sha256)
        .ok_or_else(|| ApiError::not_found("压缩体文件缺失"))?;
    let bytes = std::fs::read(&path)
        .map_err(|e| ApiError::internal(format!("读取压缩体失败: {e}")))?;
    // 下载量 +1（尽力而为，失败不影响下载本身）。
    let _ = state
        .db
        .execute(
            "UPDATE skills SET downloads = downloads + 1 WHERE id = ?1",
            db_params![id],
        )
        .await;
    // 依据实际压缩格式给出正确的扩展名与 MIME（升级后为 zstd，历史包仍为 gzip）。
    let (ext, mime) = match crate::archive::detect(&bytes) {
        crate::archive::Format::Gzip => ("tar.gz", "application/gzip"),
        _ => ("tar.zst", "application/zstd"),
    };
    let filename = format!("{}-{}.{ext}", name, info.version);
    Ok((
        [
            (header::CONTENT_TYPE, mime.to_string()),
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

/// 版本号校验：字母/数字/`._-+`（覆盖 semver 及预发布/构建号写法），长度 1..=64。
fn valid_version(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '+'))
}

/// 某技能的全部已发布版本（版本号感知排序，新→旧）。
pub(super) async fn versions_of(db: &Db, skill_id: i64) -> Vec<SkillVersionInfo> {
    let mut out: Vec<SkillVersionInfo> = db
        .query_map(
            "SELECT version, archive_sha256, archive_size, created_at
         FROM skill_versions WHERE skill_id = ?1",
            db_params![skill_id],
            |r| {
                Ok(SkillVersionInfo {
                    version: r.get(0)?,
                    archive_sha256: r.get(1)?,
                    archive_size: r.get::<i64>(2)? as u64,
                    created_at: r.get(3)?,
                })
            },
        )
        .await
        .unwrap_or_default();
    out.sort_by(|a, b| compare_versions(&b.version, &a.version));
    out
}

/// 把 `info` 的版本相关字段（版本号/说明书/依赖/压缩体指针）替换为指定历史版本的副本。
/// 版本不存在报 not_found。
async fn apply_version(
    db: &Db,
    skill_id: i64,
    version: &str,
    info: &mut SkillInfo,
) -> Result<(), ApiError> {
    let row: Option<(String, String, String, i64)> = match db
        .query_opt(
            "SELECT skill_md, metadata, archive_sha256, archive_size
             FROM skill_versions WHERE skill_id = ?1 AND version = ?2",
            db_params![skill_id, version],
        )
        .await?
    {
        Some(r) => Some((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        None => None,
    };
    let (skill_md, meta_json, sha, size) = row.ok_or_else(|| {
        ApiError::not_found(format!("未找到版本 {version}（{}/{}）", info.owner, info.name))
    })?;
    let meta: SkillMeta = serde_json::from_str(&meta_json).unwrap_or_default();
    info.version = version.to_string();
    info.skill_md = skill_md;
    info.mcp_dependencies = meta.mcp_dependencies;
    info.preferred_tools = meta.preferred_tools;
    info.archive_sha256 = sha;
    info.archive_size = size as u64;
    Ok(())
}

/// 若 blob 已不被任何技能（head 快照或版本副本）引用，则删除文件（尽力而为）。
pub(super) async fn gc_blob_if_unreferenced(state: &AppState, sha: &str) {
    if sha.is_empty() {
        return;
    }
    let referenced: bool = state
        .db
        .query_row(
            "SELECT (EXISTS(SELECT 1 FROM skills WHERE archive_sha256 = ?1)
                  OR EXISTS(SELECT 1 FROM skill_versions WHERE archive_sha256 = ?1))",
            db_params![sha],
        )
        .await
        .and_then(|r| r.get::<bool>(0))
        .unwrap_or(true);
    if !referenced {
        if let Some(p) = find_blob(state, sha) {
            let _ = std::fs::remove_file(p);
        }
    }
}

/// 某技能（head 快照 + 全部版本副本）引用的 blob sha 去重清单。
async fn collect_skill_shas(
    db: &Db,
    owner_id: i64,
    name: &str,
) -> Result<Vec<String>, ApiError> {
    let shas = db
        .query_map(
            "SELECT DISTINCT sha FROM (
                 SELECT archive_sha256 AS sha FROM skills WHERE owner_id = ?1 AND name = ?2
                 UNION
                 SELECT v.archive_sha256 FROM skill_versions v
                 JOIN skills s ON s.id = v.skill_id
                 WHERE s.owner_id = ?1 AND s.name = ?2
             ) AS t WHERE sha != ''",
            db_params![owner_id, name],
            |r| r.get::<String>(0),
        )
        .await?;
    Ok(shas)
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

/// 写入路径：按内容魔数选择扩展名（zstd → `.tar.zst`，历史 gzip → `.tar.gz`）。
pub(super) fn blob_write_path(state: &AppState, sha: &str, bytes: &[u8]) -> std::path::PathBuf {
    state
        .blobs_dir
        .join(format!("{sha}.{}", crate::archive::blob_extension(bytes)))
}

/// 读取路径：内容寻址按 sha 定位，兼容新旧扩展名（升级前的 `.tar.gz` 仍可读）。
pub(super) fn find_blob(state: &AppState, sha: &str) -> Option<std::path::PathBuf> {
    for ext in ["tar.zst", "tar.gz", "bin"] {
        let p = state.blobs_dir.join(format!("{sha}.{ext}"));
        if p.exists() {
            return Some(p);
        }
    }
    None
}

pub(super) fn sha256_hex(bytes: &[u8]) -> String {
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
    i64,    // downloads
);

fn row_to_tuple(r: &super::db::Row) -> Result<SkillRow, super::db::DbError> {
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
        r.get(12)?,
    ))
}

fn tuple_to_info(t: SkillRow) -> Result<SkillInfo, ApiError> {
    let (owner, name, category, visibility, version, description, tags_json, skill_md, meta_json, sha, size, updated_at, downloads) =
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
        labels: Vec::new(),
        likes: 0,
        favorites: 0,
        downloads,
        liked: false,
        favorited: false,
        versions: Vec::new(),
        updated_at,
    })
}

async fn load_skill(state: &AppState, owner: &str, name: &str) -> Result<(SkillInfo, String, i64), ApiError> {
    let row: Option<(SkillRow, String, i64)> = match state
        .db
        .query_opt(
            "SELECT u.username, s.name, s.category, s.visibility, s.version, s.description,
                    s.tags, s.skill_md, s.metadata, s.archive_sha256, s.archive_size, s.updated_at,
                    s.downloads, s.group_visibility, s.id
             FROM skills s JOIN users u ON u.id = s.owner_id
             WHERE u.username = ?1 AND s.name = ?2",
            db_params![owner, name],
        )
        .await?
    {
        Some(r) => Some((row_to_tuple(&r)?, r.get::<String>(13)?, r.get::<i64>(14)?)),
        None => None,
    };
    let (row, group_vis, id) =
        row.ok_or_else(|| ApiError::not_found(format!("未找到技能: {owner}/{name}")))?;
    let labels = super::routes::labels_of(&state.db, "skill_labels", "skill_id", id).await;
    let (likes, favorites, _, _) =
        super::routes::reaction_summary(&state.db, "skill_reactions", "skill_id", id, None).await;
    let mut info = tuple_to_info(row)?;
    info.labels = labels;
    info.likes = likes;
    info.favorites = favorites;
    Ok((info, group_vis, id))
}

/// 非 owner 访问技能时的可见性 + 分组校验。不可见统一报 not_found（避免泄漏存在性）。
async fn ensure_skill_access(
    state: &AppState,
    headers: &HeaderMap,
    info: &SkillInfo,
    group_vis: &str,
) -> Result<(), ApiError> {
    let viewer = auth::require_user_opt(state, headers).await;
    let is_owner = viewer.as_ref().map(|c| c.username.as_str()) == Some(info.owner.as_str());
    if is_owner {
        return Ok(());
    }
    if info.visibility != "public" {
        return Err(ApiError::not_found("未找到该技能（或为私有）"));
    }
    let viewer_groups = match viewer.as_ref() {
        Some(c) => super::routes::groups_of_user(&state.db, c.sub).await,
        None => Vec::new(),
    };
    if !super::routes::group_can_see(group_vis, &viewer_groups, false) {
        return Err(ApiError::not_found("未找到该技能（或所属分组不可见）"));
    }
    Ok(())
}

/// 管理后台用：按 owner 用户名 + 技能名删除（含全部版本 blob 的 GC）。返回是否删除了行。
pub(super) async fn delete_skill_record(
    state: &AppState,
    owner: &str,
    name: &str,
) -> Result<bool, ApiError> {
    let oid: Option<i64> = match state
        .db
        .query_opt("SELECT id FROM users WHERE username = ?1", db_params![owner])
        .await?
    {
        Some(r) => Some(r.get(0)?),
        None => None,
    };
    let Some(oid) = oid else { return Ok(false) };
    let shas = collect_skill_shas(&state.db, oid, name).await?;
    let n = state
        .db
        .execute(
            "DELETE FROM skills WHERE owner_id = ?1 AND name = ?2",
            db_params![oid, name],
        )
        .await?;
    if n == 0 {
        return Ok(false);
    }
    for sha in shas {
        gc_blob_if_unreferenced(state, &sha).await;
    }
    Ok(true)
}
