//! 数据库访问层：统一 facade，按连接串在运行时选择 SQLite / PostgreSQL / MySQL。
//!
//! 源码中的 SQL 以「SQLite ∩ PostgreSQL 通用方言 + `?N` 编号占位」为规范形态书写，
//! 各后端差异（占位符风格、upsert 语法）由 [`translate`] 层在执行前重写，
//! 列值经 [`decode`] 归一为 [`Value`] 五型（整数一律 i64，布尔按 0/1 存取），
//! [`Row::get`] 做与 rusqlite 同级的宽松转换，调用侧无须感知后端类型系统。

mod decode;
pub mod schema;
mod translate;

use std::path::Path;

use anyhow::{Context, Result};

/// SQL 方言（由连接串决定）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dialect {
    Sqlite,
    Pg,
    MySql,
}

/// 统一参数/列值。
#[derive(Clone, Debug)]
pub enum Value {
    Null,
    Int(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

impl Value {
    fn type_name(&self) -> &'static str {
        match self {
            Value::Null => "NULL",
            Value::Int(_) => "INTEGER",
            Value::Real(_) => "REAL",
            Value::Text(_) => "TEXT",
            Value::Blob(_) => "BLOB",
        }
    }
}

/// 参数绑定：把 Rust 侧常用类型（含引用与 Option）转成 [`Value`]。
pub trait ToValue {
    fn to_value(&self) -> Value;
}

macro_rules! to_value_int {
    ($($t:ty),*) => {
        $(impl ToValue for $t {
            fn to_value(&self) -> Value { Value::Int(*self as i64) }
        })*
    };
}
to_value_int!(i8, i16, i32, i64, u8, u16, u32, u64, usize, isize);

impl ToValue for bool {
    fn to_value(&self) -> Value {
        Value::Int(*self as i64)
    }
}
impl ToValue for f64 {
    fn to_value(&self) -> Value {
        Value::Real(*self)
    }
}
impl ToValue for str {
    fn to_value(&self) -> Value {
        Value::Text(self.to_string())
    }
}
impl ToValue for String {
    fn to_value(&self) -> Value {
        Value::Text(self.clone())
    }
}
impl ToValue for [u8] {
    fn to_value(&self) -> Value {
        Value::Blob(self.to_vec())
    }
}
impl ToValue for Vec<u8> {
    fn to_value(&self) -> Value {
        Value::Blob(self.clone())
    }
}
impl ToValue for Value {
    fn to_value(&self) -> Value {
        self.clone()
    }
}
impl<T: ToValue + ?Sized> ToValue for &T {
    fn to_value(&self) -> Value {
        (**self).to_value()
    }
}
impl<T: ToValue> ToValue for Option<T> {
    fn to_value(&self) -> Value {
        match self {
            Some(v) => v.to_value(),
            None => Value::Null,
        }
    }
}

/// 参数列表构造宏，替代 `rusqlite::params![]`。
macro_rules! db_params {
    () => { Vec::<crate::server::db::Value>::new() };
    ($($v:expr),+ $(,)?) => {
        vec![$(crate::server::db::ToValue::to_value(&$v)),+]
    };
}
pub(crate) use db_params;

/// 列值提取：与 rusqlite `FromSql` 同级的宽松转换。
pub trait FromValue: Sized {
    fn from_value(v: &Value) -> Result<Self, DbError>;
}

fn type_err(want: &str, got: &Value) -> DbError {
    DbError::Other(anyhow::anyhow!("列类型不匹配：期望 {want}，实际 {}", got.type_name()))
}

impl FromValue for i64 {
    fn from_value(v: &Value) -> Result<Self, DbError> {
        match v {
            Value::Int(i) => Ok(*i),
            other => Err(type_err("INTEGER", other)),
        }
    }
}
impl FromValue for u64 {
    fn from_value(v: &Value) -> Result<Self, DbError> {
        i64::from_value(v).map(|i| i as u64)
    }
}
impl FromValue for i32 {
    fn from_value(v: &Value) -> Result<Self, DbError> {
        i64::from_value(v).map(|i| i as i32)
    }
}
impl FromValue for usize {
    fn from_value(v: &Value) -> Result<Self, DbError> {
        i64::from_value(v).map(|i| i as usize)
    }
}
impl FromValue for bool {
    fn from_value(v: &Value) -> Result<Self, DbError> {
        i64::from_value(v).map(|i| i != 0)
    }
}
impl FromValue for f64 {
    fn from_value(v: &Value) -> Result<Self, DbError> {
        match v {
            Value::Real(f) => Ok(*f),
            Value::Int(i) => Ok(*i as f64),
            other => Err(type_err("REAL", other)),
        }
    }
}
impl FromValue for String {
    fn from_value(v: &Value) -> Result<Self, DbError> {
        match v {
            Value::Text(s) => Ok(s.clone()),
            // MySQL 的 *_bin 排序规则会在协议层给文本列打 BINARY 标记，驱动一律
            // 报 BLOB；只要是合法 UTF-8 就按文本收下（本项目文本列均为 utf8mb4）。
            Value::Blob(b) => String::from_utf8(b.clone())
                .map_err(|_| DbError::Other(anyhow::anyhow!("BLOB 列不是合法 UTF-8 文本"))),
            other => Err(type_err("TEXT", other)),
        }
    }
}
impl FromValue for Vec<u8> {
    fn from_value(v: &Value) -> Result<Self, DbError> {
        match v {
            Value::Blob(b) => Ok(b.clone()),
            Value::Text(s) => Ok(s.as_bytes().to_vec()),
            other => Err(type_err("BLOB", other)),
        }
    }
}
impl<T: FromValue> FromValue for Option<T> {
    fn from_value(v: &Value) -> Result<Self, DbError> {
        match v {
            Value::Null => Ok(None),
            other => T::from_value(other).map(Some),
        }
    }
}

/// 一行查询结果（列值已归一为 [`Value`]）。
pub struct Row(Vec<Value>);

impl Row {
    pub fn get<T: FromValue>(&self, i: usize) -> Result<T, DbError> {
        let v = self
            .0
            .get(i)
            .ok_or_else(|| DbError::Other(anyhow::anyhow!("列下标越界: {i}")))?;
        T::from_value(v)
    }
}

/// 数据层错误。`Unique` 单列出来供上层把并发唯一冲突映射为 409。
#[derive(Debug)]
pub enum DbError {
    /// 唯一约束冲突（check-then-insert 在连接池并发下的兜底）。
    Unique,
    Other(anyhow::Error),
}

impl std::fmt::Display for DbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DbError::Unique => write!(f, "唯一约束冲突"),
            DbError::Other(e) => write!(f, "{e}"),
        }
    }
}

impl From<sqlx::Error> for DbError {
    fn from(e: sqlx::Error) -> Self {
        if let sqlx::Error::Database(ref d) = e
            && d.is_unique_violation()
        {
            return DbError::Unique;
        }
        DbError::Other(e.into())
    }
}

// Display + Debug 齐备即可作为标准错误；anyhow 的 blanket From 负责 `?` 上抛与 .context()。
impl std::error::Error for DbError {}

/// 全局数据库句柄（内部为各后端连接池，Clone 即引用计数）。
#[derive(Clone)]
pub struct Db {
    pool: Pool,
    pub dialect: Dialect,
}

#[derive(Clone)]
enum Pool {
    Sqlite(sqlx::SqlitePool),
    Pg(sqlx::PgPool),
    MySql(sqlx::MySqlPool),
}

/// 进行中的事务。Drop 未 commit 即回滚（sqlx 语义）。
pub enum DbTx {
    Sqlite(sqlx::Transaction<'static, sqlx::Sqlite>),
    Pg(sqlx::Transaction<'static, sqlx::Postgres>),
    MySql(sqlx::Transaction<'static, sqlx::MySql>),
}

/// 连接池大小：默认 SQLite 4 / 网络型数据库 10，可用 `TRISKELION_DB_MAX_CONNS` 覆盖。
fn max_conns(default: u32) -> u32 {
    std::env::var("TRISKELION_DB_MAX_CONNS")
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .filter(|&n| n > 0)
        .unwrap_or(default)
}

impl Db {
    /// 按连接串建池。`url` 为空 → 默认 `<data_dir>/hub.db`（零配置单机部署，行为与历史一致）。
    pub async fn connect(url: Option<&str>, data_dir: &Path) -> Result<Db> {
        let url = url.map(str::trim).filter(|s| !s.is_empty());
        match url {
            None => Self::connect_sqlite(&data_dir.join("hub.db")).await,
            Some(u) if u.starts_with("sqlite:") => {
                // 接受 sqlite:/path、sqlite:///path、sqlite://path 三种写法。
                let path = u.trim_start_matches("sqlite:").trim_start_matches("//");
                Self::connect_sqlite(Path::new(path)).await
            }
            Some(u) if u.starts_with("postgres://") || u.starts_with("postgresql://") => {
                Self::connect_pg(u).await
            }
            Some(u) if u.starts_with("mysql://") || u.starts_with("mariadb://") => {
                Self::connect_mysql(u).await
            }
            Some(u) => anyhow::bail!("无法识别的数据库连接串: {u}"),
        }
    }

    async fn connect_sqlite(path: &Path) -> Result<Db> {
        use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
        let opts = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .foreign_keys(true)
            .busy_timeout(std::time::Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .max_connections(max_conns(4))
            .connect_with(opts)
            .await
            .with_context(|| format!("打开数据库 {}", path.display()))?;
        Ok(Db {
            pool: Pool::Sqlite(pool),
            dialect: Dialect::Sqlite,
        })
    }

    async fn connect_pg(url: &str) -> Result<Db> {
        use sqlx::postgres::PgPoolOptions;
        let pool = PgPoolOptions::new()
            .max_connections(max_conns(10))
            .connect(url)
            .await
            // 错误信息不回显连接串，避免口令泄露进日志。
            .context("连接 PostgreSQL 失败（检查 TRISKELION_DATABASE_URL）")?;
        Ok(Db {
            pool: Pool::Pg(pool),
            dialect: Dialect::Pg,
        })
    }

    async fn connect_mysql(url: &str) -> Result<Db> {
        use sqlx::mysql::MySqlPoolOptions;
        // mariadb:// 前缀转成 sqlx 认识的 mysql://（协议同源）。
        let url = url.replacen("mariadb://", "mysql://", 1);
        let pool = MySqlPoolOptions::new()
            .max_connections(max_conns(10))
            .connect(&url)
            .await
            .context("连接 MySQL 失败（检查 TRISKELION_DATABASE_URL）")?;
        Ok(Db {
            pool: Pool::MySql(pool),
            dialect: Dialect::MySql,
        })
    }

    /// 执行写语句，返回受影响行数。
    pub async fn execute(&self, sql: &str, params: Vec<Value>) -> Result<u64, DbError> {
        let (sql, params) = translate::rewrite(self.dialect, sql, params);
        match &self.pool {
            Pool::Sqlite(p) => sqlite_execute(p, &sql, &params).await,
            Pool::Pg(p) => pg_execute(p, &sql, &params).await,
            Pool::MySql(p) => mysql_execute(p, &sql, &params).await.map(|r| r.0),
        }
    }

    /// 执行 INSERT 并返回自增主键 id（跨后端替代 `last_insert_rowid()`）。
    pub async fn insert_id(&self, sql: &str, params: Vec<Value>) -> Result<i64, DbError> {
        match self.dialect {
            Dialect::Sqlite | Dialect::Pg => {
                let sql = format!("{sql} RETURNING id");
                self.query_row(&sql, params).await?.get(0)
            }
            Dialect::MySql => {
                let (sql, params) = translate::rewrite(self.dialect, sql, params);
                match &self.pool {
                    Pool::MySql(p) => Ok(mysql_execute(p, &sql, &params).await?.1 as i64),
                    _ => unreachable!("dialect 与连接池不一致"),
                }
            }
        }
    }

    /// 原样执行（不做占位符翻译、不带参数），供 DDL/迁移语句使用。
    pub async fn exec_raw(&self, sql: &str) -> Result<(), DbError> {
        match &self.pool {
            Pool::Sqlite(p) => {
                sqlx::raw_sql(sql).execute(p).await?;
            }
            Pool::Pg(p) => {
                sqlx::raw_sql(sql).execute(p).await?;
            }
            Pool::MySql(p) => {
                sqlx::raw_sql(sql).execute(p).await?;
            }
        }
        Ok(())
    }

    pub async fn query_all(&self, sql: &str, params: Vec<Value>) -> Result<Vec<Row>, DbError> {
        let (sql, params) = translate::rewrite(self.dialect, sql, params);
        match &self.pool {
            Pool::Sqlite(p) => sqlite_query(p, &sql, &params).await,
            Pool::Pg(p) => pg_query(p, &sql, &params).await,
            Pool::MySql(p) => mysql_query(p, &sql, &params).await,
        }
    }

    /// 期望恰有一行；查不到视为内部错误（与 rusqlite `query_row` 一致，上层通常已先校验存在性）。
    pub async fn query_row(&self, sql: &str, params: Vec<Value>) -> Result<Row, DbError> {
        self.query_opt(sql, params)
            .await?
            .ok_or_else(|| DbError::Other(anyhow::anyhow!("查询未返回任何行")))
    }

    /// 取第一行（可空），替代 rusqlite 的 `.optional()`。
    pub async fn query_opt(&self, sql: &str, params: Vec<Value>) -> Result<Option<Row>, DbError> {
        Ok(self.query_all(sql, params).await?.into_iter().next())
    }

    /// 逐行映射，保住 rusqlite `query_map` 的闭包写法。
    pub async fn query_map<T>(
        &self,
        sql: &str,
        params: Vec<Value>,
        mut f: impl FnMut(&Row) -> Result<T, DbError>,
    ) -> Result<Vec<T>, DbError> {
        self.query_all(sql, params).await?.iter().map(&mut f).collect()
    }

    pub async fn begin(&self) -> Result<DbTx, DbError> {
        match &self.pool {
            Pool::Sqlite(p) => Ok(DbTx::Sqlite(p.begin().await?)),
            Pool::Pg(p) => Ok(DbTx::Pg(p.begin().await?)),
            Pool::MySql(p) => Ok(DbTx::MySql(p.begin().await?)),
        }
    }
}

impl DbTx {
    fn dialect(&self) -> Dialect {
        match self {
            DbTx::Sqlite(_) => Dialect::Sqlite,
            DbTx::Pg(_) => Dialect::Pg,
            DbTx::MySql(_) => Dialect::MySql,
        }
    }

    pub async fn execute(&mut self, sql: &str, params: Vec<Value>) -> Result<u64, DbError> {
        let (sql, params) = translate::rewrite(self.dialect(), sql, params);
        match self {
            DbTx::Sqlite(tx) => sqlite_execute(&mut **tx, &sql, &params).await,
            DbTx::Pg(tx) => pg_execute(&mut **tx, &sql, &params).await,
            DbTx::MySql(tx) => mysql_execute(&mut **tx, &sql, &params).await.map(|r| r.0),
        }
    }

    pub async fn insert_id(&mut self, sql: &str, params: Vec<Value>) -> Result<i64, DbError> {
        match self.dialect() {
            Dialect::Sqlite | Dialect::Pg => {
                let sql = format!("{sql} RETURNING id");
                self.query_row(&sql, params).await?.get(0)
            }
            Dialect::MySql => {
                let (sql, params) = translate::rewrite(self.dialect(), sql, params);
                match self {
                    DbTx::MySql(tx) => {
                        Ok(mysql_execute(&mut **tx, &sql, &params).await?.1 as i64)
                    }
                    _ => unreachable!("dialect 与事务后端不一致"),
                }
            }
        }
    }

    pub async fn query_all(&mut self, sql: &str, params: Vec<Value>) -> Result<Vec<Row>, DbError> {
        let (sql, params) = translate::rewrite(self.dialect(), sql, params);
        match self {
            DbTx::Sqlite(tx) => sqlite_query(&mut **tx, &sql, &params).await,
            DbTx::Pg(tx) => pg_query(&mut **tx, &sql, &params).await,
            DbTx::MySql(tx) => mysql_query(&mut **tx, &sql, &params).await,
        }
    }

    pub async fn query_row(&mut self, sql: &str, params: Vec<Value>) -> Result<Row, DbError> {
        self.query_opt(sql, params)
            .await?
            .ok_or_else(|| DbError::Other(anyhow::anyhow!("查询未返回任何行")))
    }

    pub async fn query_opt(&mut self, sql: &str, params: Vec<Value>) -> Result<Option<Row>, DbError> {
        Ok(self.query_all(sql, params).await?.into_iter().next())
    }

    pub async fn query_map<T>(
        &mut self,
        sql: &str,
        params: Vec<Value>,
        mut f: impl FnMut(&Row) -> Result<T, DbError>,
    ) -> Result<Vec<T>, DbError> {
        self.query_all(sql, params).await?.iter().map(&mut f).collect()
    }

    pub async fn commit(self) -> Result<(), DbError> {
        match self {
            DbTx::Sqlite(tx) => tx.commit().await?,
            DbTx::Pg(tx) => tx.commit().await?,
            DbTx::MySql(tx) => tx.commit().await?,
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// SQLite 后端执行核心（Db 与 DbTx 共用，Executor 泛型吃 &Pool / &mut Connection）。

fn bind_sqlite<'q>(
    q: sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments<'q>>,
    v: &'q Value,
) -> sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments<'q>> {
    match v {
        Value::Null => q.bind(None::<i64>),
        Value::Int(i) => q.bind(*i),
        Value::Real(f) => q.bind(*f),
        Value::Text(s) => q.bind(s.as_str()),
        Value::Blob(b) => q.bind(b.as_slice()),
    }
}

async fn sqlite_execute<'e, E>(ex: E, sql: &str, params: &[Value]) -> Result<u64, DbError>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    let mut q = sqlx::query(sql);
    for p in params {
        q = bind_sqlite(q, p);
    }
    Ok(q.execute(ex).await?.rows_affected())
}

async fn sqlite_query<'e, E>(ex: E, sql: &str, params: &[Value]) -> Result<Vec<Row>, DbError>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    let mut q = sqlx::query(sql);
    for p in params {
        q = bind_sqlite(q, p);
    }
    let rows = q.fetch_all(ex).await?;
    rows.iter().map(decode::row_from_sqlite).collect()
}

// ---------------------------------------------------------------------------
// PostgreSQL 后端执行核心。

fn bind_pg<'q>(
    q: sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments>,
    v: &'q Value,
) -> sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments> {
    match v {
        // NULL 以 text 类型占位：本项目可空参数均出现在文本比较位（category/tag 过滤）。
        Value::Null => q.bind(None::<String>),
        Value::Int(i) => q.bind(*i),
        Value::Real(f) => q.bind(*f),
        Value::Text(s) => q.bind(s.as_str()),
        Value::Blob(b) => q.bind(b.as_slice()),
    }
}

async fn pg_execute<'e, E>(ex: E, sql: &str, params: &[Value]) -> Result<u64, DbError>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let mut q = sqlx::query(sql);
    for p in params {
        q = bind_pg(q, p);
    }
    Ok(q.execute(ex).await?.rows_affected())
}

async fn pg_query<'e, E>(ex: E, sql: &str, params: &[Value]) -> Result<Vec<Row>, DbError>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let mut q = sqlx::query(sql);
    for p in params {
        q = bind_pg(q, p);
    }
    let rows = q.fetch_all(ex).await?;
    rows.iter().map(decode::row_from_pg).collect()
}

// ---------------------------------------------------------------------------
// MySQL 后端执行核心。

fn bind_mysql<'q>(
    q: sqlx::query::Query<'q, sqlx::MySql, sqlx::mysql::MySqlArguments>,
    v: &'q Value,
) -> sqlx::query::Query<'q, sqlx::MySql, sqlx::mysql::MySqlArguments> {
    match v {
        Value::Null => q.bind(None::<String>),
        Value::Int(i) => q.bind(*i),
        Value::Real(f) => q.bind(*f),
        Value::Text(s) => q.bind(s.as_str()),
        Value::Blob(b) => q.bind(b.as_slice()),
    }
}

/// 返回 (rows_affected, last_insert_id)。
async fn mysql_execute<'e, E>(ex: E, sql: &str, params: &[Value]) -> Result<(u64, u64), DbError>
where
    E: sqlx::Executor<'e, Database = sqlx::MySql>,
{
    let mut q = sqlx::query(sql);
    for p in params {
        q = bind_mysql(q, p);
    }
    let res = q.execute(ex).await?;
    Ok((res.rows_affected(), res.last_insert_id()))
}

async fn mysql_query<'e, E>(ex: E, sql: &str, params: &[Value]) -> Result<Vec<Row>, DbError>
where
    E: sqlx::Executor<'e, Database = sqlx::MySql>,
{
    let mut q = sqlx::query(sql);
    for p in params {
        q = bind_mysql(q, p);
    }
    let rows = q.fetch_all(ex).await?;
    rows.iter().map(decode::row_from_mysql).collect()
}
