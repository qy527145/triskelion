//! SQLite schema 与迁移。万物皆 Skill，但此处先落地闭环所需的三张核心表。

use anyhow::{Context, Result};
use rusqlite::Connection;

/// 建表（幂等）。
pub fn init(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
        PRAGMA journal_mode = WAL;
        PRAGMA foreign_keys = ON;

        CREATE TABLE IF NOT EXISTS users (
            id            INTEGER PRIMARY KEY AUTOINCREMENT,
            username      TEXT NOT NULL UNIQUE,
            password_hash TEXT NOT NULL,
            created_at    TEXT NOT NULL
        );

        -- MCP 注册表：manifest 以 JSON 文本整存，逻辑分类/可见性单列索引。
        CREATE TABLE IF NOT EXISTS mcps (
            id         INTEGER PRIMARY KEY AUTOINCREMENT,
            owner_id   INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            name       TEXT NOT NULL,
            visibility TEXT NOT NULL DEFAULT 'private',
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

        -- 技能市场：万物皆 Skill。category 为逻辑分类标签（skill/kb/toolchain）。
        -- 服务端只持元数据与 SKILL.md 文本；庞大的数据体以压缩包形式承载，
        -- 按 sha256 内容寻址落盘于 blobs/，此处仅存 sha256 与字节数。
        CREATE TABLE IF NOT EXISTS skills (
            id             INTEGER PRIMARY KEY AUTOINCREMENT,
            owner_id       INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            name           TEXT NOT NULL,
            category       TEXT NOT NULL DEFAULT 'skill',
            visibility     TEXT NOT NULL DEFAULT 'private',
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
        "#,
    )
    .context("初始化数据库 schema")?;

    // 迁移：为旧库补上 tools 列（已存在则忽略 "duplicate column name"）。
    let _ = conn.execute(
        "ALTER TABLE mcps ADD COLUMN tools TEXT NOT NULL DEFAULT '[]'",
        [],
    );
    Ok(())
}
