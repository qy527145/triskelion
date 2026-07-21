//! SQL 方言翻译：把规范形态（SQLite ∩ PG 通用语法 + `?N` 编号占位）重写为目标后端形态。
//!
//! - SQLite：原样直通（`?N` 原生支持）。
//! - PostgreSQL：`?N` → `$N`（PG 原生支持编号复用，其余语法本就通用）。
//! - MySQL：`?N` 按出现顺序展开为 `?` 并按序复制参数（天然消解复用/乱序）；
//!   `ON CONFLICT … DO UPDATE SET x=excluded.x` → `ON DUPLICATE KEY UPDATE x=VALUES(x)`；
//!   `ON CONFLICT … DO NOTHING` → 改写行首为 `INSERT IGNORE INTO`。
//!
//! 所有函数都跳过单引号字符串字面量内部（SQL 中 `''` 为转义），不会误改文本常量。

use std::borrow::Cow;

use super::{Dialect, Value};

pub fn rewrite(dialect: Dialect, sql: &str, params: Vec<Value>) -> (Cow<'_, str>, Vec<Value>) {
    match dialect {
        Dialect::Sqlite => (Cow::Borrowed(sql), params),
        Dialect::Pg => (Cow::Owned(numbered_to_dollar(sql)), params),
        Dialect::MySql => {
            let (sql, params) = mysql_rewrite(sql, params);
            (Cow::Owned(sql), params)
        }
    }
}

/// 逐字符扫描的字面量感知遍历：对字面量外的每个位置调用 `f`，字面量内容原样透传。
/// `f` 返回 Some(跳过的字节数) 表示已消费并替换（模式均为 ASCII，字节数即字符边界），
/// 返回 None 则原样保留该字符。UTF-8 安全：按字符推进，绝不切在多字节字符中间。
fn scan_sql(sql: &str, out: &mut String, mut f: impl FnMut(&str, &mut String) -> Option<usize>) {
    let mut i = 0;
    let mut in_str = false;
    while i < sql.len() {
        let rest = &sql[i..];
        let c = rest.chars().next().expect("非空片段必有字符");
        if in_str {
            out.push(c);
            if c == '\'' {
                // '' 是字面量内的转义单引号
                if rest[1..].starts_with('\'') {
                    out.push('\'');
                    i += 2;
                    continue;
                }
                in_str = false;
            }
            i += c.len_utf8();
            continue;
        }
        if c == '\'' {
            in_str = true;
            out.push(c);
            i += 1;
            continue;
        }
        if let Some(consumed) = f(rest, out) {
            i += consumed;
        } else {
            out.push(c);
            i += c.len_utf8();
        }
    }
}

/// 解析位于片段开头的 `?N`，返回 (N, 消费字节数)。
fn parse_placeholder(rest: &str) -> Option<(usize, usize)> {
    let digits: String = rest[1..].chars().take_while(|c| c.is_ascii_digit()).collect();
    if rest.starts_with('?') && !digits.is_empty() {
        Some((digits.parse().ok()?, 1 + digits.len()))
    } else {
        None
    }
}

/// PostgreSQL：`?N` → `$N`。
pub fn numbered_to_dollar(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len() + 8);
    scan_sql(sql, &mut out, |rest, out| {
        parse_placeholder(rest).map(|(n, consumed)| {
            out.push('$');
            out.push_str(&n.to_string());
            consumed
        })
    });
    out
}

/// MySQL：占位符展开 + upsert 语法重写 + CAST 目标类型修正 + 保留字标识符加引号。
pub fn mysql_rewrite(sql: &str, params: Vec<Value>) -> (String, Vec<Value>) {
    // 先做语法层重写（字符串层面），再展开占位符并复制参数。
    let sql = mysql_upsert(&sql_normalize_ws(sql));
    // MySQL 的 CAST 不接受 TEXT 目标类型，规范形态的 CAST(x AS TEXT) 改为 AS CHAR。
    let sql = sql.replace(" AS TEXT)", " AS CHAR)");
    let sql = mysql_quote_reserved(&sql);
    let mut out = String::with_capacity(sql.len());
    let mut ordered = Vec::with_capacity(params.len());
    scan_sql(&sql, &mut out, |rest, out| {
        parse_placeholder(rest).map(|(n, consumed)| {
            out.push('?');
            // ?N 为 1 起编号；越界让后端在参数校验时报错，这里透传 Null 保持参数计数一致。
            ordered.push(params.get(n - 1).cloned().unwrap_or(Value::Null));
            consumed
        })
    });
    (out, ordered)
}

/// 本项目 schema 里撞上 MySQL 保留字的标识符：`groups` 表（8.0.2+ 窗口函数保留字）、
/// `key` 列（secrets/settings）。按小写整词匹配加反引号——DDL/改写产物中的关键字
/// （PRIMARY KEY、ON DUPLICATE KEY）为大写，大小写敏感匹配不会误伤。
pub(super) fn mysql_quote_reserved(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len() + 16);
    scan_sql(sql, &mut out, |rest, out| {
        for word in ["groups", "key"] {
            if rest.starts_with(word) {
                // 整词边界：后一个字符不是标识符成分；前一个字符由调用方保证
                // （scan_sql 逐字符推进，标识符中段不会以词首匹配到这里——
                //  额外用 out 的末字符兜底判断前界）。
                let after = rest[word.len()..].chars().next();
                let prev = out.chars().last();
                let is_word_char =
                    |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '`' || c == '.';
                if after.is_none_or(|c| !is_word_char(c))
                    && prev.is_none_or(|c| !is_word_char(c))
                {
                    out.push('`');
                    out.push_str(word);
                    out.push('`');
                    return Some(word.len());
                }
            }
        }
        None
    });
    out
}

/// 把字面量外的连续空白压成单个空格，便于 upsert 子句做跨行匹配。
/// （本项目 SQL 全部为源码内嵌常量，压缩空白不影响语义。）UTF-8 安全：按字符推进。
fn sql_normalize_ws(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len());
    let mut last_space = false;
    let mut in_str = false;
    for c in sql.chars() {
        if in_str {
            out.push(c);
            if c == '\'' {
                in_str = false;
            }
            continue;
        }
        match c {
            '\'' => {
                in_str = true;
                last_space = false;
                out.push(c);
            }
            c if c.is_whitespace() => {
                if !last_space {
                    out.push(' ');
                    last_space = true;
                }
            }
            c => {
                last_space = false;
                out.push(c);
            }
        }
    }
    out
}

/// `ON CONFLICT` upsert → MySQL `ON DUPLICATE KEY` 形态。
///
/// 依赖本项目 schema 的一个事实：各表除主键外至多一个 UNIQUE 约束，
/// 因此丢弃 conflict target（MySQL 语法不支持指定）不改变语义。
/// `x=VALUES(x)` 写法同时兼容 MySQL 8 与 MariaDB（新别名语法 MariaDB 不支持，不用）。
fn mysql_upsert(sql: &str) -> String {
    // ① ON CONFLICT(...) DO UPDATE SET a=excluded.a, ... → ON DUPLICATE KEY UPDATE a=VALUES(a), ...
    if let Some(pos) = find_ci(sql, "ON CONFLICT") {
        let head = &sql[..pos];
        let tail = &sql[pos..];
        if let Some(do_update) = find_ci(tail, "DO UPDATE SET") {
            let set_clause = &tail[do_update + "DO UPDATE SET".len()..];
            return format!(
                "{}ON DUPLICATE KEY UPDATE{}",
                head,
                replace_excluded(set_clause)
            );
        }
        if find_ci(tail, "DO NOTHING").is_some() {
            // ② ON CONFLICT [...] DO NOTHING → 去掉子句，行首 INSERT INTO → INSERT IGNORE INTO
            return replace_first_ci(head.trim_end(), "INSERT INTO", "INSERT IGNORE INTO");
        }
    }
    sql.to_string()
}

/// `excluded.col` → `VALUES(col)`（列名为字母/数字/下划线序列）。
fn replace_excluded(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 16);
    let mut rest = s;
    while let Some(pos) = find_ci(rest, "excluded.") {
        out.push_str(&rest[..pos]);
        let after = &rest[pos + "excluded.".len()..];
        let name_len: usize = after
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .map(|c| c.len_utf8())
            .sum();
        out.push_str("VALUES(");
        out.push_str(&after[..name_len]);
        out.push(')');
        rest = &after[name_len..];
    }
    out.push_str(rest);
    out
}

/// 大小写不敏感查找子串位置（SQL 关键字大小写不定）。
fn find_ci(haystack: &str, needle: &str) -> Option<usize> {
    let h = haystack.to_ascii_uppercase();
    h.find(&needle.to_ascii_uppercase())
}

fn replace_first_ci(s: &str, from: &str, to: &str) -> String {
    match find_ci(s, from) {
        Some(pos) => format!("{}{}{}", &s[..pos], to, &s[pos + from.len()..]),
        None => s.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pg_numbered() {
        assert_eq!(
            numbered_to_dollar("SELECT * FROM t WHERE a = ?1 AND b = ?2"),
            "SELECT * FROM t WHERE a = $1 AND b = $2"
        );
    }

    #[test]
    fn pg_reuse_and_two_digit() {
        assert_eq!(
            numbered_to_dollar("WHERE a LIKE ?1 OR b LIKE ?1 OR c = ?12"),
            "WHERE a LIKE $1 OR b LIKE $1 OR c = $12"
        );
    }

    #[test]
    fn pg_skips_string_literal() {
        assert_eq!(
            numbered_to_dollar("SELECT '?1 keeps' , a FROM t WHERE b = ?1"),
            "SELECT '?1 keeps' , a FROM t WHERE b = $1"
        );
        assert_eq!(
            numbered_to_dollar("SELECT 'it''s ?2' WHERE a = ?1"),
            "SELECT 'it''s ?2' WHERE a = $1"
        );
    }

    #[test]
    fn mysql_expand_reuse() {
        let (sql, params) = mysql_rewrite(
            "WHERE a LIKE ?1 OR b LIKE ?1 OR c = ?2",
            vec![Value::Text("x".into()), Value::Int(7)],
        );
        assert_eq!(sql, "WHERE a LIKE ? OR b LIKE ? OR c = ?");
        assert_eq!(params.len(), 3);
        assert!(matches!(&params[0], Value::Text(s) if s == "x"));
        assert!(matches!(&params[1], Value::Text(s) if s == "x"));
        assert!(matches!(&params[2], Value::Int(7)));
    }

    #[test]
    fn mysql_out_of_order() {
        let (sql, params) = mysql_rewrite(
            "UPDATE t SET a = ?2 WHERE id = ?1",
            vec![Value::Int(1), Value::Int(2)],
        );
        assert_eq!(sql, "UPDATE t SET a = ? WHERE id = ?");
        assert!(matches!(params[0], Value::Int(2)));
        assert!(matches!(params[1], Value::Int(1)));
    }

    #[test]
    fn mysql_upsert_do_update() {
        let (sql, _) = mysql_rewrite(
            "INSERT INTO settings(key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            vec![Value::Text("k".into()), Value::Text("v".into())],
        );
        assert_eq!(
            sql,
            "INSERT INTO settings(`key`, value) VALUES (?, ?) ON DUPLICATE KEY UPDATE value = VALUES(value)"
        );
    }

    #[test]
    fn utf8_literals_pass_through() {
        // 中文字面量在各翻译路径下必须原样保留（曾因按字节扫描损坏编码）。
        let sql = "INSERT INTO labels(name) VALUES ('官方'), ('社区') ON CONFLICT DO NOTHING";
        assert_eq!(
            numbered_to_dollar(sql),
            sql,
            "PG 路径不得改动中文字面量"
        );
        let (my, _) = mysql_rewrite(sql, vec![]);
        assert_eq!(
            my,
            "INSERT IGNORE INTO labels(name) VALUES ('官方'), ('社区')"
        );
    }

    #[test]
    fn mysql_upsert_multi_col() {
        let (sql, _) = mysql_rewrite(
            "INSERT INTO mcps(owner_id, name, manifest, updated_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(owner_id, name) DO UPDATE SET manifest = excluded.manifest, updated_at = excluded.updated_at",
            vec![Value::Int(1), Value::Text("n".into()), Value::Text("m".into()), Value::Text("t".into())],
        );
        assert_eq!(
            sql,
            "INSERT INTO mcps(owner_id, name, manifest, updated_at) VALUES (?, ?, ?, ?) \
             ON DUPLICATE KEY UPDATE manifest = VALUES(manifest), updated_at = VALUES(updated_at)"
        );
    }

    #[test]
    fn mysql_do_nothing() {
        let (sql, params) = mysql_rewrite(
            "INSERT INTO user_groups(user_id, group_id) VALUES (?1, ?2) ON CONFLICT DO NOTHING",
            vec![Value::Int(1), Value::Int(2)],
        );
        assert_eq!(sql, "INSERT IGNORE INTO user_groups(user_id, group_id) VALUES (?, ?)");
        assert_eq!(params.len(), 2);
    }

    #[test]
    fn mysql_do_nothing_insert_select() {
        let (sql, _) = mysql_rewrite(
            "INSERT INTO user_groups(user_id, group_id)
             SELECT id, group_id FROM users WHERE group_id IS NOT NULL
             ON CONFLICT DO NOTHING",
            vec![],
        );
        assert_eq!(
            sql,
            "INSERT IGNORE INTO user_groups(user_id, group_id) SELECT id, group_id FROM users WHERE group_id IS NOT NULL"
        );
    }

    #[test]
    fn mysql_quotes_reserved_words() {
        let (sql, _) = mysql_rewrite(
            "SELECT g.name FROM groups g JOIN user_groups ug ON ug.group_id = g.id WHERE key = ?1",
            vec![Value::Text("k".into())],
        );
        assert_eq!(
            sql,
            "SELECT g.name FROM `groups` g JOIN user_groups ug ON ug.group_id = g.id WHERE `key` = ?"
        );
        // 点号限定（s.key）与大写关键字（PRIMARY KEY / ON DUPLICATE KEY）不受影响
        let (sql, _) = mysql_rewrite(
            "INSERT INTO settings(key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            vec![Value::Text("k".into()), Value::Text("v".into())],
        );
        assert_eq!(
            sql,
            "INSERT INTO settings(`key`, value) VALUES (?, ?) ON DUPLICATE KEY UPDATE value = VALUES(value)"
        );
        let q = mysql_quote_reserved("SELECT u.username, s.key FROM secrets s");
        assert_eq!(q, "SELECT u.username, s.key FROM secrets s");
        let q = mysql_quote_reserved("PRIMARY KEY (user_id, group_id)");
        assert_eq!(q, "PRIMARY KEY (user_id, group_id)");
        let q = mysql_quote_reserved("'key groups inside literal' , key");
        assert_eq!(q, "'key groups inside literal' , `key`");
    }

    #[test]
    fn sqlite_passthrough() {
        let (sql, params) = rewrite(Dialect::Sqlite, "SELECT ?1", vec![Value::Int(1)]);
        assert_eq!(sql, "SELECT ?1");
        assert_eq!(params.len(), 1);
    }
}
