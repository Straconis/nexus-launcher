//! Client-facing errors. Upstream bodies are not copied through, so tokens cannot leak.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use nexus_protocol::ApiError;

#[derive(Debug)]
pub struct ApiFailure {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: &'static str,
}

impl ApiFailure {
    pub fn new(status: StatusCode, code: &'static str, message: &'static str) -> Self {
        Self {
            status,
            code,
            message,
        }
    }

    pub fn unauthorized() -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "unauthorized", "sign in required")
    }

    pub fn forbidden() -> Self {
        Self::new(StatusCode::FORBIDDEN, "forbidden", "not allowed")
    }

    pub fn not_found() -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found", "not found")
    }

    pub fn internal() -> Self {
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "the server could not complete that request",
        )
    }

    pub fn auth_unconfigured() -> Self {
        Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "auth_not_configured",
            "GitHub login is not configured on this server",
        )
    }
}

impl From<rusqlite::Error> for ApiFailure {
    fn from(_: rusqlite::Error) -> Self {
        Self::internal()
    }
}

impl IntoResponse for ApiFailure {
    fn into_response(self) -> Response {
        let body = Json(ApiError {
            error: self.code.to_string(),
            message: self.message.to_string(),
        });
        (self.status, body).into_response()
    }
}
