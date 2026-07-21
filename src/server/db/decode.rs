//! 行解码：把各后端的原生行按「值的运行时类型」归一为 `Vec<Value>`。
//!
//! 统一在此处做类型归一（而非调用侧 try_get 具体类型），
//! 是为了复刻 rusqlite 的宽松取值语义并屏蔽后端间的类型系统差异
//! （PG 的 INT2/4/8、MySQL 的 TINYINT/unsigned 等都折叠到 i64）。

use anyhow::anyhow;
use sqlx::{Row as _, TypeInfo as _, ValueRef as _};

use super::{DbError, Row, Value};

pub(super) fn row_from_sqlite(r: &sqlx::sqlite::SqliteRow) -> Result<Row, DbError> {
    let mut out = Vec::with_capacity(r.len());
    for i in 0..r.len() {
        let raw = r
            .try_get_raw(i)
            .map_err(|e| DbError::Other(anyhow!("读取第 {i} 列失败: {e}")))?;
        if raw.is_null() {
            out.push(Value::Null);
            continue;
        }
        // SQLite 的 ValueRef 类型即存储类别（runtime storage class）。
        let ty = raw.type_info().name().to_string();
        let v = match ty.as_str() {
            "INTEGER" | "BOOLEAN" => Value::Int(decode_as::<i64>(raw, i)?),
            "REAL" | "NUMERIC" => Value::Real(decode_as::<f64>(raw, i)?),
            "TEXT" | "DATETIME" | "DATE" | "TIME" => Value::Text(decode_as::<String>(raw, i)?),
            "BLOB" => Value::Blob(decode_as::<Vec<u8>>(raw, i)?),
            other => {
                return Err(DbError::Other(anyhow!("第 {i} 列未知的 SQLite 类型: {other}")));
            }
        };
        out.push(v);
    }
    Ok(Row(out))
}

fn decode_as<'r, T: sqlx::Decode<'r, sqlx::Sqlite>>(
    raw: sqlx::sqlite::SqliteValueRef<'r>,
    i: usize,
) -> Result<T, DbError> {
    T::decode(raw).map_err(|e| DbError::Other(anyhow!("解码第 {i} 列失败: {e}")))
}

pub(super) fn row_from_pg(r: &sqlx::postgres::PgRow) -> Result<Row, DbError> {
    let mut out = Vec::with_capacity(r.len());
    for i in 0..r.len() {
        let raw = r
            .try_get_raw(i)
            .map_err(|e| DbError::Other(anyhow!("读取第 {i} 列失败: {e}")))?;
        if raw.is_null() {
            out.push(Value::Null);
            continue;
        }
        // sqlx 对 PG 的整数解码严格按宽度匹配，这里按列声明类型分派后折叠到 i64。
        let ty = raw.type_info().name().to_string();
        let v = match ty.as_str() {
            "INT8" => Value::Int(pg_decode_as::<i64>(raw, i)?),
            "INT4" => Value::Int(pg_decode_as::<i32>(raw, i)? as i64),
            "INT2" => Value::Int(pg_decode_as::<i16>(raw, i)? as i64),
            "BOOL" => Value::Int(pg_decode_as::<bool>(raw, i)? as i64),
            "FLOAT8" => Value::Real(pg_decode_as::<f64>(raw, i)?),
            "FLOAT4" => Value::Real(pg_decode_as::<f32>(raw, i)? as f64),
            "TEXT" | "VARCHAR" | "CHAR" | "BPCHAR" | "NAME" => {
                Value::Text(pg_decode_as::<String>(raw, i)?)
            }
            "BYTEA" => Value::Blob(pg_decode_as::<Vec<u8>>(raw, i)?),
            other => {
                return Err(DbError::Other(anyhow!(
                    "第 {i} 列未知的 PostgreSQL 类型: {other}（如为聚合结果请在 SQL 里 CAST 到 BIGINT/TEXT）"
                )));
            }
        };
        out.push(v);
    }
    Ok(Row(out))
}

fn pg_decode_as<'r, T: sqlx::Decode<'r, sqlx::Postgres>>(
    raw: sqlx::postgres::PgValueRef<'r>,
    i: usize,
) -> Result<T, DbError> {
    T::decode(raw).map_err(|e| DbError::Other(anyhow!("解码第 {i} 列失败: {e}")))
}

pub(super) fn row_from_mysql(r: &sqlx::mysql::MySqlRow) -> Result<Row, DbError> {
    let mut out = Vec::with_capacity(r.len());
    for i in 0..r.len() {
        let raw = r
            .try_get_raw(i)
            .map_err(|e| DbError::Other(anyhow!("读取第 {i} 列失败: {e}")))?;
        if raw.is_null() {
            out.push(Value::Null);
            continue;
        }
        // 整数按声明宽度/符号分派后折叠到 i64（本项目 DDL 只有 BIGINT，
        // 其余宽度来自表达式结果，如 EXISTS→BIGINT、布尔→TINYINT）。
        let ty = raw.type_info().name().to_string();
        let v = match ty.as_str() {
            "BIGINT" => Value::Int(my_decode_as::<i64>(raw, i)?),
            "INT" | "MEDIUMINT" => Value::Int(my_decode_as::<i32>(raw, i)? as i64),
            "SMALLINT" => Value::Int(my_decode_as::<i16>(raw, i)? as i64),
            "TINYINT" | "BOOLEAN" => Value::Int(my_decode_as::<i8>(raw, i)? as i64),
            "BIGINT UNSIGNED" => Value::Int(my_decode_as::<u64>(raw, i)? as i64),
            "INT UNSIGNED" | "MEDIUMINT UNSIGNED" => {
                Value::Int(my_decode_as::<u32>(raw, i)? as i64)
            }
            "SMALLINT UNSIGNED" => Value::Int(my_decode_as::<u16>(raw, i)? as i64),
            "TINYINT UNSIGNED" => Value::Int(my_decode_as::<u8>(raw, i)? as i64),
            "FLOAT" => Value::Real(my_decode_as::<f32>(raw, i)? as f64),
            "DOUBLE" => Value::Real(my_decode_as::<f64>(raw, i)?),
            "VARCHAR" | "TEXT" | "CHAR" | "TINYTEXT" | "MEDIUMTEXT" | "LONGTEXT" | "ENUM" => {
                Value::Text(my_decode_as::<String>(raw, i)?)
            }
            "BLOB" | "TINYBLOB" | "MEDIUMBLOB" | "LONGBLOB" | "VARBINARY" | "BINARY" => {
                Value::Blob(my_decode_as::<Vec<u8>>(raw, i)?)
            }
            other => {
                return Err(DbError::Other(anyhow!(
                    "第 {i} 列未知的 MySQL 类型: {other}（如为聚合结果请在 SQL 里 CAST）"
                )));
            }
        };
        out.push(v);
    }
    Ok(Row(out))
}

fn my_decode_as<'r, T: sqlx::Decode<'r, sqlx::MySql>>(
    raw: sqlx::mysql::MySqlValueRef<'r>,
    i: usize,
) -> Result<T, DbError> {
    T::decode(raw).map_err(|e| DbError::Other(anyhow!("解码第 {i} 列失败: {e}")))
}
