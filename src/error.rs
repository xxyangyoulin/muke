use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;

#[derive(Debug)]
pub enum Error {
    Http(StatusCode, String),
    Database(sqlx::Error),
    Io(std::io::Error),
    Internal(String),
}
pub type Result<T> = std::result::Result<T, Error>;
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Http(_, message) | Self::Internal(message) => write!(f, "{message}"),
            Self::Database(error) => write!(f, "{error}"),
            Self::Io(error) => write!(f, "{error}"),
        }
    }
}
impl std::error::Error for Error {}
impl From<sqlx::Error> for Error {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error)
    }
}
impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}
impl Error {
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::Http(StatusCode::UNPROCESSABLE_ENTITY, message.into())
    }
    pub fn unauthorized() -> Self {
        Self::Http(StatusCode::UNAUTHORIZED, "请先登录管理后台".into())
    }
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::Http(StatusCode::NOT_FOUND, message.into())
    }
}
impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            Self::Http(status, message) => (status, message),
            error => {
                tracing::error!(error=%error,"request failed");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "操作失败，请查看服务日志".into(),
                )
            }
        };
        (status, Json(json!({"error":message}))).into_response()
    }
}
