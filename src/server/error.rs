//! Generic API error wrapper that renders internal failures as JSON HTTP
//! responses.
//!
//! [`ApiError`] is the single error type returned by every handler. Internal
//! failures render as a generic HTTP 500 body (no chain, no filesystem paths,
//! no OS strings) while explicit request-validation failures render as HTTP
//! 400 with the supplied message. The full `anyhow` chain is logged at ERROR
//! level by the [`IntoResponse`] impl for operator post-mortem.

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;

/// A wrapper around [`anyhow::Error`] that renders internal failures as generic
/// HTTP 500 responses and explicit request validation failures as HTTP 400.
///
/// The full error chain is logged at ERROR level but internal response bodies
/// remain generic to avoid leaking file paths, OS error strings, or
/// implementation details.
pub(crate) struct ApiError {
    source: anyhow::Error,
    status: StatusCode,
    public_message: Option<String>,
}

impl ApiError {
    /// Construct a client-visible 400 response carrying `message` verbatim.
    ///
    /// Use only for messages that have been sanitised of internal detail
    /// (e.g. `"bbox × scale exceeds maximum supported block extent"`).
    pub(crate) fn bad_request(message: impl Into<String>) -> Self {
        let message = message.into();
        Self {
            source: anyhow::anyhow!(message.clone()),
            status: StatusCode::BAD_REQUEST,
            public_message: Some(message),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        log::error!("API error: {:#}", self.source);
        let message = self
            .public_message
            .unwrap_or_else(|| "An internal server error occurred.".to_string());
        let body = json!({ "error": message });
        (self.status, Json(body)).into_response()
    }
}

impl<E: Into<anyhow::Error>> From<E> for ApiError {
    fn from(e: E) -> Self {
        Self {
            source: e.into(),
            status: StatusCode::INTERNAL_SERVER_ERROR,
            public_message: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ApiError;
    use axum::http::StatusCode;
    use axum::response::IntoResponse as _;

    #[test]
    fn bad_request_response_uses_request_error_status() {
        let response = ApiError::bad_request("Invalid POI source mode").into_response();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
