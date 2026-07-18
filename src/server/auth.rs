//! Optional shared-secret API authentication (SEC-001 / SEC-007).
//!
//! When the operator supplies `--api-key` (or `OSM_TO_BEDROCK_API_KEY`),
//! [`require_api_key`] becomes an Axum middleware that gates every mutating
//! route plus `/download`, `/status`, and `/cache/areas` behind a constant-time
//! key comparison. `/health` stays public so liveness probes work without
//! credentials.
//!
//! When no key is configured, [`AuthState::api_key`] is `None` and the
//! middleware is a no-op — preserving the historical loopback-dev workflow.
//!
//! [`enforce_safe_bind`] is a startup guard that refuses to bind a
//! non-loopback interface without a key (unless explicitly overridden via
//! `OSM_TO_BEDROCK_ALLOW_INSECURE_BIND=1`).

use std::sync::Arc;

use anyhow::Result;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::Response;

/// Shared-secret API key used by [`require_api_key`].
///
/// When `None` (no `--api-key` flag and no `OSM_TO_BEDROCK_API_KEY` env var),
/// the server runs unauthenticated — the historical loopback-dev behaviour.
/// When `Some`, mutating routes and `/download`, `/status`, `/cache` require
/// an `Authorization: Bearer <key>` (or `X-API-Key: <key>`) header.
///
/// Wrapped in `Arc` so the auth state clones cheaply per request.
#[derive(Clone, Default)]
pub(crate) struct AuthState {
    pub(crate) api_key: Option<Arc<String>>,
}

/// Constant-time byte-slice equality comparison.
///
/// Avoids timing side-channels when comparing the presented API key against
/// the expected one. Length-leakage is acceptable (we early-return on length
/// mismatch) — the key length is not secret.
pub(crate) fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Extract the presented shared-secret from request headers.
///
/// Accepts either `Authorization: Bearer <key>` (standard) or
/// `X-API-Key: <key>` (convenient for browser-driven clients that cannot
/// easily set `Authorization`). Returns the bytes that should be compared
/// against the configured key.
pub(crate) fn extract_presented_key(headers: &axum::http::HeaderMap) -> Option<&[u8]> {
    if let Some(v) = headers.get("X-API-Key") {
        return Some(v.as_bytes());
    }
    let auth = headers.get(axum::http::header::AUTHORIZATION)?;
    let s = auth.to_str().ok()?;
    // Accept "Bearer <key>", "bearer <key>", or a bare key.
    if let Some(rest) = s
        .strip_prefix("Bearer ")
        .or_else(|| s.strip_prefix("bearer "))
    {
        Some(rest.trim().as_bytes())
    } else {
        Some(s.trim().as_bytes())
    }
}

/// Axum middleware enforcing the optional shared-secret API key.
///
/// When [`AuthState::api_key`] is `None` the request passes through unchanged
/// (preserving the unauthenticated loopback-dev workflow). When set, requests
/// must present the key in the `Authorization` (or `X-API-Key`) header.
/// Mismatched or missing credentials get HTTP 401.
pub(crate) async fn require_api_key(
    State(auth): State<AuthState>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let Some(expected) = auth.api_key.as_deref() else {
        return Ok(next.run(request).await);
    };
    match extract_presented_key(request.headers()) {
        Some(presented) if constant_time_eq(presented, expected.as_bytes()) => {
            Ok(next.run(request).await)
        }
        _ => Err(StatusCode::UNAUTHORIZED),
    }
}

/// Classify a bind host as loopback (safe to expose without auth) or not.
///
/// Returns `true` for the literal strings `127.0.0.1`, `::1`, `localhost`,
/// and any IPv4 host whose first octet is `127` (e.g. `127.0.0.2`,
/// `127.1.2.3`). Everything else (including `0.0.0.0`) returns `false`.
pub(crate) fn is_loopback_host(host: &str) -> bool {
    let host = host.trim();
    if host.is_empty() {
        return true; // unspecified → default-bind, treat as loopback
    }
    if host == "127.0.0.1" || host == "::1" || host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    // 127.x.x.x — loopback IPv4 block.
    host.split('.').next().is_some_and(|first_octet| {
        first_octet.len() <= 3
            && first_octet.bytes().all(|b| b.is_ascii_digit())
            && first_octet == "127"
    })
}

/// Resolve the API key from the `--api-key` flag, falling back to the
/// `OSM_TO_BEDROCK_API_KEY` env var. Returns `None` when neither is set
/// (or when the resolved value is empty).
pub(crate) fn resolve_api_key(flag_value: Option<String>) -> Option<String> {
    flag_value.filter(|s| !s.is_empty()).or_else(|| {
        std::env::var("OSM_TO_BEDROCK_API_KEY")
            .ok()
            .filter(|s| !s.is_empty())
    })
}

/// Fail-safe bind guard: refuse to start when binding a non-loopback host
/// without an API key configured, unless explicitly overridden.
///
/// Operators who genuinely want unauthenticated exposure on a public interface
/// can set `OSM_TO_BEDROCK_ALLOW_INSECURE_BIND=1` to acknowledge the risk.
pub(crate) fn enforce_safe_bind(host: &str, api_key: &Option<String>) -> Result<()> {
    if is_loopback_host(host) || api_key.is_some() {
        return Ok(());
    }
    let allow = std::env::var("OSM_TO_BEDROCK_ALLOW_INSECURE_BIND")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("yes"))
        .unwrap_or(false);
    if allow {
        log::warn!(
            "Binding non-loopback host '{host}' without an API key — INSECURE. \
             Anyone reachable can submit conversions and read any job's output. \
             Set --api-key / OSM_TO_BEDROCK_API_KEY to require authentication."
        );
        return Ok(());
    }
    anyhow::bail!(
        "Refusing to bind non-loopback host '{host}' without an API key. \
         Set --api-key / OSM_TO_BEDROCK_API_KEY to enable authentication, \
         or set OSM_TO_BEDROCK_ALLOW_INSECURE_BIND=1 to acknowledge the risk."
    )
}

#[cfg(test)]
mod tests {
    use super::{
        AuthState, constant_time_eq, enforce_safe_bind, extract_presented_key, is_loopback_host,
        resolve_api_key,
    };
    use std::sync::{Arc, Mutex};

    /// Serialize env-mutating tests so `OSM_TO_BEDROCK_*` var reads/writes
    /// don't race each other under cargo's default parallel runner.
    static ENV_GUARD: Mutex<()> = Mutex::new(());

    #[test]
    fn constant_time_eq_matches_identical_bytes() {
        assert!(constant_time_eq(b"abcdef", b"abcdef"));
    }

    #[test]
    fn constant_time_eq_rejects_different_bytes() {
        assert!(!constant_time_eq(b"abcdef", b"abcdeg"));
    }

    #[test]
    fn constant_time_eq_rejects_different_lengths() {
        assert!(!constant_time_eq(b"abc", b"abcd"));
        assert!(!constant_time_eq(b"abcd", b"abc"));
    }

    #[test]
    fn constant_time_eq_accepts_empty_slices() {
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn is_loopback_host_recognizes_loopback_addresses() {
        assert!(is_loopback_host("127.0.0.1"));
        assert!(is_loopback_host("127.0.0.2"));
        assert!(is_loopback_host("127.1.2.3"));
        assert!(is_loopback_host("::1"));
        assert!(is_loopback_host("localhost"));
        assert!(is_loopback_host("Localhost"));
        assert!(is_loopback_host("")); // unspecified → default-bind
    }

    #[test]
    fn is_loopback_host_rejects_public_and_wildcard_addresses() {
        assert!(!is_loopback_host("0.0.0.0"));
        assert!(!is_loopback_host("192.168.1.1"));
        assert!(!is_loopback_host("10.0.0.1"));
        assert!(!is_loopback_host("example.com"));
        assert!(!is_loopback_host("169.254.0.1"));
    }

    #[test]
    fn extract_presented_key_reads_bearer_authorization_header() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            axum::http::HeaderValue::from_static("Bearer s3cret-key"),
        );
        assert_eq!(
            extract_presented_key(&headers),
            Some(b"s3cret-key".as_slice())
        );
    }

    #[test]
    fn extract_presented_key_reads_lowercase_bearer_scheme() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            axum::http::HeaderValue::from_static("bearer s3cret-key"),
        );
        assert_eq!(
            extract_presented_key(&headers),
            Some(b"s3cret-key".as_slice())
        );
    }

    #[test]
    fn extract_presented_key_reads_x_api_key_header() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            "X-API-Key",
            axum::http::HeaderValue::from_static("s3cret-key"),
        );
        assert_eq!(
            extract_presented_key(&headers),
            Some(b"s3cret-key".as_slice())
        );
    }

    #[test]
    fn extract_presented_key_reads_bare_authorization_value() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            axum::http::HeaderValue::from_static("s3cret-key"),
        );
        assert_eq!(
            extract_presented_key(&headers),
            Some(b"s3cret-key".as_slice())
        );
    }

    #[test]
    fn extract_presented_key_returns_none_when_absent() {
        let headers = axum::http::HeaderMap::new();
        assert_eq!(extract_presented_key(&headers), None);
    }

    #[test]
    fn resolve_api_key_prefers_flag_value_over_env() {
        let _guard = ENV_GUARD.lock().unwrap();
        // SAFETY: ENV_GUARD serializes these tests so there is no concurrent access.
        unsafe { std::env::set_var("OSM_TO_BEDROCK_API_KEY", "env-value") };
        let result = resolve_api_key(Some("flag-value".to_string()));
        unsafe { std::env::remove_var("OSM_TO_BEDROCK_API_KEY") };
        assert_eq!(result.as_deref(), Some("flag-value"));
    }

    #[test]
    fn resolve_api_key_falls_back_to_env_var() {
        let _guard = ENV_GUARD.lock().unwrap();
        // SAFETY: ENV_GUARD serializes these tests so there is no concurrent access.
        unsafe { std::env::set_var("OSM_TO_BEDROCK_API_KEY", "env-value") };
        let result = resolve_api_key(None);
        unsafe { std::env::remove_var("OSM_TO_BEDROCK_API_KEY") };
        assert_eq!(result.as_deref(), Some("env-value"));
    }

    #[test]
    fn resolve_api_key_ignores_empty_flag_value() {
        let _guard = ENV_GUARD.lock().unwrap();
        // SAFETY: ENV_GUARD serializes these tests so there is no concurrent access.
        unsafe { std::env::set_var("OSM_TO_BEDROCK_API_KEY", "env-value") };
        let result = resolve_api_key(Some(String::new()));
        unsafe { std::env::remove_var("OSM_TO_BEDROCK_API_KEY") };
        assert_eq!(result.as_deref(), Some("env-value"));
    }

    #[test]
    fn resolve_api_key_returns_none_when_unset() {
        let _guard = ENV_GUARD.lock().unwrap();
        // SAFETY: ENV_GUARD serializes these tests so there is no concurrent access.
        unsafe { std::env::remove_var("OSM_TO_BEDROCK_API_KEY") };
        assert!(resolve_api_key(None).is_none());
    }

    #[test]
    fn enforce_safe_bind_allows_loopback_without_key() {
        let _guard = ENV_GUARD.lock().unwrap();
        // SAFETY: ENV_GUARD serializes these tests so there is no concurrent access.
        unsafe { std::env::remove_var("OSM_TO_BEDROCK_ALLOW_INSECURE_BIND") };
        assert!(enforce_safe_bind("127.0.0.1", &None).is_ok());
        assert!(enforce_safe_bind("localhost", &None).is_ok());
        assert!(enforce_safe_bind("::1", &None).is_ok());
    }

    #[test]
    fn enforce_safe_bind_allows_public_with_key() {
        let _guard = ENV_GUARD.lock().unwrap();
        // SAFETY: ENV_GUARD serializes these tests so there is no concurrent access.
        unsafe { std::env::remove_var("OSM_TO_BEDROCK_ALLOW_INSECURE_BIND") };
        let key = Some("a-secret".to_string());
        assert!(enforce_safe_bind("0.0.0.0", &key).is_ok());
    }

    #[test]
    fn enforce_safe_bind_rejects_public_without_key_by_default() {
        let _guard = ENV_GUARD.lock().unwrap();
        // SAFETY: ENV_GUARD serializes these tests so there is no concurrent access.
        unsafe { std::env::remove_var("OSM_TO_BEDROCK_ALLOW_INSECURE_BIND") };
        let err = enforce_safe_bind("0.0.0.0", &None).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("OSM_TO_BEDROCK_API_KEY"), "msg = {msg}");
        assert!(
            msg.contains("OSM_TO_BEDROCK_ALLOW_INSECURE_BIND"),
            "msg = {msg}"
        );
    }

    #[test]
    fn enforce_safe_bind_allows_public_without_key_when_overridden() {
        let _guard = ENV_GUARD.lock().unwrap();
        // SAFETY: ENV_GUARD serializes these tests so there is no concurrent access.
        unsafe { std::env::set_var("OSM_TO_BEDROCK_ALLOW_INSECURE_BIND", "1") };
        let result = enforce_safe_bind("0.0.0.0", &None);
        unsafe { std::env::remove_var("OSM_TO_BEDROCK_ALLOW_INSECURE_BIND") };
        assert!(result.is_ok());
    }

    #[test]
    fn auth_state_default_is_unauthenticated() {
        let state = AuthState::default();
        assert!(state.api_key.is_none());
    }

    #[test]
    fn auth_state_holds_arc_wrapped_key() {
        let state = AuthState {
            api_key: Some(Arc::new("a-secret".to_string())),
        };
        assert_eq!(
            state.api_key.as_deref().map(String::as_str),
            Some("a-secret")
        );
    }
}
