//! Schema 与迁移：单模板 + 方言 token 渲染，三后端共用一份表结构定义。
//!
//! 设计要点：
//! - `CREATE TABLE IF NOT EXISTS` 幂等建表；既有 SQLite 库沿用旧表定义，无迁移动作。
//! - 外键用**表级** `FOREIGN KEY` 子句：MySQL 会静默忽略列内联 `REFERENCES`，
//!   而删除用户/技能的逻辑依赖 `ON DELETE CASCADE`，必须表级声明才三库一致。
//! - 补列迁移沿用吞错式 `ALTER TABLE ADD COLUMN`（duplicate column 错误直接忽略），
//!   与历史行为一致，且为三库最小公倍数写法（MySQL 8 无 `IF NOT EXISTS`）。
//! - MySQL 的 TEXT/LONGTEXT 列不允许字面量 DEFAULT，渲染时转为表达式默认值
//!   `DEFAULT ('…')`（要求 MySQL ≥ 8.0.13 / MariaDB ≥ 10.2）。

use anyhow::{Context, Result};

use super::{Db, Dialect, db_params};

/// 表结构模板。token：{PK} 自增主键、{INT} 整数、{KEYTEXT} 进唯一索引/主键的文本、
/// {DOC} 大文本、{BLOB} 二进制、{TAIL} 建表尾缀（MySQL 指定引擎与 collation）。
const SCHEMA: &str = r#"
-- 用户分组：管理后台维护的逻辑分组，用于市场资源的可见性控制。
CREATE TABLE IF NOT EXISTS groups (
    id          {PK},
    name        {KEYTEXT} NOT NULL UNIQUE,
    description {DOC} NOT NULL DEFAULT '',
    created_at  {KEYTEXT} NOT NULL
){TAIL};

CREATE TABLE IF NOT EXISTS users (
    id            {PK},
    username      {KEYTEXT} NOT NULL UNIQUE,
    password_hash {KEYTEXT} NOT NULL,
    -- 历史遗留的单分组列（保留以兼容旧库，现已由 user_groups 多对多关联取代）。
    group_id      {INT},
    -- 认证来源：'local' 本地口令 / 'ldap' 企业目录（影子账号，口令不可本地登录）。
    auth_source   {KEYTEXT} NOT NULL DEFAULT 'local',
    created_at    {KEYTEXT} NOT NULL
){TAIL};

-- 用户 ↔ 分组多对多关联：一个用户可绑定多个分组。删除用户或分组时随 FK 级联清理。
CREATE TABLE IF NOT EXISTS user_groups (
    user_id  {INT} NOT NULL,
    group_id {INT} NOT NULL,
    PRIMARY KEY (user_id, group_id),
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
    FOREIGN KEY (group_id) REFERENCES groups(id) ON DELETE CASCADE
){TAIL};

-- MCP 注册表：manifest 以 JSON 文本整存，逻辑分类/可见性单列索引。
-- group_visibility：'all' 表示所有分组可见（默认），或 JSON 数组 [id,...] 限定可见分组。
CREATE TABLE IF NOT EXISTS mcps (
    id         {PK},
    owner_id   {INT} NOT NULL,
    name       {KEYTEXT} NOT NULL,
    visibility {KEYTEXT} NOT NULL DEFAULT 'private',
    group_visibility {DOC} NOT NULL DEFAULT 'all',
    version    {KEYTEXT} NOT NULL DEFAULT '0.1.0',
    manifest   {DOC} NOT NULL,
    tools      {DOC} NOT NULL DEFAULT '[]',
    updated_at {KEYTEXT} NOT NULL,
    UNIQUE(owner_id, name),
    FOREIGN KEY (owner_id) REFERENCES users(id) ON DELETE CASCADE
){TAIL};

-- 加密凭据池：AES-256-GCM，nonce 与密文分列存储。
CREATE TABLE IF NOT EXISTS secrets (
    id         {PK},
    owner_id   {INT} NOT NULL,
    key        {KEYTEXT} NOT NULL,
    nonce      {BLOB} NOT NULL,
    ciphertext {BLOB} NOT NULL,
    updated_at {KEYTEXT} NOT NULL,
    UNIQUE(owner_id, key),
    FOREIGN KEY (owner_id) REFERENCES users(id) ON DELETE CASCADE
){TAIL};

-- 技能市场：万物皆 Skill。category 为逻辑分类标签（skill/kb/toolchain/agent）。
-- 服务端只持元数据与说明书文本（skill_md 列，agent 分类对应 AGENT.md，其余 SKILL.md）；
-- 庞大的数据体以压缩包形式承载，按 sha256 内容寻址落盘于 blobs/，此处仅存 sha256 与字节数。
CREATE TABLE IF NOT EXISTS skills (
    id             {PK},
    owner_id       {INT} NOT NULL,
    name           {KEYTEXT} NOT NULL,
    category       {KEYTEXT} NOT NULL DEFAULT 'skill',
    visibility     {KEYTEXT} NOT NULL DEFAULT 'private',
    group_visibility {DOC} NOT NULL DEFAULT 'all',
    version        {KEYTEXT} NOT NULL DEFAULT '0.1.0',
    description    {DOC} NOT NULL DEFAULT '',
    tags           {DOC} NOT NULL DEFAULT '[]',
    skill_md       {DOC} NOT NULL DEFAULT '',
    metadata       {DOC} NOT NULL DEFAULT '{}',
    archive_sha256 {KEYTEXT} NOT NULL DEFAULT '',
    archive_size   {INT} NOT NULL DEFAULT 0,
    downloads      {INT} NOT NULL DEFAULT 0,
    updated_at     {KEYTEXT} NOT NULL,
    UNIQUE(owner_id, name),
    FOREIGN KEY (owner_id) REFERENCES users(id) ON DELETE CASCADE
){TAIL};

-- 技能版本历史：服务端保留每个已发布版本的完整副本（说明书 + 元数据 + 压缩体指针），
-- 客户端可按版本拉取；重复发布同一版本号则覆盖该版本。skills 表始终持有「最新版」快照。
-- 压缩体按 sha256 内容寻址落盘于 blobs/，多版本同内容自动复用。
CREATE TABLE IF NOT EXISTS skill_versions (
    id             {PK},
    skill_id       {INT} NOT NULL,
    version        {KEYTEXT} NOT NULL,
    skill_md       {DOC} NOT NULL DEFAULT '',
    metadata       {DOC} NOT NULL DEFAULT '{}',
    archive_sha256 {KEYTEXT} NOT NULL DEFAULT '',
    archive_size   {INT} NOT NULL DEFAULT 0,
    created_at     {KEYTEXT} NOT NULL,
    UNIQUE(skill_id, version),
    FOREIGN KEY (skill_id) REFERENCES skills(id) ON DELETE CASCADE
){TAIL};

-- 受管标签（taxonomy）：管理后台维护的标签，用于市场资源的标注与筛选。
-- 默认内置「官方」「社区」两种（见 init 末尾的 seed）。
CREATE TABLE IF NOT EXISTS labels (
    id         {PK},
    name       {KEYTEXT} NOT NULL UNIQUE,
    created_at {KEYTEXT} NOT NULL DEFAULT ''
){TAIL};

-- 资源 ↔ 标签多对多关联（技能、MCP 各一张），随资源/标签删除级联清理。
CREATE TABLE IF NOT EXISTS skill_labels (
    skill_id {INT} NOT NULL,
    label_id {INT} NOT NULL,
    PRIMARY KEY (skill_id, label_id),
    FOREIGN KEY (skill_id) REFERENCES skills(id) ON DELETE CASCADE,
    FOREIGN KEY (label_id) REFERENCES labels(id) ON DELETE CASCADE
){TAIL};
CREATE TABLE IF NOT EXISTS mcp_labels (
    mcp_id   {INT} NOT NULL,
    label_id {INT} NOT NULL,
    PRIMARY KEY (mcp_id, label_id),
    FOREIGN KEY (mcp_id) REFERENCES mcps(id) ON DELETE CASCADE,
    FOREIGN KEY (label_id) REFERENCES labels(id) ON DELETE CASCADE
){TAIL};

-- 资源互动：点赞 / 收藏（用户 ↔ 资源多对多，kind 取 'like' / 'favorite'），
-- 随用户 / 资源删除级联清理。下载量走 skills.downloads 计数列。
CREATE TABLE IF NOT EXISTS skill_reactions (
    user_id    {INT} NOT NULL,
    skill_id   {INT} NOT NULL,
    kind       {KEYTEXT} NOT NULL,
    created_at {KEYTEXT} NOT NULL,
    PRIMARY KEY (user_id, skill_id, kind),
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
    FOREIGN KEY (skill_id) REFERENCES skills(id) ON DELETE CASCADE
){TAIL};
CREATE TABLE IF NOT EXISTS mcp_reactions (
    user_id    {INT} NOT NULL,
    mcp_id     {INT} NOT NULL,
    kind       {KEYTEXT} NOT NULL,
    created_at {KEYTEXT} NOT NULL,
    PRIMARY KEY (user_id, mcp_id, kind),
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
    FOREIGN KEY (mcp_id) REFERENCES mcps(id) ON DELETE CASCADE
){TAIL};

-- 系统设置：管理后台运行时可改的 key-value（value 为 JSON 文本），
-- 如认证配置（注册开关、LDAP）。敏感字段由写入方自行加密后再入库。
CREATE TABLE IF NOT EXISTS settings (
    key   {KEYTEXT} NOT NULL PRIMARY KEY,
    value {DOC} NOT NULL
){TAIL};

-- 工具调用审计：每次经 Hub 网关代调用 MCP 工具时记录一行，供管理后台统计
-- 24h/累计调用量、热门工具与最近错误。caller 为发起者用户名快照。
CREATE TABLE IF NOT EXISTS tool_calls (
    id         {PK},
    caller     {KEYTEXT} NOT NULL DEFAULT '',
    owner      {KEYTEXT} NOT NULL DEFAULT '',
    mcp_name   {KEYTEXT} NOT NULL DEFAULT '',
    tool       {KEYTEXT} NOT NULL DEFAULT '',
    ok         {INT} NOT NULL DEFAULT 1,
    error      {DOC} NOT NULL DEFAULT '',
    result     {DOC} NOT NULL DEFAULT '',
    ms         {INT} NOT NULL DEFAULT 0,
    created_at {KEYTEXT} NOT NULL,
    created_ts {INT} NOT NULL DEFAULT 0
){TAIL}
"#;

/// 索引单列一份：MySQL 不支持 `CREATE INDEX IF NOT EXISTS`，需按方言分别处理。
const INDEXES: &str = "CREATE INDEX IF NOT EXISTS idx_tool_calls_ts ON tool_calls(created_ts)";

/// 吞错式补列迁移（旧库升级；已存在则报 duplicate column，直接忽略）。
const MIGRATIONS: &[&str] = &[
    "ALTER TABLE mcps ADD COLUMN tools {DOC} NOT NULL DEFAULT '[]'",
    "ALTER TABLE users ADD COLUMN group_id {INT}",
    "ALTER TABLE mcps ADD COLUMN group_visibility {DOC} NOT NULL DEFAULT 'all'",
    "ALTER TABLE skills ADD COLUMN group_visibility {DOC} NOT NULL DEFAULT 'all'",
    "ALTER TABLE tool_calls ADD COLUMN result {DOC} NOT NULL DEFAULT ''",
    "ALTER TABLE skills ADD COLUMN downloads {INT} NOT NULL DEFAULT 0",
    "ALTER TABLE users ADD COLUMN auth_source {KEYTEXT} NOT NULL DEFAULT 'local'",
];

fn tokens(d: Dialect) -> [(&'static str, &'static str); 6] {
    match d {
        Dialect::Sqlite => [
            ("{PK}", "INTEGER PRIMARY KEY AUTOINCREMENT"),
            ("{INT}", "INTEGER"),
            ("{KEYTEXT}", "TEXT"),
            ("{DOC}", "TEXT"),
            ("{BLOB}", "BLOB"),
            ("{TAIL}", ""),
        ],
        Dialect::Pg => [
            ("{PK}", "BIGINT GENERATED BY DEFAULT AS IDENTITY PRIMARY KEY"),
            // 整数一律 BIGINT，避免 INT2/INT4 在解码层的类型分歧。
            ("{INT}", "BIGINT"),
            ("{KEYTEXT}", "TEXT"),
            ("{DOC}", "TEXT"),
            ("{BLOB}", "BYTEA"),
            ("{TAIL}", ""),
        ],
        Dialect::MySql => [
            ("{PK}", "BIGINT AUTO_INCREMENT PRIMARY KEY"),
            ("{INT}", "BIGINT"),
            // utf8mb4 下 255×4+2 字节，低于 InnoDB 3072 字节索引前缀上限。
            ("{KEYTEXT}", "VARCHAR(255)"),
            ("{DOC}", "LONGTEXT"),
            ("{BLOB}", "LONGBLOB"),
            // utf8mb4_bin 保证大小写敏感，与 SQLite/PG 的用户名/资源名唯一性语义一致。
            ("{TAIL}", " ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin"),
        ],
    }
}

fn render(d: Dialect, template: &str) -> String {
    let mut s = template.to_string();
    for (k, v) in tokens(d) {
        s = s.replace(k, v);
    }
    if d == Dialect::MySql {
        s = mysql_fix_text_defaults(&s);
        // groups 表 / key 列撞上 MySQL 保留字，DDL 同样需要反引号。
        s = super::translate::mysql_quote_reserved(&s);
    }
    s
}

/// MySQL 不允许 TEXT/LONGTEXT 列带字面量 DEFAULT，转为表达式默认值 `DEFAULT ('…')`。
fn mysql_fix_text_defaults(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len() + 16);
    let mut rest = sql;
    const PAT: &str = "TEXT NOT NULL DEFAULT '";
    while let Some(pos) = rest.find(PAT) {
        let lit_start = pos + PAT.len();
        // 找到字面量收尾的单引号（本模板的默认值不含转义引号）
        let Some(lit_len) = rest[lit_start..].find('\'') else {
            break;
        };
        out.push_str(&rest[..pos]);
        out.push_str("TEXT NOT NULL DEFAULT ('");
        out.push_str(&rest[lit_start..lit_start + lit_len]);
        out.push_str("')");
        rest = &rest[lit_start + lit_len + 1..];
    }
    out.push_str(rest);
    out
}

/// 按分号切分为独立语句（模板内无触发器/存储过程，切分安全），去掉注释行。
fn split_statements(sql: &str) -> Vec<String> {
    sql.split(';')
        .map(|stmt| {
            stmt.lines()
                .filter(|l| !l.trim_start().starts_with("--"))
                .collect::<Vec<_>>()
                .join("\n")
                .trim()
                .to_string()
        })
        .filter(|s| !s.is_empty())
        .collect()
}

/// 建表 + 迁移 + 回填 + 种子（幂等）。
pub async fn init(db: &Db) -> Result<()> {
    for stmt in split_statements(&render(db.dialect, SCHEMA)) {
        db.exec_raw(&stmt)
            .await
            .map_err(anyhow::Error::from)
            .with_context(|| format!("初始化数据库 schema: {}", stmt.lines().next().unwrap_or("")))?;
    }

    // 索引：MySQL 无 IF NOT EXISTS，吞 duplicate key name 错误（1061）。
    match db.dialect {
        Dialect::MySql => {
            let _ = db.exec_raw(&INDEXES.replace(" IF NOT EXISTS", "")).await;
        }
        _ => db
            .exec_raw(INDEXES)
            .await
            .map_err(anyhow::Error::from)
            .context("创建索引")?,
    }

    // 补列迁移（吞错）。
    for stmt in MIGRATIONS {
        let _ = db.exec_raw(&render(db.dialect, stmt)).await;
    }

    // 一次性回填：把旧的单分组 users.group_id 并入多对多关联表（仅当关联表为空时执行，
    // 避免重复回填或覆盖后续的多分组编辑）。
    let ug_count: i64 = match db.query_row("SELECT COUNT(*) FROM user_groups", db_params![]).await {
        Ok(r) => r.get(0).unwrap_or(0),
        Err(_) => 0,
    };
    if ug_count == 0 {
        let _ = db
            .execute(
                "INSERT INTO user_groups(user_id, group_id)
                 SELECT id, group_id FROM users WHERE group_id IS NOT NULL
                 ON CONFLICT DO NOTHING",
                db_params![],
            )
            .await;
    }

    // 回填版本历史：旧库里已有的技能（升级前只存最新版）把当前快照补录为一个版本副本。
    // 幂等：UNIQUE(skill_id, version) + DO NOTHING，已有版本行则不动。
    // SELECT 无 WHERE 时直接接 upsert 子句在 SQLite 有解析歧义，补 WHERE true 规避。
    let _ = db
        .execute(
            "INSERT INTO skill_versions(skill_id, version, skill_md, metadata,
                                        archive_sha256, archive_size, created_at)
             SELECT id, version, skill_md, metadata, archive_sha256, archive_size, updated_at
             FROM skills WHERE true
             ON CONFLICT DO NOTHING",
            db_params![],
        )
        .await;

    // 内置默认标签：官方 / 社区（幂等，已存在则忽略）。
    let _ = db
        .execute(
            "INSERT INTO labels(name) VALUES ('官方'), ('社区') ON CONFLICT DO NOTHING",
            db_params![],
        )
        .await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sqlite_render_matches_legacy() {
        let s = render(Dialect::Sqlite, SCHEMA);
        assert!(s.contains("id          INTEGER PRIMARY KEY AUTOINCREMENT"));
        for tok in ["{PK}", "{INT}", "{KEYTEXT}", "{DOC}", "{BLOB}", "{TAIL}"] {
            assert!(!s.contains(tok), "存在未替换的 token: {tok}");
        }
    }

    #[test]
    fn mysql_text_defaults_wrapped() {
        let s = render(Dialect::MySql, SCHEMA);
        assert!(s.contains("LONGTEXT NOT NULL DEFAULT ('[]')"));
        assert!(!s.contains("LONGTEXT NOT NULL DEFAULT '"), "MySQL 不允许 TEXT 字面量默认值");
        assert!(s.contains("ENGINE=InnoDB"));
    }

    #[test]
    fn statements_split() {
        let stmts = split_statements(&render(Dialect::Sqlite, SCHEMA));
        assert_eq!(stmts.len(), 14, "应为 14 张表（索引单列），实际 {}", stmts.len());
        assert!(stmts.iter().all(|s| s.to_uppercase().starts_with("CREATE TABLE")));
    }
}
