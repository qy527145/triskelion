//! 统一 API 错误：转成 `(StatusCode, Json<ErrorResp>)`。

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::shared::ErrorResp;

pub struct ApiError {
    pub status: StatusCode,
    pub message: String,
}

impl ApiError {
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
    pub fn bad_request(m: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, m)
    }
    pub fn unauthorized(m: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, m)
    }
    pub fn not_found(m: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, m)
    }
    pub fn conflict(m: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, m)
    }
    pub fn internal(m: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, m)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ErrorResp {
                error: self.message,
            }),
        )
            .into_response()
    }
}

/// 任意 anyhow 错误归为 500（内部 bug），并打印到 stderr。
impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        eprintln!("internal error: {e:#}");
        ApiError::internal(format!("internal error: {e}"))
    }
}

/// 数据层错误：唯一约束冲突映射 409（连接池并发下 check-then-insert 的兜底），
/// 其余归为 500。统一在此打日志，调用侧可直接 `?` 上抛。
impl From<super::db::DbError> for ApiError {
    fn from(e: super::db::DbError) -> Self {
        eprintln!("db error: {e}");
        match e {
            super::db::DbError::Unique => ApiError::conflict("记录已存在（并发写入冲突）"),
            super::db::DbError::Other(_) => ApiError::internal(format!("数据库错误: {e}")),
        }
    }
}
