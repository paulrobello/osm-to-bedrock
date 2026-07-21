//! End-to-end HTTP tests that drive the real Axum router.
//!
//! These exercise the full request → middleware → handler → response path for
//! every public endpoint, complementing the per-module unit tests in
//! `src/server/{auth,error,state,options}.rs` (which cover middleware,
//! validation, and state internals in isolation).
//!
//! # Coverage
//!
//! - `GET /health` — liveness (public even under auth).
//! - `GET /cache/areas` — JSON array response.
//! - `POST /parse` — happy path (GeoJSON + bounds + stats) plus 400/500 error
//!   paths (missing/empty/garbage upload).
//! - `POST /convert` — accept → poll `/status` → `/download` happy path; 400
//!   (missing/empty file); 500 (bad options JSON, out-of-range numeric).
//! - `GET /status/{id}` — running/done/error shapes; 404 unknown job.
//! - `GET /download/{id}` — archive bytes + headers; 404 unknown job; 422
//!   errored job; 409 while running (best-effort).
//! - `POST /fetch-convert` — 400 validation paths (inverted bbox,
//!   continent-scale, malformed JSON body).
//! - auth wired into the router (SEC-001): 401 missing/wrong key, 200 Bearer /
//!   X-API-Key, public `/health`.
//!
//! # Fixtures
//!
//! `tests/fixtures/sample.osm` (XML, drives `/parse`) and `sample.osm.pbf`
//! (binary, drives `/convert` — that handler hard-codes a `.osm.pbf` temp
//! suffix). Both contain exactly one road, one building, one water body;
//! stats assertions are pinned to that shape (11 nodes, 3 ways).
//!
//! # Known behavior pinned (not bugs introduced by these tests)
//!
//! `convert_handler` parses the multipart `options` field and validates numeric
//! ranges through `anyhow` → `ApiError::From`, which renders as HTTP 500. By
//! contrast `/fetch-convert` validates the same shapes through
//! `ApiError::bad_request` (HTTP 400). The `/convert` tests below pin the
//! *current* 500 behavior and are flagged; if that handler is taught to return
//! 400 for client-side input errors, update the expected statuses here.

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, StatusCode, header};
use axum::response::Response;
use osm_to_bedrock::server::{build_router, build_router_with_key};
use serde_json::{Value, json};
use tower::ServiceExt;

// ── Fixtures ─────────────────────────────────────────────────────────────────

/// Tiny `.osm` XML extract (1 road + 1 building + 1 water; 11 nodes, 3 ways).
/// Drives `/parse`, which honours the uploaded `.osm` suffix and parses as XML.
const FIXTURE_XML: &[u8] = include_bytes!("fixtures/sample.osm");

/// Same data as a binary `.osm.pbf`. Drives `/convert` / `/preview`, whose
/// handlers hard-code a `.osm.pbf` temp suffix regardless of upload name.
const FIXTURE_PBF: &[u8] = include_bytes!("fixtures/sample.osm.pbf");

/// Shared-secret used by the auth tests.
const TEST_KEY: &str = "test-secret-key";

// ── Request / response helpers ───────────────────────────────────────────────

/// One `multipart/form-data` part.
struct Part {
    name: &'static str,
    filename: Option<&'static str>,
    content_type: Option<&'static str>,
    data: Vec<u8>,
}

impl Part {
    /// A plain text field (no `filename`).
    fn field(name: &'static str, data: Vec<u8>) -> Self {
        Self {
            name,
            filename: None,
            content_type: None,
            data,
        }
    }

    /// A file-upload part with `application/octet-stream` content type.
    fn file(name: &'static str, filename: &'static str, data: Vec<u8>) -> Self {
        Self {
            name,
            filename: Some(filename),
            content_type: Some("application/octet-stream"),
            data,
        }
    }
}

/// Multipart boundary used by [`multipart_body`].
const BOUNDARY: &str = "----osm2bedrocktestboundary123";

/// Build a `multipart/form-data` body from `parts`.
///
/// Returns the `Content-Type` header value and the raw body bytes. The body is
/// a hand-rolled RFC 7578 form so the tests need no extra multipart-writer
/// dependency; axum's `Multipart` extractor parses it from the boundary in the
/// `Content-Type` header.
fn multipart_body(parts: &[Part]) -> (String, Vec<u8>) {
    let mut body = Vec::new();
    for p in parts {
        body.extend_from_slice(format!("--{BOUNDARY}\r\n").as_bytes());
        let cd = match p.filename {
            Some(fname) => format!(
                "Content-Disposition: form-data; name=\"{}\"; filename=\"{fname}\"\r\n",
                p.name
            ),
            None => format!("Content-Disposition: form-data; name=\"{}\"\r\n", p.name),
        };
        body.extend_from_slice(cd.as_bytes());
        if let Some(ct) = p.content_type {
            body.extend_from_slice(format!("Content-Type: {ct}\r\n").as_bytes());
        }
        body.extend_from_slice(b"\r\n");
        body.extend_from_slice(&p.data);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{BOUNDARY}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={BOUNDARY}"), body)
}

fn request(method: Method, uri: &str) -> axum::http::request::Builder {
    Request::builder().method(method).uri(uri)
}

fn post_multipart(uri: &str, parts: &[Part]) -> Request<Body> {
    let (ct, body) = multipart_body(parts);
    request(Method::POST, uri)
        .header(header::CONTENT_TYPE, ct)
        .body(Body::from(body))
        .unwrap()
}

fn post_json(uri: &str, value: &Value) -> Request<Body> {
    request(Method::POST, uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(value.to_string()))
        .unwrap()
}

fn get(uri: &str) -> Request<Body> {
    request(Method::GET, uri).body(Body::empty()).unwrap()
}

fn get_with_bearer(uri: &str, key: &str) -> Request<Body> {
    request(Method::GET, uri)
        .header(header::AUTHORIZATION, format!("Bearer {key}"))
        .body(Body::empty())
        .unwrap()
}

/// Send `req` against `app` and return the response. `Router` is cheaply
/// cloneable (Arc-backed); cloning per call keeps one shared jobs map alive
/// across the convert → status → download sequence.
async fn send(app: &Router, req: Request<Body>) -> Response {
    app.clone()
        .oneshot(req)
        .await
        .expect("axum router oneshot is infallible")
}

async fn body_json(resp: Response) -> Value {
    let bytes = to_bytes(resp.into_body(), 64 * 1024 * 1024)
        .await
        .expect("body bytes");
    serde_json::from_slice(&bytes).unwrap_or_else(|e| {
        panic!(
            "response was not valid JSON ({e}); body={}",
            String::from_utf8_lossy(&bytes)
        )
    })
}

async fn body_bytes(resp: Response) -> Vec<u8> {
    to_bytes(resp.into_body(), 256 * 1024 * 1024)
        .await
        .expect("body bytes")
        .to_vec()
}

// ── Convert job helpers ──────────────────────────────────────────────────────

/// POST `/convert` with the given file bytes/filename and optional `options`
/// JSON, returning the minted job id.
async fn submit_convert(
    app: &Router,
    file_bytes: &[u8],
    filename: &'static str,
    options: Option<&Value>,
) -> String {
    let mut parts = vec![Part::file("file", filename, file_bytes.to_vec())];
    if let Some(opts) = options {
        parts.push(Part::field("options", opts.to_string().into_bytes()));
    }
    let resp = send(app, post_multipart("/convert", &parts)).await;
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "/convert should accept the job"
    );
    let body = body_json(resp).await;
    body["job_id"]
        .as_str()
        .expect("job_id in response")
        .to_string()
}

/// Poll `/status/{id}` until the job reaches a terminal (`done`/`error`)
/// state, or panic after a generous deadline (the tiny fixture converts in
/// well under a second; 30s is a wide margin).
///
/// Also returns the first `running` snapshot observed (if any) so callers can
/// assert on the running-state shape without racing the conversion.
async fn poll_until_terminal(app: &Router, job_id: &str) -> (Value, Option<Value>) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let mut first_running: Option<Value> = None;
    loop {
        let resp = send(app, get(&format!("/status/{job_id}"))).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_json(resp).await;
        let state = body["state"].as_str().unwrap_or("");
        if state == "running" && first_running.is_none() {
            first_running = Some(body.clone());
        }
        if state == "done" || state == "error" {
            return (body, first_running);
        }
        assert!(
            std::time::Instant::now() < deadline,
            "job {job_id} did not reach a terminal state within 30s; last status: {body}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

// ── /health + /cache/areas ───────────────────────────────────────────────────

#[tokio::test]
async fn health_returns_ok_and_reports_overture_availability() {
    let app = build_router();
    let resp = send(&app, get("/health")).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["status"], "ok");
    // `overture_available` is a boolean flag (present either way).
    assert!(
        body.get("overture_available").is_some(),
        "health must report overture_available; got {body}"
    );
}

#[tokio::test]
async fn cache_areas_returns_json_array() {
    let app = build_router();
    let resp = send(&app, get("/cache/areas")).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert!(
        body.is_array(),
        "/cache/areas must return a JSON array; got {body}"
    );
}

// ── /parse ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn parse_returns_geojson_bounds_and_stats_for_valid_osm_upload() {
    let app = build_router();
    let req = post_multipart(
        "/parse",
        &[Part::file("file", "sample.osm", FIXTURE_XML.to_vec())],
    );
    let resp = send(&app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;

    // Fixture: exactly 1 road, 1 building, 1 water; 11 nodes; 3 ways.
    assert_eq!(body["stats"]["roads"].as_i64(), Some(1));
    assert_eq!(body["stats"]["buildings"].as_i64(), Some(1));
    assert_eq!(body["stats"]["water"].as_i64(), Some(1));
    assert_eq!(body["stats"]["landuse"].as_i64(), Some(0));
    assert_eq!(body["stats"]["other"].as_i64(), Some(0));
    assert_eq!(body["stats"]["total_features"].as_i64(), Some(3));
    assert_eq!(body["stats"]["nodes"].as_i64(), Some(11));
    assert_eq!(body["stats"]["ways"].as_i64(), Some(3));
    assert!(
        body["bounds"].is_object(),
        "/parse must derive bounds; got {body}"
    );
    assert!(
        body["geojson"]["features"].is_array(),
        "/parse must return a GeoJSON feature collection; got {body}",
    );
}

#[tokio::test]
async fn parse_rejects_missing_file_field_with_400() {
    let app = build_router();
    // A part with the wrong name is ignored by the handler, leaving no `file`.
    let req = post_multipart("/parse", &[Part::field("not-file", b"ignored".to_vec())]);
    let resp = send(&app, req).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn parse_rejects_empty_file_upload_with_400() {
    let app = build_router();
    let req = post_multipart("/parse", &[Part::file("file", "empty.osm", Vec::new())]);
    let resp = send(&app, req).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn parse_rejects_garbage_bytes_as_internal_error() {
    // A malformed upload is an *internal* failure: the handler returns a generic
    // 500 with no parser internals leaked (SEC-002).
    let app = build_router();
    let req = post_multipart(
        "/parse",
        &[Part::file(
            "file",
            "bad.osm.pbf",
            b"this is not valid osm data".to_vec(),
        )],
    );
    let resp = send(&app, req).await;
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = body_json(resp).await;
    assert_eq!(body["error"], "An internal server error occurred.");
}

// ── /convert → /status → /download ───────────────────────────────────────────

#[tokio::test]
async fn convert_status_download_roundtrip_produces_world_archive() {
    let app = build_router();

    let job_id = submit_convert(&app, FIXTURE_PBF, "sample.osm.pbf", None).await;

    let (status, running) = poll_until_terminal(&app, &job_id).await;
    assert_eq!(
        status["state"], "done",
        "job should succeed; last status: {status}"
    );
    // If we caught the running phase, it must carry progress + message.
    if let Some(running_status) = running {
        assert!(
            running_status["progress"].is_number(),
            "running status needs progress"
        );
        assert!(
            running_status["message"].is_string(),
            "running status needs message"
        );
    }

    // Download the produced world archive.
    let resp = send(&app, get(&format!("/download/{job_id}"))).await;
    assert_eq!(resp.status(), StatusCode::OK);

    let content_type = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .expect("content-type")
        .to_str()
        .unwrap()
        .to_string();
    let disposition = resp
        .headers()
        .get(header::CONTENT_DISPOSITION)
        .expect("content-disposition")
        .to_str()
        .unwrap()
        .to_string();
    let content_length = resp
        .headers()
        .get(header::CONTENT_LENGTH)
        .expect("content-length")
        .to_str()
        .unwrap()
        .to_string();

    let bytes = body_bytes(resp).await;
    assert_eq!(content_type, "application/octet-stream");
    assert!(
        disposition.starts_with("attachment;"),
        "Content-Disposition must be an attachment: {disposition}",
    );
    assert!(
        disposition.ends_with(".mcworld\""),
        "default Bedrock world name must yield a .mcworld filename: {disposition}",
    );
    assert_eq!(content_length, bytes.len().to_string());
    assert!(
        bytes.len() > 64,
        "world archive should be non-trivially sized; got {} bytes",
        bytes.len(),
    );
    // Bedrock `.mcworld` is a ZIP archive → `PK` magic.
    assert_eq!(&bytes[0..2], b"PK", ".mcworld must be a ZIP archive");
}

#[tokio::test]
async fn convert_garbage_upload_ends_in_error_and_download_returns_422() {
    let app = build_router();

    // Garbage bytes → PBF parse fails inside the worker → JobState::Error with
    // a generic, client-safe message.
    let job_id = submit_convert(&app, b"definitely not a pbf", "garbage.osm.pbf", None).await;

    let (status, _) = poll_until_terminal(&app, &job_id).await;
    assert_eq!(status["state"], "error");
    assert_eq!(
        status["message"], "conversion failed",
        "error status must carry the generic public message; got {status}",
    );

    // Downloading an errored job → 422 with the same generic message.
    let resp = send(&app, get(&format!("/download/{job_id}"))).await;
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = body_json(resp).await;
    assert_eq!(body["error"], "conversion failed");
}

#[tokio::test]
async fn download_while_running_returns_409_if_job_still_running() {
    // The tiny fixture may convert before this request lands. If we catch it
    // running, /download must return 409; if it already finished, we confirm
    // the terminal state instead — so the test never flakes on timing, it just
    // only exercises the 409 branch when a Running window is observable.
    let app = build_router();
    let job_id = submit_convert(&app, FIXTURE_PBF, "sample.osm.pbf", None).await;

    let resp = send(&app, get(&format!("/download/{job_id}"))).await;
    match resp.status() {
        StatusCode::CONFLICT => {
            let body = body_json(resp).await;
            assert_eq!(body["error"], "conversion still in progress");
        }
        StatusCode::OK => {
            let (status, _) = poll_until_terminal(&app, &job_id).await;
            assert_eq!(
                status["state"], "done",
                "non-409 download implies the job already completed; got {status}",
            );
        }
        other => panic!("unexpected /download status while job in flight: {other}"),
    }
}

#[tokio::test]
async fn status_for_unknown_job_returns_404() {
    let app = build_router();
    let resp = send(&app, get("/status/00000000-0000-0000-0000-000000000000")).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let body = body_json(resp).await;
    assert_eq!(body["error"], "unknown job ID");
}

#[tokio::test]
async fn download_for_unknown_job_returns_404() {
    let app = build_router();
    let resp = send(&app, get("/download/00000000-0000-0000-0000-000000000000")).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let body = body_json(resp).await;
    assert_eq!(body["error"], "unknown job ID");
}

// ── /convert validation (400 happy, 500 flagged) ─────────────────────────────

#[tokio::test]
async fn convert_rejects_missing_file_field_with_400() {
    let app = build_router();
    let req = post_multipart("/convert", &[Part::field("not-file", b"ignored".to_vec())]);
    let resp = send(&app, req).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn convert_rejects_empty_file_with_400() {
    let app = build_router();
    let req = post_multipart(
        "/convert",
        &[Part::file("file", "empty.osm.pbf", Vec::new())],
    );
    let resp = send(&app, req).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn convert_rejects_malformed_options_json_as_internal_error() {
    // FLAGGED: convert_handler parses the `options` field manually and maps a
    // serde failure through `anyhow` → HTTP 500 (generic body). `/fetch-convert`
    // returns 400 for the equivalent case. Pins current behavior; update the
    // expected status if the handler is changed to 400.
    let app = build_router();
    let req = post_multipart(
        "/convert",
        &[
            Part::file("file", "sample.osm.pbf", FIXTURE_PBF.to_vec()),
            Part::field("options", b"{ this is not : json".to_vec()),
        ],
    );
    let resp = send(&app, req).await;
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = body_json(resp).await;
    assert_eq!(body["error"], "An internal server error occurred.");
}

#[tokio::test]
async fn convert_rejects_out_of_range_scale_as_internal_error() {
    // FLAGGED: same discrepancy as malformed-options — `/convert` validates
    // numeric ranges via `anyhow` → 500, while `/fetch-convert` uses 400.
    // Validation runs before a job is spawned, so no job is created here.
    let app = build_router();
    let opts = json!({"scale": 999.0}); // valid range is 0.01..=100.0
    let req = post_multipart(
        "/convert",
        &[
            Part::file("file", "sample.osm.pbf", FIXTURE_PBF.to_vec()),
            Part::field("options", opts.to_string().into_bytes()),
        ],
    );
    let resp = send(&app, req).await;
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

// ── /fetch-convert validation (deterministic 400, no network) ────────────────

#[tokio::test]
async fn fetch_convert_rejects_inverted_bbox_with_400() {
    let app = build_router();
    // south > north — validate_bbox fails before any Overpass call.
    let req = post_json("/fetch-convert", &json!({"bbox": [10.0, 0.0, 5.0, 1.0]}));
    let resp = send(&app, req).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = body_json(resp).await;
    assert!(
        body["error"].as_str().unwrap().contains("south latitude"),
        "expected south-latitude hint; got {body}",
    );
}

#[tokio::test]
async fn fetch_convert_rejects_continent_scale_bbox_with_400() {
    let app = build_router();
    let req = post_json("/fetch-convert", &json!({"bbox": [0.0, 0.0, 50.0, 50.0]}));
    let resp = send(&app, req).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn fetch_convert_rejects_malformed_json_body_with_400() {
    let app = build_router();
    // Not JSON → axum `Json` extractor rejection (400).
    let req = request(Method::POST, "/fetch-convert")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from("not json"))
        .unwrap();
    let resp = send(&app, req).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// ── Auth (SEC-001) wired through the real router ─────────────────────────────

fn keyed_app() -> Router {
    build_router_with_key(Some(TEST_KEY.to_string()))
}

#[tokio::test]
async fn health_is_public_even_when_api_key_is_set() {
    let app = keyed_app();
    let resp = send(&app, get("/health")).await;
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn protected_route_rejects_missing_key_with_401() {
    let app = keyed_app();
    let resp = send(&app, get("/cache/areas")).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn protected_route_rejects_wrong_key_with_401() {
    let app = keyed_app();
    let resp = send(&app, get_with_bearer("/cache/areas", "wrong-key")).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn protected_route_accepts_correct_bearer_key() {
    let app = keyed_app();
    let resp = send(&app, get_with_bearer("/cache/areas", TEST_KEY)).await;
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn protected_route_accepts_correct_x_api_key_header() {
    let app = keyed_app();
    let req = request(Method::GET, "/cache/areas")
        .header("X-API-Key", TEST_KEY)
        .body(Body::empty())
        .unwrap();
    let resp = send(&app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
}
