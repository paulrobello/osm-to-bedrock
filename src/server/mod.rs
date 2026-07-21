//! HTTP API server for OSM-to-Bedrock conversion.
//!
//! ## Endpoints
//!
//! - `GET  /health`             — liveness check, returns `{"status":"ok"}`
//! - `POST /parse`              — multipart upload of one or more `.osm.pbf`
//!   files; returns GeoJSON + bounding box + feature-count stats.
//! - `POST /convert`            — multipart upload of a `.osm.pbf` file +
//!   options JSON; spawns a background conversion and returns a job ID.
//! - `POST /preview`            — multipart upload of a `.osm.pbf` file +
//!   optional options JSON; returns the surface block grid as JSON.
//! - `POST /fetch-preview`      — Overpass-backed variant of `/parse`.
//! - `POST /fetch-block-preview` — lightweight surface-only preview.
//! - `POST /fetch-convert`      — Overpass → `.mcworld` in one step.
//! - `POST /terrain-convert`    — SRTM-only world (no OSM features).
//! - `POST /overture-convert`   — Overture-only world.
//! - `GET  /status/{id}`        — poll conversion progress for a job ID.
//! - `GET  /download/{id}`      — download the `.mcworld` (Bedrock) or `.zip`
//!   (Java) file once conversion is done (includes `Content-Length`).
//! - `GET  /cache/areas`        — list cached Overpass bboxes.
//!
//! ## Usage
//!
//! ```text
//! osm-to-bedrock serve --host 127.0.0.1 --port 3002
//! curl http://localhost:3002/health
//! curl -X POST http://localhost:3002/parse \
//!      -F "file=@my_area.osm.pbf" | jq .stats
//! curl -X POST http://localhost:3002/convert \
//!      -F "file=@my_area.osm.pbf" \
//!      -F 'options={"scale":1.0,"sea_level":65}' | jq .job_id
//! curl http://localhost:3002/status/<job_id>
//! curl -OJ http://localhost:3002/download/<job_id>
//! ```
//!
//! ## Module layout (ARC-004)
//!
//! - `state`   — `Jobs` / `AppState` / `JobState` + lock + eviction.
//! - `error`   — `ApiError` wrapper rendering generic 500 / explicit 400.
//! - `auth`    — optional shared-secret API key middleware (SEC-001).
//! - `options` — request/response structs, serde defaults, validation.
//! - `handlers` — HTTP handlers + QA-004 job-control helpers.

use anyhow::Result;
use axum::{
    Router,
    extract::DefaultBodyLimit,
    http::HeaderValue,
    routing::{get, post},
};
use tower_http::cors::CorsLayer;

mod auth;
mod error;
mod handlers;
mod options;
mod state;

// Re-export the public surface so external callers (e.g. `main.rs`) see the
// same shape as the pre-split `server.rs`. `run` and `build_router` are the
// only items that need to be `pub`; everything else stays crate-private.
//
// Note: the items below are defined directly in this file (not re-exported
// from a submodule), so no `pub use` is needed — they are already `pub fn`.

use auth::{AuthState, enforce_safe_bind, require_api_key, resolve_api_key};
use handlers::{
    cache_areas_handler, convert_handler, download_handler, fetch_block_preview_handler,
    fetch_convert_handler, fetch_preview_handler, health, overture_convert_handler,
    parse_pbf_handler, preview_handler, status_handler, terrain_convert_handler,
};
use state::{AppState, build_state, job_eviction_task};

/// Resolve the allowed CORS origin.
///
/// Reads `CORS_ALLOWED_ORIGIN` from the environment; falls back to the default
/// Next.js dev server origin (`http://localhost:8031`).
fn cors_allowed_origin() -> HeaderValue {
    std::env::var("CORS_ALLOWED_ORIGIN")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| HeaderValue::from_static("http://localhost:8031"))
}

/// Delete any leftover `terrain-world-*` and `osm-world-*` temp directories
/// from a previous server run that was killed before `TempDir::drop` could run.
fn cleanup_orphaned_temp_dirs() {
    let tmp = std::env::temp_dir();
    let prefixes = ["terrain-world-", "osm-world-"];
    let Ok(entries) = std::fs::read_dir(&tmp) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if prefixes.iter().any(|p| name.starts_with(p)) {
            let path = entry.path();
            if path.is_dir() {
                match std::fs::remove_dir_all(&path) {
                    Ok(()) => log::info!("Cleaned up orphaned temp dir: {}", path.display()),
                    Err(e) => log::warn!("Could not remove {}: {e}", path.display()),
                }
            }
        }
    }
}

/// Build the Axum router with a fresh state and no API key (useful for tests).
#[allow(dead_code)] // public API: test helper + library entry for embedding the router without auth
pub fn build_router() -> Router {
    let (state, _) = build_state();
    build_router_with_state(state, None)
}

/// Build the Axum router with a fresh state and an optional shared-secret API
/// key — the keyed counterpart to [`build_router`], for tests/embeddings that
/// need to exercise the authenticated path end-to-end (SEC-001).
#[allow(dead_code)] // public API: test helper + library entry for the keyed router
pub fn build_router_with_key(api_key: Option<String>) -> Router {
    let (state, _) = build_state();
    build_router_with_state(state, api_key)
}

/// Build the Axum router from an existing [`AppState`] and an optional
/// shared-secret API key.
///
/// Separated from [`build_router`] so callers (e.g. [`run`]) can share the
/// same `Jobs` reference with the eviction task.
///
/// When `api_key` is `Some`, mutating routes plus `/download`, `/status`,
/// and `/cache/areas` require the matching key in the `Authorization` (or
/// `X-API-Key`) header. `/health` remains unauthenticated.
fn build_router_with_state(state: AppState, api_key: Option<String>) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(cors_allowed_origin())
        .allow_methods([
            axum::http::Method::GET,
            axum::http::Method::POST,
            axum::http::Method::OPTIONS,
        ])
        .allow_headers([axum::http::header::CONTENT_TYPE, axum::http::header::ACCEPT]);

    const PARSE_LIMIT: usize = 100 * 1024 * 1024;
    const CONVERT_LIMIT: usize = 500 * 1024 * 1024;
    const PREVIEW_LIMIT: usize = 50 * 1024 * 1024;
    // JSON request bodies are tiny (bbox + options + filter); 1 MiB is generous
    // and explicitly caps what was previously Axum's implicit 2 MiB default
    // (SEC-005).
    const JSON_LIMIT: usize = 1024 * 1024;

    let auth_state = AuthState {
        api_key: api_key.map(std::sync::Arc::new),
    };

    // Routes that require the shared-secret API key when one is configured.
    // `/health` is intentionally public so liveness probes work without creds.
    let protected_routes = Router::new()
        .route(
            "/parse",
            post(parse_pbf_handler).layer(DefaultBodyLimit::max(PARSE_LIMIT)),
        )
        .route(
            "/convert",
            post(convert_handler).layer(DefaultBodyLimit::max(CONVERT_LIMIT)),
        )
        .route(
            "/preview",
            post(preview_handler).layer(DefaultBodyLimit::max(PREVIEW_LIMIT)),
        )
        .route(
            "/fetch-preview",
            post(fetch_preview_handler).layer(DefaultBodyLimit::max(JSON_LIMIT)),
        )
        .route(
            "/fetch-block-preview",
            post(fetch_block_preview_handler).layer(DefaultBodyLimit::max(JSON_LIMIT)),
        )
        .route(
            "/fetch-convert",
            post(fetch_convert_handler).layer(DefaultBodyLimit::max(JSON_LIMIT)),
        )
        .route(
            "/terrain-convert",
            post(terrain_convert_handler).layer(DefaultBodyLimit::max(JSON_LIMIT)),
        )
        .route(
            "/overture-convert",
            post(overture_convert_handler).layer(DefaultBodyLimit::max(JSON_LIMIT)),
        )
        .route("/cache/areas", get(cache_areas_handler))
        .route("/status/{id}", get(status_handler))
        .route("/download/{id}", get(download_handler))
        .layer(axum::middleware::from_fn_with_state(
            auth_state,
            require_api_key,
        ))
        .with_state(state.clone());

    Router::new()
        .route("/health", get(health))
        .with_state(state)
        .merge(protected_routes)
        .layer(cors)
}

/// Start the HTTP server and block until it exits.
///
/// `api_key_flag` is the value of the `--api-key` CLI flag (may be `None`);
/// it falls back to the `OSM_TO_BEDROCK_API_KEY` env var. When both are unset
/// the server runs unauthenticated — the historical loopback-dev behaviour.
pub async fn run(host: &str, port: u16, api_key_flag: Option<String>) -> Result<()> {
    cleanup_orphaned_temp_dirs();

    let api_key = resolve_api_key(api_key_flag);

    // Fail-safe bind guard (SEC-001): refuse to expose the server on a
    // non-loopback interface without an API key, unless explicitly overridden.
    enforce_safe_bind(host, &api_key)?;

    // Build shared state so the eviction task and router share the same Jobs map.
    let (state, jobs) = build_state();

    // Spawn the job TTL eviction background task.
    tokio::spawn(job_eviction_task(jobs));

    let addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    println!("API server listening on http://{addr}");
    if api_key.is_some() {
        log::info!("API key authentication enabled on protected routes.");
    } else {
        log::info!(
            "No API key configured — running unauthenticated. \
             Set --api-key / OSM_TO_BEDROCK_API_KEY before binding non-loopback hosts."
        );
    }
    axum::serve(listener, build_router_with_state(state, api_key)).await?;
    Ok(())
}
