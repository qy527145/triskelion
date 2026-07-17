//! SQLite schema 与迁移。万物皆 Skill，但此处先落地闭环所需的三张核心表。

use anyhow::{Context, Result};
use rusqlite::Connection;

/// 建表（幂等）。
pub fn init(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
        PRAGMA journal_mode = WAL;
        PRAGMA foreign_keys = ON;

        -- 用户分组：管理后台维护的逻辑分组，用于市场资源的可见性控制。
        CREATE TABLE IF NOT EXISTS groups (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            name        TEXT NOT NULL UNIQUE,
            description TEXT NOT NULL DEFAULT '',
            created_at  TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS users (
            id            INTEGER PRIMARY KEY AUTOINCREMENT,
            username      TEXT NOT NULL UNIQUE,
            password_hash TEXT NOT NULL,
            -- 历史遗留的单分组列（保留以兼容旧库，现已由 user_groups 多对多关联取代）。
            group_id      INTEGER,
            created_at    TEXT NOT NULL
        );

        -- 用户 ↔ 分组多对多关联：一个用户可绑定多个分组。删除用户或分组时随 FK 级联清理。
        CREATE TABLE IF NOT EXISTS user_groups (
            user_id  INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            group_id INTEGER NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
            PRIMARY KEY (user_id, group_id)
        );

        -- MCP 注册表：manifest 以 JSON 文本整存，逻辑分类/可见性单列索引。
        -- group_visibility：'all' 表示所有分组可见（默认），或 JSON 数组 [id,...] 限定可见分组。
        CREATE TABLE IF NOT EXISTS mcps (
            id         INTEGER PRIMARY KEY AUTOINCREMENT,
            owner_id   INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            name       TEXT NOT NULL,
            visibility TEXT NOT NULL DEFAULT 'private',
            group_visibility TEXT NOT NULL DEFAULT 'all',
            version    TEXT NOT NULL DEFAULT '0.1.0',
            manifest   TEXT NOT NULL,
            tools      TEXT NOT NULL DEFAULT '[]',
            updated_at TEXT NOT NULL,
            UNIQUE(owner_id, name)
        );

        -- 加密凭据池：AES-256-GCM，nonce 与密文分列存储。
        CREATE TABLE IF NOT EXISTS secrets (
            id         INTEGER PRIMARY KEY AUTOINCREMENT,
            owner_id   INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            key        TEXT NOT NULL,
            nonce      BLOB NOT NULL,
            ciphertext BLOB NOT NULL,
            updated_at TEXT NOT NULL,
            UNIQUE(owner_id, key)
        );

        -- 技能市场：万物皆 Skill。category 为逻辑分类标签（skill/kb/toolchain/agent）。
        -- 服务端只持元数据与说明书文本（skill_md 列，agent 分类对应 AGENT.md，其余 SKILL.md）；
        -- 庞大的数据体以压缩包形式承载，按 sha256 内容寻址落盘于 blobs/，此处仅存 sha256 与字节数。
        CREATE TABLE IF NOT EXISTS skills (
            id             INTEGER PRIMARY KEY AUTOINCREMENT,
            owner_id       INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            name           TEXT NOT NULL,
            category       TEXT NOT NULL DEFAULT 'skill',
            visibility     TEXT NOT NULL DEFAULT 'private',
            group_visibility TEXT NOT NULL DEFAULT 'all',
            version        TEXT NOT NULL DEFAULT '0.1.0',
            description    TEXT NOT NULL DEFAULT '',
            tags           TEXT NOT NULL DEFAULT '[]',
            skill_md       TEXT NOT NULL DEFAULT '',
            metadata       TEXT NOT NULL DEFAULT '{}',
            archive_sha256 TEXT NOT NULL DEFAULT '',
            archive_size   INTEGER NOT NULL DEFAULT 0,
            updated_at     TEXT NOT NULL,
            UNIQUE(owner_id, name)
        );

        -- 技能版本历史：服务端保留每个已发布版本的完整副本（说明书 + 元数据 + 压缩体指针），
        -- 客户端可按版本拉取；重复发布同一版本号则覆盖该版本。skills 表始终持有「最新版」快照。
        -- 压缩体按 sha256 内容寻址落盘于 blobs/，多版本同内容自动复用。
        CREATE TABLE IF NOT EXISTS skill_versions (
            id             INTEGER PRIMARY KEY AUTOINCREMENT,
            skill_id       INTEGER NOT NULL REFERENCES skills(id) ON DELETE CASCADE,
            version        TEXT NOT NULL,
            skill_md       TEXT NOT NULL DEFAULT '',
            metadata       TEXT NOT NULL DEFAULT '{}',
            archive_sha256 TEXT NOT NULL DEFAULT '',
            archive_size   INTEGER NOT NULL DEFAULT 0,
            created_at     TEXT NOT NULL,
            UNIQUE(skill_id, version)
        );

        -- 受管标签（taxonomy）：管理后台维护的标签，用于市场资源的标注与筛选。
        -- 默认内置「官方」「社区」两种（见 init 末尾的 seed）。
        CREATE TABLE IF NOT EXISTS labels (
            id         INTEGER PRIMARY KEY AUTOINCREMENT,
            name       TEXT NOT NULL UNIQUE,
            created_at TEXT NOT NULL DEFAULT ''
        );

        -- 资源 ↔ 标签多对多关联（技能、MCP 各一张），随资源/标签删除级联清理。
        CREATE TABLE IF NOT EXISTS skill_labels (
            skill_id INTEGER NOT NULL REFERENCES skills(id) ON DELETE CASCADE,
            label_id INTEGER NOT NULL REFERENCES labels(id) ON DELETE CASCADE,
            PRIMARY KEY (skill_id, label_id)
        );
        CREATE TABLE IF NOT EXISTS mcp_labels (
            mcp_id   INTEGER NOT NULL REFERENCES mcps(id) ON DELETE CASCADE,
            label_id INTEGER NOT NULL REFERENCES labels(id) ON DELETE CASCADE,
            PRIMARY KEY (mcp_id, label_id)
        );

        -- 资源互动：点赞 / 收藏（用户 ↔ 资源多对多，kind 取 'like' / 'favorite'），
        -- 随用户 / 资源删除级联清理。下载量走 skills.downloads 计数列（见下方迁移）。
        CREATE TABLE IF NOT EXISTS skill_reactions (
            user_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            skill_id   INTEGER NOT NULL REFERENCES skills(id) ON DELETE CASCADE,
            kind       TEXT NOT NULL,
            created_at TEXT NOT NULL,
            PRIMARY KEY (user_id, skill_id, kind)
        );
        CREATE TABLE IF NOT EXISTS mcp_reactions (
            user_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            mcp_id     INTEGER NOT NULL REFERENCES mcps(id) ON DELETE CASCADE,
            kind       TEXT NOT NULL,
            created_at TEXT NOT NULL,
            PRIMARY KEY (user_id, mcp_id, kind)
        );

        -- 工具调用审计：每次经 Hub 网关代调用 MCP 工具时记录一行，供管理后台统计
        -- 24h/累计调用量、热门工具与最近错误。caller 为发起者用户名快照。
        CREATE TABLE IF NOT EXISTS tool_calls (
            id         INTEGER PRIMARY KEY AUTOINCREMENT,
            caller     TEXT NOT NULL DEFAULT '',
            owner      TEXT NOT NULL DEFAULT '',
            mcp_name   TEXT NOT NULL DEFAULT '',
            tool       TEXT NOT NULL DEFAULT '',
            ok         INTEGER NOT NULL DEFAULT 1,
            error      TEXT NOT NULL DEFAULT '',
            result     TEXT NOT NULL DEFAULT '',
            ms         INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL,
            created_ts INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_tool_calls_ts ON tool_calls(created_ts);
        "#,
    )
    .context("初始化数据库 schema")?;

    // 迁移：为旧库补上 tools 列（已存在则忽略 "duplicate column name"）。
    let _ = conn.execute(
        "ALTER TABLE mcps ADD COLUMN tools TEXT NOT NULL DEFAULT '[]'",
        [],
    );
    // 迁移：用户分组、市场资源的分组可见性（旧库补列，已存在则忽略）。
    let _ = conn.execute("ALTER TABLE users ADD COLUMN group_id INTEGER", []);
    let _ = conn.execute(
        "ALTER TABLE mcps ADD COLUMN group_visibility TEXT NOT NULL DEFAULT 'all'",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE skills ADD COLUMN group_visibility TEXT NOT NULL DEFAULT 'all'",
        [],
    );
    // 迁移：调用审计补上「结果摘要」列（成功调用的结果概要 / 失败可留空，旧库补列）。
    let _ = conn.execute(
        "ALTER TABLE tool_calls ADD COLUMN result TEXT NOT NULL DEFAULT ''",
        [],
    );
    // 迁移：技能下载量计数列（旧库补列，已存在则忽略）。
    let _ = conn.execute(
        "ALTER TABLE skills ADD COLUMN downloads INTEGER NOT NULL DEFAULT 0",
        [],
    );

    // 一次性回填：把旧的单分组 users.group_id 并入多对多关联表（仅当关联表为空时执行，
    // 避免重复回填或覆盖后续的多分组编辑）。
    let ug_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM user_groups", [], |r| r.get(0))
        .unwrap_or(0);
    if ug_count == 0 {
        let _ = conn.execute(
            "INSERT OR IGNORE INTO user_groups(user_id, group_id)
             SELECT id, group_id FROM users WHERE group_id IS NOT NULL",
            [],
        );
    }

    // 回填版本历史：旧库里已有的技能（升级前只存最新版）把当前快照补录为一个版本副本。
    // 幂等：UNIQUE(skill_id, version) + OR IGNORE，已有版本行则不动。
    let _ = conn.execute(
        "INSERT OR IGNORE INTO skill_versions(skill_id, version, skill_md, metadata,
                                              archive_sha256, archive_size, created_at)
         SELECT id, version, skill_md, metadata, archive_sha256, archive_size, updated_at
         FROM skills",
        [],
    );

    // 内置默认标签：官方 / 社区（幂等，已存在则忽略）。
    let _ = conn.execute(
        "INSERT OR IGNORE INTO labels(name) VALUES ('官方'), ('社区')",
        [],
    );
    Ok(())
}
