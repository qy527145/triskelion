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
