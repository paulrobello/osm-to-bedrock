//! HTTP API server for OSM-to-Bedrock conversion.
//!
//! ## Endpoints
//!
//! - `GET  /health`       — liveness check, returns `{"status":"ok"}`
//! - `POST /parse`        — multipart upload of one or more `.osm.pbf` files;
//!   returns GeoJSON + bounding box + feature-count stats.
//! - `POST /convert`      — multipart upload of a `.osm.pbf` file + options JSON;
//!   spawns a background conversion and returns a job ID.
//! - `POST /preview`      — multipart upload of a `.osm.pbf` file + optional
//!   options JSON; returns the surface block grid as JSON.
//! - `GET  /status/{id}`   — poll conversion progress for a job ID.
//! - `GET  /download/{id}` — download the `.mcworld` file once conversion is done
//!   (includes `Content-Length` header).
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

use std::collections::HashMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Multipart, State},
    http::{HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use geojson::GeoJson;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tower_http::cors::CorsLayer;
use uuid::Uuid;

use crate::geojson_export;
use crate::osm;
use crate::params::{ConvertParams, TerrainParams};
use crate::pipeline::{
    run_conversion, run_conversion_preview, run_surface_preview, run_terrain_only_to_disk,
    zip_directory,
};

// ── Error wrapper ──────────────────────────────────────────────────────────

/// A newtype around [`anyhow::Error`] that renders as an HTTP 500 JSON body.
///
/// The full error chain is logged at ERROR level but only a generic message is
/// returned to the caller to avoid leaking internal file paths, OS error
/// strings, or implementation details.
struct ApiError(anyhow::Error);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        // Log the full detail server-side; return a generic message to callers.
        log::error!("Internal server error: {:#}", self.0);
        let body = json!({ "error": "An internal server error occurred." });
        (StatusCode::INTERNAL_SERVER_ERROR, Json(body)).into_response()
    }
}

impl<E: Into<anyhow::Error>> From<E> for ApiError {
    fn from(e: E) -> Self {
        ApiError(e.into())
    }
}

// ── Job state ─────────────────────────────────────────────────────────────

/// The state of a background conversion job.
#[derive(Clone)]
enum JobState {
    Running {
        progress: f32,
        message: String,
    },
    Done {
        path: PathBuf,
        /// Wall-clock time at which the job reached the Done state.
        created: Instant,
    },
    Error {
        message: String,
        /// Wall-clock time at which the job failed.
        created: Instant,
    },
}

/// Shared application state holding all conversion jobs.
type Jobs = Arc<Mutex<HashMap<String, JobState>>>;

/// How long completed (Done or Error) jobs are kept in memory before eviction.
///
/// The persisted temp directory is also cleaned up at eviction time.
const JOB_TTL: Duration = Duration::from_secs(2 * 60 * 60); // 2 hours

/// Maximum number of simultaneously running conversion jobs.
///
/// Requests that would exceed this cap are rejected with HTTP 429.
const MAX_CONCURRENT_JOBS: usize = 4;

/// Axum app state.
#[derive(Clone)]
struct AppState {
    jobs: Jobs,
    /// Semaphore that bounds the number of concurrent blocking conversion jobs.
    semaphore: Arc<tokio::sync::Semaphore>,
}

// ── Job helper functions ───────────────────────────────────────────────────

/// Record a terminal error state for `jid` into the jobs map.
///
/// Centralises the repetitive `jobs.lock().insert(jid, JobState::Error{…})` pattern
/// found in every background-task closure.
fn set_job_error(jobs: &Jobs, jid: &str, message: String) {
    let mut map = jobs.lock().expect("jobs lock poisoned");
    map.insert(
        jid.to_string(),
        JobState::Error {
            message,
            created: Instant::now(),
        },
    );
}

/// Zip `world_dir` into a `.mcworld` archive, persist the containing temp directory
/// to disk (so the file survives the `TempDir` drop), and record `JobState::Done`.
///
/// On failure the archive is left on the filesystem (it may be partial) and
/// `JobState::Error` is recorded instead.
fn zip_and_persist(
    jobs: &Jobs,
    jid: &str,
    output_dir: tempfile::TempDir,
    world_dir: &Path,
    world_name: &str,
) {
    let mcworld_path = output_dir.path().join(format!("{world_name}.mcworld"));
    match zip_directory(world_dir, &mcworld_path) {
        Ok(()) => {
            let persisted_dir = output_dir.keep();
            let final_path = persisted_dir.join(format!("{world_name}.mcworld"));
            let mut map = jobs.lock().expect("jobs lock poisoned");
            map.insert(
                jid.to_string(),
                JobState::Done {
                    path: final_path,
                    created: Instant::now(),
                },
            );
        }
        Err(e) => {
            set_job_error(jobs, jid, format!("Failed to create .mcworld: {e}"));
        }
    }
}

// ── Input sanitisation ────────────────────────────────────────────────────

/// Sanitise a user-supplied world name so it is safe to use as a directory
/// component and as an HTTP `Content-Disposition` filename.
///
/// Rules applied (in order):
/// 1. Strip any leading/trailing whitespace.
/// 2. Remove path separator characters (`/`, `\`), dot characters (`.`,
///    which could form `..` traversal sequences), ASCII control characters
///    (0x00–0x1F), and DEL (0x7F).
/// 3. Collapse any remaining runs of whitespace to a single space.
/// 4. If the result is empty after sanitisation, fall back to `"OSM World"`.
fn sanitize_world_name(name: &str) -> String {
    // Step 1: trim surrounding whitespace.
    let s = name.trim();

    // Step 2: remove unsafe characters character by character.
    let char_filtered: String = s
        .chars()
        .filter(|c| *c != '/' && *c != '\\' && *c != '.' && (*c as u32) >= 0x20 && *c != '\x7f')
        .collect();

    // Step 3: collapse whitespace runs.
    let collapsed: String = char_filtered
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    // Step 4: fall back to a safe default if nothing remains.
    if collapsed.is_empty() {
        "OSM World".to_string()
    } else {
        collapsed
    }
}

// ── Response types ─────────────────────────────────────────────────────────

/// Feature-count statistics included with every `/parse` response.
#[derive(Debug, Serialize, Deserialize)]
pub struct ParseStats {
    pub total_features: usize,
    pub roads: usize,
    pub buildings: usize,
    pub water: usize,
    pub landuse: usize,
    pub other: usize,
    pub nodes: usize,
    pub ways: usize,
}

/// Bounding box derived from the parsed OSM data.
#[derive(Debug, Serialize, Deserialize)]
pub struct Bounds {
    pub min_lat: f64,
    pub min_lon: f64,
    pub max_lat: f64,
    pub max_lon: f64,
}

/// The full response body returned by `POST /parse`.
#[derive(Debug, Serialize)]
pub struct ParseResponse {
    pub geojson: serde_json::Value,
    pub bounds: Option<Bounds>,
    pub stats: ParseStats,
}

/// Conversion options sent as JSON in the multipart `options` field.
#[derive(Debug, Deserialize)]
struct ConvertOptions {
    #[serde(default = "default_scale")]
    scale: f64,
    #[serde(default = "default_sea_level")]
    sea_level: i32,
    #[serde(default = "default_building_height")]
    building_height: i32,
    #[serde(default = "default_wall_straighten_threshold")]
    wall_straighten_threshold: i32,
    #[serde(default = "default_world_name")]
    world_name: String,
    /// Explicit spawn block coordinates — take priority over spawn_lat/lon.
    spawn_x: Option<i32>,
    spawn_y: Option<i32>,
    spawn_z: Option<i32>,
    /// Spawn position as geographic coordinates — converted to block coords by the converter.
    spawn_lat: Option<f64>,
    spawn_lon: Option<f64>,
    #[serde(default)]
    signs: Option<bool>,
    #[serde(default)]
    address_signs: Option<bool>,
    #[serde(default)]
    poi_markers: Option<bool>,
    // Feature filter fields
    #[serde(default = "default_true")]
    roads: bool,
    #[serde(default = "default_true")]
    buildings: bool,
    #[serde(default = "default_true")]
    water: bool,
    #[serde(default = "default_true")]
    landuse: bool,
    #[serde(default = "default_true")]
    railways: bool,
    #[serde(default)]
    use_elevation: bool,
    #[serde(default = "default_vertical_scale")]
    vertical_scale: f64,
    #[serde(default = "default_elevation_smoothing")]
    elevation_smoothing: i32,
    #[serde(default = "default_surface_thickness")]
    surface_thickness: i32,
    #[serde(default = "default_true")]
    poi_decorations: bool,
    #[serde(default = "default_true")]
    nature_decorations: bool,
}

impl Default for ConvertOptions {
    fn default() -> Self {
        Self {
            scale: default_scale(),
            sea_level: default_sea_level(),
            building_height: default_building_height(),
            wall_straighten_threshold: default_wall_straighten_threshold(),
            world_name: default_world_name(),
            spawn_x: None,
            spawn_y: None,
            spawn_z: None,
            spawn_lat: None,
            spawn_lon: None,
            signs: None,
            address_signs: None,
            poi_markers: None,
            roads: true,
            buildings: true,
            water: true,
            landuse: true,
            railways: true,
            use_elevation: false,
            vertical_scale: default_vertical_scale(),
            elevation_smoothing: default_elevation_smoothing(),
            surface_thickness: default_surface_thickness(),
            poi_decorations: true,
            nature_decorations: true,
        }
    }
}

/// Validate numeric bounds on `ConvertOptions`.
///
/// Returns an error string if any value is outside the accepted range.
/// Prevents crafted inputs like `scale = 1e300` from causing near-infinite
/// rasterization loops or memory exhaustion.
fn validate_convert_options(opts: &ConvertOptions) -> Result<(), &'static str> {
    if !(0.01..=100.0).contains(&opts.scale) {
        return Err("scale must be in range 0.01 .. 100.0");
    }
    if !(0..=320).contains(&opts.sea_level) {
        return Err("sea_level must be in range 0 .. 320");
    }
    if !(1..=64).contains(&opts.building_height) {
        return Err("building_height must be in range 1 .. 64");
    }
    if !(0.01..=100.0).contains(&opts.vertical_scale) {
        return Err("vertical_scale must be in range 0.01 .. 100.0");
    }
    if !(0..=5).contains(&opts.elevation_smoothing) {
        return Err("elevation_smoothing must be in range 0 .. 5");
    }
    if !(1..=128).contains(&opts.surface_thickness) {
        return Err("surface_thickness must be in range 1 .. 128");
    }
    Ok(())
}

fn build_filter(opts: &ConvertOptions) -> crate::filter::FeatureFilter {
    crate::filter::FeatureFilter {
        roads: opts.roads,
        buildings: opts.buildings,
        water: opts.water,
        landuse: opts.landuse,
        railways: opts.railways,
    }
}

/// Validate numeric bounds on `FetchConvertOptions`.
fn validate_fetch_convert_options(opts: &FetchConvertOptions) -> Result<(), &'static str> {
    if !(0.01..=100.0).contains(&opts.scale) {
        return Err("scale must be in range 0.01 .. 100.0");
    }
    if !(0..=320).contains(&opts.sea_level) {
        return Err("sea_level must be in range 0 .. 320");
    }
    if !(1..=64).contains(&opts.building_height) {
        return Err("building_height must be in range 1 .. 64");
    }
    if !(0.01..=100.0).contains(&opts.vertical_scale) {
        return Err("vertical_scale must be in range 0.01 .. 100.0");
    }
    if !(0..=5).contains(&opts.elevation_smoothing) {
        return Err("elevation_smoothing must be in range 0 .. 5");
    }
    if !(1..=128).contains(&opts.surface_thickness) {
        return Err("surface_thickness must be in range 1 .. 128");
    }
    Ok(())
}

/// Validate numeric bounds on `TerrainConvertOptions`.
fn validate_terrain_convert_options(opts: &TerrainConvertOptions) -> Result<(), &'static str> {
    if !(0.01..=100.0).contains(&opts.scale) {
        return Err("scale must be in range 0.01 .. 100.0");
    }
    if !(0..=320).contains(&opts.sea_level) {
        return Err("sea_level must be in range 0 .. 320");
    }
    if !(0.01..=100.0).contains(&opts.vertical_scale) {
        return Err("vertical_scale must be in range 0.01 .. 100.0");
    }
    if !(0..=5).contains(&opts.elevation_smoothing) {
        return Err("elevation_smoothing must be in range 0 .. 5");
    }
    if !(1..=128).contains(&opts.surface_thickness) {
        return Err("surface_thickness must be in range 1 .. 128");
    }
    Ok(())
}

fn default_scale() -> f64 {
    1.0
}
fn default_sea_level() -> i32 {
    65
}
fn default_building_height() -> i32 {
    8
}
fn default_wall_straighten_threshold() -> i32 {
    1
}
fn default_elevation_smoothing() -> i32 {
    1
}
fn default_surface_thickness() -> i32 {
    4
}
fn default_world_name() -> String {
    "OSM World".to_string()
}
fn default_true() -> bool {
    true
}
fn default_vertical_scale() -> f64 {
    1.0
}
fn default_overture_timeout() -> u64 {
    120
}

/// Request body for `POST /fetch-convert`.
#[derive(Debug, Deserialize)]
struct FetchConvertRequest {
    /// Bounding box [south, west, north, east].
    bbox: [f64; 4],
    #[serde(default)]
    options: FetchConvertOptions,
    #[serde(default)]
    filter: crate::filter::FeatureFilter,
    /// If true, bypass cache read — always fetch from Overpass but still write result to cache.
    #[serde(default)]
    force_refresh: bool,
    /// Optional Overpass API URL override. Falls back to OVERPASS_URL env var or default.
    #[serde(default)]
    overpass_url: Option<String>,
    /// If true, also fetch and merge Overture Maps data.
    #[serde(default)]
    overture: bool,
    /// Overture themes to fetch (empty = all themes).
    #[serde(default)]
    overture_themes: Vec<String>,
    /// Per-theme priority override map (theme name → "overture" | "osm" | "both").
    #[serde(default)]
    overture_priority: std::collections::HashMap<String, String>,
    /// Timeout in seconds for the overturemaps CLI subprocess.
    #[serde(default = "default_overture_timeout")]
    overture_timeout: u64,
}

/// Conversion options for `POST /fetch-convert`.
#[derive(Debug, Deserialize)]
struct FetchConvertOptions {
    #[serde(default = "default_scale")]
    scale: f64,
    #[serde(default = "default_sea_level")]
    sea_level: i32,
    #[serde(default = "default_building_height")]
    building_height: i32,
    #[serde(default = "default_wall_straighten_threshold")]
    wall_straighten_threshold: i32,
    #[serde(default = "default_world_name")]
    world_name: String,
    spawn_x: Option<i32>,
    spawn_y: Option<i32>,
    spawn_z: Option<i32>,
    spawn_lat: Option<f64>,
    spawn_lon: Option<f64>,
    #[serde(default)]
    signs: Option<bool>,
    #[serde(default)]
    address_signs: Option<bool>,
    #[serde(default)]
    poi_markers: Option<bool>,
    #[serde(default)]
    use_elevation: bool,
    #[serde(default = "default_vertical_scale")]
    vertical_scale: f64,
    #[serde(default = "default_elevation_smoothing")]
    elevation_smoothing: i32,
    #[serde(default = "default_surface_thickness")]
    surface_thickness: i32,
    #[serde(default = "default_true")]
    poi_decorations: bool,
    #[serde(default = "default_true")]
    nature_decorations: bool,
}

impl Default for FetchConvertOptions {
    fn default() -> Self {
        Self {
            scale: default_scale(),
            sea_level: default_sea_level(),
            building_height: default_building_height(),
            wall_straighten_threshold: default_wall_straighten_threshold(),
            world_name: default_world_name(),
            spawn_x: None,
            spawn_y: None,
            spawn_z: None,
            spawn_lat: None,
            spawn_lon: None,
            signs: None,
            address_signs: None,
            poi_markers: None,
            use_elevation: false,
            vertical_scale: default_vertical_scale(),
            elevation_smoothing: default_elevation_smoothing(),
            surface_thickness: default_surface_thickness(),
            poi_decorations: true,
            nature_decorations: true,
        }
    }
}

// ── Handlers ───────────────────────────────────────────────────────────────

/// `GET /health` — liveness probe.
async fn health() -> impl IntoResponse {
    Json(json!({
        "status": "ok",
        "overture_available": crate::overture::is_cli_available()
    }))
}

/// `POST /parse` — accept a multipart upload with one or more `file` fields,
/// parse the PBF data, merge results, convert to GeoJSON, and return bounds + stats.
async fn parse_pbf_handler(mut multipart: Multipart) -> Result<impl IntoResponse, ApiError> {
    // Collect (bytes, suffix) pairs — suffix determined from uploaded filename.
    let mut file_bytes_list: Vec<(Vec<u8>, String)> = Vec::new();

    while let Some(field) = multipart.next_field().await? {
        if field.name() == Some("file") {
            let filename = field.file_name().unwrap_or("upload.osm.pbf").to_string();
            let suffix = if filename.ends_with(".osm") {
                ".osm".to_string()
            } else {
                ".osm.pbf".to_string()
            };
            let bytes = field.bytes().await?.to_vec();
            if !bytes.is_empty() {
                file_bytes_list.push((bytes, suffix));
            }
        }
    }

    if file_bytes_list.is_empty() {
        return Err(anyhow::anyhow!("multipart field 'file' is missing").into());
    }

    // ── Parse each file and merge ────────────────────────────────────────
    let osm_data = tokio::task::spawn_blocking(move || -> Result<osm::OsmData> {
        let mut merged_data: Option<osm::OsmData> = None;
        for (bytes, suffix) in file_bytes_list {
            let mut tmp_file = tempfile::Builder::new().suffix(&suffix).tempfile()?;
            tmp_file.write_all(&bytes)?;
            let (_, tmp_path) = tmp_file.into_parts();
            let path: &Path = &tmp_path;
            let data = osm::parse_osm_file(path)?;
            match &mut merged_data {
                Some(existing) => existing.merge(data),
                None => merged_data = Some(data),
            }
        }
        Ok(merged_data.unwrap())
    })
    .await??;

    // ── Convert to GeoJSON (CPU-bound, run in blocking thread) ───────────
    let (fc, stats, bounds) =
        tokio::task::spawn_blocking(move || -> Result<(_, ParseStats, Option<Bounds>)> {
            let fc = geojson_export::to_geojson(&osm_data);

            // Compute per-type stats from the feature collection.
            let mut roads = 0usize;
            let mut buildings = 0usize;
            let mut water = 0usize;
            let mut landuse = 0usize;
            let mut other = 0usize;
            for feature in &fc.features {
                let kind = feature
                    .properties
                    .as_ref()
                    .and_then(|p| p.get("_type"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("other");
                match kind {
                    "road" => roads += 1,
                    "building" => buildings += 1,
                    "water" => water += 1,
                    "landuse" => landuse += 1,
                    _ => other += 1,
                }
            }
            let total_features = fc.features.len();

            let stats = ParseStats {
                total_features,
                roads,
                buildings,
                water,
                landuse,
                other,
                nodes: osm_data.nodes.len(),
                ways: osm_data.ways.len(),
            };

            let bounds = osm_data
                .bounds
                .map(|(min_lat, min_lon, max_lat, max_lon)| Bounds {
                    min_lat,
                    min_lon,
                    max_lat,
                    max_lon,
                });

            Ok((fc, stats, bounds))
        })
        .await??;

    // Serialise GeoJSON via its own Display impl (correct RFC 7946 output).
    let geojson_value: serde_json::Value =
        serde_json::from_str(&GeoJson::FeatureCollection(fc).to_string())?;

    Ok(Json(ParseResponse {
        geojson: geojson_value,
        bounds,
        stats,
    }))
}

/// Request body for `POST /fetch-preview`.
#[derive(Debug, Deserialize)]
struct FetchPreviewRequest {
    /// Bounding box [south, west, north, east].
    bbox: [f64; 4],
    #[serde(default)]
    filter: crate::filter::FeatureFilter,
    /// Optional Overpass API URL override.
    #[serde(default)]
    overpass_url: Option<String>,
}

/// `POST /fetch-preview` — fetch OSM data from Overpass (cache-aware) and return
/// GeoJSON + bounds + stats, the same shape as `/parse`.
async fn fetch_preview_handler(
    Json(req): Json<FetchPreviewRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let bbox = (req.bbox[0], req.bbox[1], req.bbox[2], req.bbox[3]);
    let filter = req.filter;
    let overpass_url = match req.overpass_url.as_deref().filter(|s| !s.is_empty()) {
        Some(url) => url.to_string(),
        None => crate::overpass::default_overpass_url().to_string(),
    };

    let mut osm_data = tokio::task::spawn_blocking(move || {
        crate::overpass::fetch_osm_data(bbox, &filter, true, &overpass_url)
    })
    .await??;

    // Clip to requested bbox so cached larger areas don't include extra features.
    osm_data.clip_to_bbox(bbox);

    let (fc, stats, bounds) =
        tokio::task::spawn_blocking(move || -> Result<(_, ParseStats, Option<Bounds>)> {
            let fc = geojson_export::to_geojson(&osm_data);

            let mut roads = 0usize;
            let mut buildings = 0usize;
            let mut water = 0usize;
            let mut landuse = 0usize;
            let mut other = 0usize;
            for feature in &fc.features {
                let kind = feature
                    .properties
                    .as_ref()
                    .and_then(|p| p.get("_type"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("other");
                match kind {
                    "road" => roads += 1,
                    "building" => buildings += 1,
                    "water" => water += 1,
                    "landuse" => landuse += 1,
                    _ => other += 1,
                }
            }
            let total_features = fc.features.len();
            let stats = ParseStats {
                total_features,
                roads,
                buildings,
                water,
                landuse,
                other,
                nodes: osm_data.nodes.len(),
                ways: osm_data.ways.len(),
            };
            let bounds = osm_data
                .bounds
                .map(|(min_lat, min_lon, max_lat, max_lon)| Bounds {
                    min_lat,
                    min_lon,
                    max_lat,
                    max_lon,
                });
            Ok((fc, stats, bounds))
        })
        .await??;

    let geojson_value: serde_json::Value =
        serde_json::from_str(&GeoJson::FeatureCollection(fc).to_string())?;

    Ok(Json(ParseResponse {
        geojson: geojson_value,
        bounds,
        stats,
    }))
}

/// `POST /convert` — accept a multipart upload with `file` and `options` fields.
/// Spawns a background conversion task and returns a job ID immediately.
async fn convert_handler(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<impl IntoResponse, ApiError> {
    let mut file_bytes: Option<Vec<u8>> = None;
    let mut options_str: Option<String> = None;

    while let Some(field) = multipart.next_field().await? {
        match field.name() {
            Some("file") => {
                file_bytes = Some(field.bytes().await?.to_vec());
            }
            Some("options") => {
                options_str = Some(field.text().await?);
            }
            _ => {}
        }
    }

    let bytes = file_bytes.ok_or_else(|| anyhow::anyhow!("multipart field 'file' is missing"))?;
    if bytes.is_empty() {
        return Err(anyhow::anyhow!("uploaded file is empty").into());
    }

    let options: ConvertOptions = match options_str {
        Some(s) => {
            serde_json::from_str(&s).map_err(|e| anyhow::anyhow!("invalid options JSON: {e}"))?
        }
        None => ConvertOptions::default(),
    };

    // Validate numeric parameters before accepting the job.
    validate_convert_options(&options).map_err(|e| anyhow::anyhow!("{e}"))?;

    // Enforce concurrency cap: reject new jobs when the pool is exhausted.
    let _permit = state.semaphore.clone().try_acquire_owned().map_err(|_| {
        anyhow::anyhow!("server is busy — too many concurrent conversions; retry later")
    })?;

    let job_id = Uuid::new_v4().to_string();

    // Insert initial state
    {
        let mut jobs = state.jobs.lock().expect("jobs lock poisoned");
        jobs.insert(
            job_id.clone(),
            JobState::Running {
                progress: 0.0,
                message: "Queued".to_string(),
            },
        );
    }

    let jobs = state.jobs.clone();
    let jid = job_id.clone();

    // Spawn background conversion
    tokio::task::spawn_blocking(move || {
        // The permit is moved into this closure and released when the closure returns.
        let _permit = _permit;

        // Write PBF to temp file
        let tmp_file = match tempfile::Builder::new().suffix(".osm.pbf").tempfile() {
            Ok(mut f) => {
                if let Err(e) = f.write_all(&bytes).and_then(|_| f.flush()) {
                    set_job_error(&jobs, &jid, format!("Failed to write temp file: {e}"));
                    return;
                }
                f
            }
            Err(e) => {
                set_job_error(&jobs, &jid, format!("Failed to create temp file: {e}"));
                return;
            }
        };
        let (_, tmp_path) = tmp_file.into_parts();

        // Create output directory via tempdir
        let output_dir = match tempfile::Builder::new().prefix("osm-world-").tempdir() {
            Ok(d) => d,
            Err(e) => {
                set_job_error(&jobs, &jid, format!("Failed to create output dir: {e}"));
                return;
            }
        };

        let world_name = sanitize_world_name(&options.world_name);
        let world_dir = output_dir.path().join(&world_name);
        if let Err(e) = std::fs::create_dir_all(&world_dir) {
            set_job_error(&jobs, &jid, format!("Failed to create world dir: {e}"));
            return;
        }

        // Optional: download SRTM elevation tiles for the uploaded PBF's bbox.
        let elevation_dir = if options.use_elevation {
            match download_elevation_for_pbf(&tmp_path, &jobs, &jid) {
                Ok(dir) => Some(dir),
                Err(e) => {
                    set_job_error(&jobs, &jid, format!("Elevation download failed: {e}"));
                    return;
                }
            }
        } else {
            None
        };

        let params = ConvertParams {
            input: Some(tmp_path.to_path_buf()),
            output: world_dir.clone(),
            scale: options.scale,
            sea_level: options.sea_level,
            building_height: options.building_height,
            wall_straighten_threshold: options.wall_straighten_threshold,
            spawn_x: options.spawn_x,
            spawn_y: options.spawn_y,
            spawn_z: options.spawn_z,
            spawn_lat: options.spawn_lat,
            spawn_lon: options.spawn_lon,
            signs: options.signs.unwrap_or(false),
            address_signs: options.address_signs.unwrap_or(false),
            poi_markers: options.poi_markers.unwrap_or(false),
            poi_decorations: options.poi_decorations,
            nature_decorations: options.nature_decorations,
            filter: build_filter(&options),
            elevation: elevation_dir,
            vertical_scale: options.vertical_scale,
            elevation_smoothing: options.elevation_smoothing,
            surface_thickness: options.surface_thickness,
        };

        let jobs_for_progress = jobs.clone();
        let jid_for_progress = jid.clone();

        let result = run_conversion(&params, &|progress, msg| {
            let mut map = jobs_for_progress.lock().expect("jobs lock poisoned");
            map.insert(
                jid_for_progress.clone(),
                JobState::Running {
                    progress,
                    message: msg.to_string(),
                },
            );
        });

        match result {
            Ok(()) => zip_and_persist(&jobs, &jid, output_dir, &world_dir, &world_name),
            Err(e) => set_job_error(&jobs, &jid, format!("Conversion failed: {e}")),
        }
    });

    Ok(Json(json!({ "job_id": job_id })))
}

/// `GET /status/{id}` — poll conversion progress.
async fn status_handler(
    State(state): State<AppState>,
    axum::extract::Path(job_id): axum::extract::Path<String>,
) -> Result<impl IntoResponse, Response> {
    let jobs = state.jobs.lock().expect("jobs lock poisoned");
    match jobs.get(&job_id) {
        Some(JobState::Running { progress, message }) => Ok(Json(json!({
            "state": "running",
            "progress": progress,
            "message": message,
        }))),
        Some(JobState::Done { .. }) => Ok(Json(json!({
            "state": "done",
            "progress": 1.0,
            "message": "Conversion complete",
        }))),
        Some(JobState::Error { message, .. }) => Ok(Json(json!({
            "state": "error",
            "progress": 0.0,
            "message": message,
        }))),
        None => Err((
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "unknown job ID" })),
        )
            .into_response()),
    }
}

/// `GET /download/{id}` — serve the `.mcworld` file for a completed job.
async fn download_handler(
    State(state): State<AppState>,
    axum::extract::Path(job_id): axum::extract::Path<String>,
) -> Result<Response, Response> {
    let path = {
        let jobs = state.jobs.lock().expect("jobs lock poisoned");
        match jobs.get(&job_id) {
            Some(JobState::Done { path, .. }) => path.clone(),
            Some(JobState::Running { .. }) => {
                return Err((
                    StatusCode::CONFLICT,
                    Json(json!({ "error": "conversion still in progress" })),
                )
                    .into_response());
            }
            Some(JobState::Error { message, .. }) => {
                return Err((
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(json!({ "error": format!("conversion failed: {message}") })),
                )
                    .into_response());
            }
            None => {
                return Err((
                    StatusCode::NOT_FOUND,
                    Json(json!({ "error": "unknown job ID" })),
                )
                    .into_response());
            }
        }
    };

    // Read the file and serve it
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "world.mcworld".to_string());

    // Sanitize the filename for the Content-Disposition header to prevent
    // header injection: strip any characters that are unsafe in a quoted
    // header parameter (double-quotes, backslashes, CR, LF, NUL).
    let safe_file_name: String = file_name
        .chars()
        .filter(|c| *c != '"' && *c != '\\' && *c != '\r' && *c != '\n' && *c != '\0')
        .collect();

    match tokio::fs::read(&path).await {
        Ok(data) => {
            let headers = [
                (
                    axum::http::header::CONTENT_TYPE,
                    "application/octet-stream".to_string(),
                ),
                (
                    axum::http::header::CONTENT_DISPOSITION,
                    format!("attachment; filename=\"{safe_file_name}\""),
                ),
                (axum::http::header::CONTENT_LENGTH, data.len().to_string()),
            ];
            Ok((headers, data).into_response())
        }
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("failed to read mcworld file: {e}") })),
        )
            .into_response()),
    }
}

// ── Preview response types ────────────────────────────────────────────────

/// A single surface block in the preview response.
#[derive(Debug, Serialize)]
struct PreviewBlock {
    x: i32,
    z: i32,
    y: i32,
    #[serde(rename = "type")]
    block_type: String,
}

/// Bounding box in block coordinates for the preview response.
#[derive(Debug, Serialize)]
struct PreviewBounds {
    min_x: i32,
    max_x: i32,
    min_z: i32,
    max_z: i32,
}

/// The full response body returned by `POST /preview`.
#[derive(Debug, Serialize)]
struct PreviewResponse {
    blocks: Vec<PreviewBlock>,
    bounds: PreviewBounds,
    spawn: PreviewSpawn,
}

#[derive(Debug, Serialize)]
struct PreviewSpawn {
    x: i32,
    y: i32,
    z: i32,
}

/// `POST /preview` — accept a multipart upload with `file` and optional
/// `options` fields, run the conversion in memory, and return the surface
/// block grid as JSON (downsampled if large).
async fn preview_handler(mut multipart: Multipart) -> Result<impl IntoResponse, ApiError> {
    let mut file_bytes: Option<Vec<u8>> = None;
    let mut options_str: Option<String> = None;

    while let Some(field) = multipart.next_field().await? {
        match field.name() {
            Some("file") => {
                file_bytes = Some(field.bytes().await?.to_vec());
            }
            Some("options") => {
                options_str = Some(field.text().await?);
            }
            _ => {}
        }
    }

    let bytes = file_bytes.ok_or_else(|| anyhow::anyhow!("multipart field 'file' is missing"))?;
    if bytes.is_empty() {
        return Err(anyhow::anyhow!("uploaded file is empty").into());
    }

    let options: ConvertOptions = match options_str {
        Some(s) => {
            serde_json::from_str(&s).map_err(|e| anyhow::anyhow!("invalid options JSON: {e}"))?
        }
        None => ConvertOptions::default(),
    };

    let response = tokio::task::spawn_blocking(move || -> Result<PreviewResponse> {
        // Write PBF to temp file
        let mut tmp_file = tempfile::Builder::new().suffix(".osm.pbf").tempfile()?;
        tmp_file.write_all(&bytes)?;
        tmp_file.flush()?;
        let (_, tmp_path) = tmp_file.into_parts();

        // Create a temporary output directory (won't actually be written to)
        let output_dir = tempfile::Builder::new().prefix("osm-preview-").tempdir()?;
        let world_name = sanitize_world_name(&options.world_name);
        let world_dir = output_dir.path().join(&world_name);
        std::fs::create_dir_all(&world_dir)?;

        let params = ConvertParams {
            input: Some(tmp_path.to_path_buf()),
            output: world_dir,
            scale: options.scale,
            sea_level: options.sea_level,
            building_height: options.building_height,
            wall_straighten_threshold: options.wall_straighten_threshold,
            spawn_x: options.spawn_x,
            spawn_y: options.spawn_y,
            spawn_z: options.spawn_z,
            spawn_lat: options.spawn_lat,
            spawn_lon: options.spawn_lon,
            signs: options.signs.unwrap_or(false),
            address_signs: options.address_signs.unwrap_or(false),
            poi_markers: options.poi_markers.unwrap_or(false),
            poi_decorations: options.poi_decorations,
            nature_decorations: options.nature_decorations,
            filter: build_filter(&options),
            elevation: None,
            vertical_scale: 1.0,
            elevation_smoothing: 0,
            surface_thickness: 4,
        };

        let (world, spawn_x, spawn_y, spawn_z) =
            run_conversion_preview(&params, &|_progress, _msg| {})?;

        let surface = world.surface_blocks();

        // Compute percentile-based bounds to exclude outlier roads
        let mut xs: Vec<i32> = surface.iter().map(|(x, _, _, _)| *x).collect();
        let mut zs: Vec<i32> = surface.iter().map(|(_, z, _, _)| *z).collect();
        xs.sort_unstable();
        zs.sort_unstable();

        let (min_x, max_x, min_z, max_z) = if xs.is_empty() {
            (0, 0, 0, 0)
        } else {
            // Use IQR-based outlier detection: compute Q1/Q3, clip at Q1-1.5*IQR / Q3+1.5*IQR
            let q1x = xs[xs.len() / 4];
            let q3x = xs[xs.len() * 3 / 4];
            let iqr_x = (q3x - q1x).max(16);
            let q1z = zs[zs.len() / 4];
            let q3z = zs[zs.len() * 3 / 4];
            let iqr_z = (q3z - q1z).max(16);
            (
                q1x - iqr_x * 3 / 2,
                q3x + iqr_x * 3 / 2,
                q1z - iqr_z * 3 / 2,
                q3z + iqr_z * 3 / 2,
            )
        };

        // Filter to blocks within bounds
        let mut surface: Vec<_> = surface
            .into_iter()
            .filter(|(x, z, _, _)| *x >= min_x && *x <= max_x && *z >= min_z && *z <= max_z)
            .collect();

        // Downsample if more than 50,000 entries
        let max_entries = 500_000;
        if surface.len() > max_entries {
            let step = surface.len() / max_entries + 1;
            surface = surface.into_iter().step_by(step).collect();
        }

        let blocks: Vec<PreviewBlock> = surface
            .into_iter()
            .map(|(x, z, y, block_type)| PreviewBlock {
                x,
                z,
                y,
                block_type,
            })
            .collect();

        Ok(PreviewResponse {
            blocks,
            bounds: PreviewBounds {
                min_x,
                max_x,
                min_z,
                max_z,
            },
            spawn: PreviewSpawn {
                x: spawn_x,
                y: spawn_y,
                z: spawn_z,
            },
        })
    })
    .await??;

    Ok(Json(response))
}

/// `POST /fetch-block-preview` — lightweight surface-only 3D preview.
///
/// Fetches OSM data from Overpass (cache-aware), computes a height map, and
/// classifies each (x,z) position by feature type — without allocating any
/// ChunkData.  Orders of magnitude faster than the full conversion preview.
async fn fetch_block_preview_handler(
    Json(req): Json<FetchPreviewRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let bbox = (req.bbox[0], req.bbox[1], req.bbox[2], req.bbox[3]);
    let filter = req.filter;
    let overpass_url = match req.overpass_url.as_deref().filter(|s| !s.is_empty()) {
        Some(url) => url.to_string(),
        None => crate::overpass::default_overpass_url().to_string(),
    };

    let response = tokio::task::spawn_blocking(move || -> Result<PreviewResponse> {
        let mut data = crate::overpass::fetch_osm_data(bbox, &filter, true, &overpass_url)?;
        data.clip_to_bbox(bbox);

        let output_dir = tempfile::Builder::new().prefix("osm-preview-").tempdir()?;
        let world_dir = output_dir.path().join("preview");
        std::fs::create_dir_all(&world_dir)?;

        let params = ConvertParams {
            input: None,
            output: world_dir,
            scale: 1.0,
            sea_level: 65,
            building_height: 8,
            wall_straighten_threshold: 1,
            spawn_x: None,
            spawn_y: None,
            spawn_z: None,
            spawn_lat: None,
            spawn_lon: None,
            signs: false,
            address_signs: false,
            poi_markers: false,
            poi_decorations: false,
            nature_decorations: false,
            filter,
            elevation: None,
            vertical_scale: 1.0,
            elevation_smoothing: 0,
            surface_thickness: 4,
        };

        let (mut surface, spawn_x, spawn_y, spawn_z) =
            run_surface_preview(data, &params, &|_progress, _msg| {})?;

        // Compute bounds from the surface data
        let mut xs: Vec<i32> = surface.iter().map(|(x, _, _, _)| *x).collect();
        let mut zs: Vec<i32> = surface.iter().map(|(_, z, _, _)| *z).collect();
        xs.sort_unstable();
        zs.sort_unstable();

        let (min_x, max_x, min_z, max_z) = if xs.is_empty() {
            (0, 0, 0, 0)
        } else {
            (
                *xs.first().unwrap(),
                *xs.last().unwrap(),
                *zs.first().unwrap(),
                *zs.last().unwrap(),
            )
        };

        let max_entries = 500_000;
        if surface.len() > max_entries {
            let step = surface.len() / max_entries + 1;
            surface = surface.into_iter().step_by(step).collect();
        }

        let blocks: Vec<PreviewBlock> = surface
            .into_iter()
            .map(|(x, z, y, block_type)| PreviewBlock {
                x,
                z,
                y,
                block_type,
            })
            .collect();

        Ok(PreviewResponse {
            blocks,
            bounds: PreviewBounds {
                min_x,
                max_x,
                min_z,
                max_z,
            },
            spawn: PreviewSpawn {
                x: spawn_x,
                y: spawn_y,
                z: spawn_z,
            },
        })
    })
    .await??;

    Ok(Json(response))
}

/// `POST /fetch-convert` — fetch OSM data from Overpass and convert to .mcworld.
/// Request body is JSON (not multipart).
async fn fetch_convert_handler(
    State(state): State<AppState>,
    Json(req): Json<FetchConvertRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let bbox = (req.bbox[0], req.bbox[1], req.bbox[2], req.bbox[3]);
    let filter = req.filter;
    let options = req.options;
    let force_refresh = req.force_refresh;

    // Validate numeric parameters before accepting the job.
    validate_fetch_convert_options(&options).map_err(|e| anyhow::anyhow!("{e}"))?;

    // Enforce concurrency cap.
    let _permit = state.semaphore.clone().try_acquire_owned().map_err(|_| {
        anyhow::anyhow!("server is busy — too many concurrent conversions; retry later")
    })?;

    let job_id = Uuid::new_v4().to_string();
    {
        let mut jobs = state.jobs.lock().expect("jobs lock poisoned");
        jobs.insert(
            job_id.clone(),
            JobState::Running {
                progress: 0.0,
                message: "Queued".to_string(),
            },
        );
    }

    let jobs = state.jobs.clone();
    let jid = job_id.clone();
    let overpass_url = match req.overpass_url.as_deref().filter(|s| !s.is_empty()) {
        Some(url) => url.to_string(),
        None => crate::overpass::default_overpass_url().to_string(),
    };
    let req_overture = req.overture;
    let req_overture_themes = req.overture_themes;
    let req_overture_priority = req.overture_priority;
    let req_overture_timeout = req.overture_timeout;

    tokio::task::spawn_blocking(move || {
        // The permit is held for the duration of the blocking task.
        let _permit = _permit;

        // Fetch from Overpass (use cache unless force_refresh was requested)
        let use_cache = !force_refresh;
        let mut data =
            match crate::overpass::fetch_osm_data(bbox, &filter, use_cache, &overpass_url) {
                Ok(d) => d,
                Err(e) => {
                    set_job_error(&jobs, &jid, format!("Overpass fetch failed: {e}"));
                    return;
                }
            };

        // Optionally merge Overture Maps data.
        if req_overture {
            let themes: Vec<crate::params::OvertureTheme> = if req_overture_themes.is_empty() {
                crate::params::OvertureTheme::all()
            } else {
                req_overture_themes
                    .iter()
                    .filter_map(|s| crate::params::OvertureTheme::from_str_loose(s))
                    .collect()
            };
            let priority: std::collections::HashMap<
                crate::params::OvertureTheme,
                crate::params::ThemePriority,
            > = req_overture_priority
                .iter()
                .filter_map(|(k, v)| {
                    let theme = crate::params::OvertureTheme::from_str_loose(k)?;
                    let prio = match v.as_str() {
                        "overture" => crate::params::ThemePriority::Overture,
                        "osm" => crate::params::ThemePriority::Osm,
                        _ => crate::params::ThemePriority::Both,
                    };
                    Some((theme, prio))
                })
                .collect();
            let overture_params = crate::params::OvertureParams {
                enabled: true,
                themes,
                priority,
                timeout_secs: req_overture_timeout,
            };
            let jobs_ov = jobs.clone();
            let jid_ov = jid.clone();
            let overture_data = match crate::overture::fetch_overture_data(
                bbox,
                &overture_params,
                &mut |progress, msg| {
                    let mut map = jobs_ov.lock().expect("jobs lock poisoned");
                    map.insert(
                        jid_ov.clone(),
                        JobState::Running {
                            progress: progress * 0.3,
                            message: msg.to_string(),
                        },
                    );
                },
            ) {
                Ok(d) => d,
                Err(e) => {
                    set_job_error(&jobs, &jid, format!("Overture fetch failed: {e}"));
                    return;
                }
            };
            data.merge(overture_data);
        }

        // Clip to the requested bbox so cached larger areas don't bloat the world.
        data.clip_to_bbox(bbox);

        let output_dir = match tempfile::Builder::new().prefix("osm-world-").tempdir() {
            Ok(d) => d,
            Err(e) => {
                set_job_error(&jobs, &jid, format!("Failed to create output dir: {e}"));
                return;
            }
        };

        let world_name = sanitize_world_name(&options.world_name);
        let world_dir = output_dir.path().join(&world_name);
        if let Err(e) = std::fs::create_dir_all(&world_dir) {
            set_job_error(&jobs, &jid, format!("Failed to create world dir: {e}"));
            return;
        }

        // Optional: download SRTM elevation tiles for the requested bbox.
        let elevation_dir = if options.use_elevation {
            match download_elevation_for_bbox(bbox.0, bbox.1, bbox.2, bbox.3, &jobs, &jid) {
                Ok(dir) => Some(dir),
                Err(e) => {
                    set_job_error(&jobs, &jid, format!("Elevation download failed: {e}"));
                    return;
                }
            }
        } else {
            None
        };

        let params = ConvertParams {
            input: None,
            output: world_dir.clone(),
            scale: options.scale,
            sea_level: options.sea_level,
            building_height: options.building_height,
            wall_straighten_threshold: options.wall_straighten_threshold,
            spawn_x: options.spawn_x,
            spawn_y: options.spawn_y,
            spawn_z: options.spawn_z,
            spawn_lat: options.spawn_lat,
            spawn_lon: options.spawn_lon,
            signs: options.signs.unwrap_or(false),
            address_signs: options.address_signs.unwrap_or(false),
            poi_markers: options.poi_markers.unwrap_or(false),
            poi_decorations: options.poi_decorations,
            nature_decorations: options.nature_decorations,
            filter,
            elevation: elevation_dir,
            vertical_scale: options.vertical_scale,
            elevation_smoothing: options.elevation_smoothing,
            surface_thickness: options.surface_thickness,
        };

        let jobs_for_progress = jobs.clone();
        let jid_for_progress = jid.clone();

        let result = crate::pipeline::run_conversion_from_data(data, &params, &|progress, msg| {
            let mut map = jobs_for_progress.lock().expect("jobs lock poisoned");
            map.insert(
                jid_for_progress.clone(),
                JobState::Running {
                    progress,
                    message: msg.to_string(),
                },
            );
        });

        match result {
            Ok(()) => zip_and_persist(&jobs, &jid, output_dir, &world_dir, &world_name),
            Err(e) => set_job_error(&jobs, &jid, format!("Conversion failed: {e}")),
        }
    });

    Ok(Json(json!({ "job_id": job_id })))
}

/// Request body for `POST /terrain-convert`.
#[derive(Debug, Deserialize)]
struct TerrainConvertRequest {
    /// Bounding box [south, west, north, east].
    bbox: [f64; 4],
    #[serde(default)]
    options: TerrainConvertOptions,
}

/// Conversion options for `POST /terrain-convert`.
#[derive(Debug, Deserialize)]
struct TerrainConvertOptions {
    #[serde(default = "default_world_name")]
    world_name: String,
    #[serde(default = "default_scale")]
    scale: f64,
    #[serde(default = "default_sea_level")]
    sea_level: i32,
    #[serde(default = "default_vertical_scale")]
    vertical_scale: f64,
    #[serde(default = "default_snow_line")]
    snow_line: i32,
    #[serde(default = "default_elevation_smoothing")]
    elevation_smoothing: i32,
    #[serde(default = "default_surface_thickness")]
    surface_thickness: i32,
    spawn_x: Option<i32>,
    spawn_y: Option<i32>,
    spawn_z: Option<i32>,
    spawn_lat: Option<f64>,
    spawn_lon: Option<f64>,
    /// When true, auto-download SRTM tiles for the bbox. Defaults to true.
    #[serde(default = "default_true")]
    use_elevation: bool,
}

fn default_snow_line() -> i32 {
    80
}

impl Default for TerrainConvertOptions {
    fn default() -> Self {
        Self {
            world_name: default_world_name(),
            scale: default_scale(),
            sea_level: default_sea_level(),
            vertical_scale: default_vertical_scale(),
            snow_line: default_snow_line(),
            elevation_smoothing: default_elevation_smoothing(),
            surface_thickness: default_surface_thickness(),
            spawn_x: None,
            spawn_y: None,
            spawn_z: None,
            spawn_lat: None,
            spawn_lon: None,
            use_elevation: true,
        }
    }
}

/// `POST /terrain-convert` — generate a terrain-only world from SRTM elevation.
/// Accepts a JSON body with `bbox` and `options`; returns a job ID immediately.
async fn terrain_convert_handler(
    State(state): State<AppState>,
    Json(req): Json<TerrainConvertRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let bbox = (req.bbox[0], req.bbox[1], req.bbox[2], req.bbox[3]);
    let options = req.options;

    // Validate numeric parameters before accepting the job.
    validate_terrain_convert_options(&options).map_err(|e| anyhow::anyhow!("{e}"))?;

    // Enforce concurrency cap.
    let _permit = state.semaphore.clone().try_acquire_owned().map_err(|_| {
        anyhow::anyhow!("server is busy — too many concurrent conversions; retry later")
    })?;

    let job_id = Uuid::new_v4().to_string();
    {
        let mut jobs = state.jobs.lock().expect("jobs lock poisoned");
        jobs.insert(
            job_id.clone(),
            JobState::Running {
                progress: 0.0,
                message: "Queued".to_string(),
            },
        );
    }

    let jobs = state.jobs.clone();
    let jid = job_id.clone();

    tokio::task::spawn_blocking(move || {
        // The permit is held for the duration of the blocking task.
        let _permit = _permit;

        // Download SRTM elevation tiles when requested.
        let elevation_path = if options.use_elevation {
            match download_elevation_for_bbox(bbox.0, bbox.1, bbox.2, bbox.3, &jobs, &jid) {
                Ok(dir) => Some(dir),
                Err(e) => {
                    set_job_error(&jobs, &jid, format!("Elevation download failed: {e}"));
                    return;
                }
            }
        } else {
            None
        };

        let output_dir = match tempfile::Builder::new().prefix("terrain-world-").tempdir() {
            Ok(d) => d,
            Err(e) => {
                set_job_error(&jobs, &jid, format!("Failed to create output dir: {e}"));
                return;
            }
        };

        let world_name = sanitize_world_name(&options.world_name);
        let world_dir = output_dir.path().join(&world_name);
        if let Err(e) = std::fs::create_dir_all(&world_dir) {
            set_job_error(&jobs, &jid, format!("Failed to create world dir: {e}"));
            return;
        }

        let params = TerrainParams {
            bbox,
            output: world_dir.clone(),
            scale: options.scale,
            sea_level: options.sea_level,
            vertical_scale: options.vertical_scale,
            snow_line: options.snow_line,
            elevation_smoothing: options.elevation_smoothing,
            surface_thickness: options.surface_thickness,
            spawn_x: options.spawn_x,
            spawn_y: options.spawn_y,
            spawn_z: options.spawn_z,
            spawn_lat: options.spawn_lat,
            spawn_lon: options.spawn_lon,
            elevation: elevation_path,
        };

        let jobs_for_progress = jobs.clone();
        let jid_for_progress = jid.clone();

        let result = run_terrain_only_to_disk(&params, &|progress, msg| {
            let mut map = jobs_for_progress.lock().expect("jobs lock poisoned");
            map.insert(
                jid_for_progress.clone(),
                JobState::Running {
                    progress,
                    message: msg.to_string(),
                },
            );
        });

        match result {
            Ok(()) => zip_and_persist(&jobs, &jid, output_dir, &world_dir, &world_name),
            Err(e) => set_job_error(&jobs, &jid, format!("Terrain generation failed: {e:#}")),
        }
    });

    Ok(Json(json!({ "job_id": job_id })))
}

/// Request body for `POST /overture-convert`.
#[derive(Debug, Deserialize)]
struct OvertureConvertRequest {
    /// Bounding box [south, west, north, east].
    bbox: [f64; 4],
    #[serde(default)]
    options: FetchConvertOptions,
    /// Overture themes to fetch (empty = all themes).
    #[serde(default)]
    themes: Vec<String>,
    /// Timeout in seconds for the overturemaps CLI subprocess.
    #[serde(default = "default_overture_timeout")]
    timeout: u64,
}

/// `POST /overture-convert` — fetch Overture Maps data and convert to .mcworld.
/// Request body is JSON. Returns a job ID immediately.
async fn overture_convert_handler(
    State(state): State<AppState>,
    Json(req): Json<OvertureConvertRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let bbox = (req.bbox[0], req.bbox[1], req.bbox[2], req.bbox[3]);
    let options = req.options;

    // Validate numeric parameters before accepting the job.
    validate_fetch_convert_options(&options).map_err(|e| anyhow::anyhow!("{e}"))?;

    // Enforce concurrency cap.
    let _permit = state.semaphore.clone().try_acquire_owned().map_err(|_| {
        anyhow::anyhow!("server is busy — too many concurrent conversions; retry later")
    })?;

    let job_id = Uuid::new_v4().to_string();
    {
        let mut jobs = state.jobs.lock().expect("jobs lock poisoned");
        jobs.insert(
            job_id.clone(),
            JobState::Running {
                progress: 0.0,
                message: "Queued".to_string(),
            },
        );
    }

    let jobs = state.jobs.clone();
    let jid = job_id.clone();
    let themes_raw = req.themes;
    let timeout_secs = req.timeout;

    tokio::task::spawn_blocking(move || {
        // The permit is held for the duration of the blocking task.
        let _permit = _permit;

        let themes: Vec<crate::params::OvertureTheme> = if themes_raw.is_empty() {
            crate::params::OvertureTheme::all()
        } else {
            themes_raw
                .iter()
                .filter_map(|s| crate::params::OvertureTheme::from_str_loose(s))
                .collect()
        };

        let overture_params = crate::params::OvertureParams {
            enabled: true,
            themes,
            priority: std::collections::HashMap::new(),
            timeout_secs,
        };

        let jobs_ov = jobs.clone();
        let jid_ov = jid.clone();
        let data = match crate::overture::fetch_overture_data(
            bbox,
            &overture_params,
            &mut |progress, msg| {
                let mut map = jobs_ov.lock().expect("jobs lock poisoned");
                map.insert(
                    jid_ov.clone(),
                    JobState::Running {
                        progress: progress * 0.3,
                        message: msg.to_string(),
                    },
                );
            },
        ) {
            Ok(mut d) => {
                d.clip_to_bbox(bbox);
                d
            }
            Err(e) => {
                set_job_error(&jobs, &jid, format!("Overture fetch failed: {e}"));
                return;
            }
        };

        // Check if any data was actually returned.
        if data.ways.is_empty() && data.poi_nodes.is_empty() && data.addr_nodes.is_empty() {
            set_job_error(
                &jobs,
                &jid,
                "No Overture data found for this area".to_string(),
            );
            return;
        }

        let output_dir = match tempfile::Builder::new().prefix("osm-world-").tempdir() {
            Ok(d) => d,
            Err(e) => {
                set_job_error(&jobs, &jid, format!("Failed to create output dir: {e}"));
                return;
            }
        };

        let world_name = sanitize_world_name(&options.world_name);
        let world_dir = output_dir.path().join(&world_name);
        if let Err(e) = std::fs::create_dir_all(&world_dir) {
            set_job_error(&jobs, &jid, format!("Failed to create world dir: {e}"));
            return;
        }

        // Optional: download SRTM elevation tiles for the requested bbox.
        let elevation_dir = if options.use_elevation {
            match download_elevation_for_bbox(bbox.0, bbox.1, bbox.2, bbox.3, &jobs, &jid) {
                Ok(dir) => Some(dir),
                Err(e) => {
                    set_job_error(&jobs, &jid, format!("Elevation download failed: {e}"));
                    return;
                }
            }
        } else {
            None
        };

        let params = ConvertParams {
            input: None,
            output: world_dir.clone(),
            scale: options.scale,
            sea_level: options.sea_level,
            building_height: options.building_height,
            wall_straighten_threshold: options.wall_straighten_threshold,
            spawn_x: options.spawn_x,
            spawn_y: options.spawn_y,
            spawn_z: options.spawn_z,
            spawn_lat: options.spawn_lat,
            spawn_lon: options.spawn_lon,
            signs: options.signs.unwrap_or(false),
            address_signs: options.address_signs.unwrap_or(false),
            poi_markers: options.poi_markers.unwrap_or(false),
            poi_decorations: options.poi_decorations,
            nature_decorations: options.nature_decorations,
            filter: crate::filter::FeatureFilter::default(),
            elevation: elevation_dir,
            vertical_scale: options.vertical_scale,
            elevation_smoothing: options.elevation_smoothing,
            surface_thickness: options.surface_thickness,
        };

        let jobs_for_progress = jobs.clone();
        let jid_for_progress = jid.clone();

        let result = crate::pipeline::run_conversion_from_data(data, &params, &|progress, msg| {
            let mut map = jobs_for_progress.lock().expect("jobs lock poisoned");
            map.insert(
                jid_for_progress.clone(),
                JobState::Running {
                    progress: 0.3 + progress * 0.6,
                    message: msg.to_string(),
                },
            );
        });

        match result {
            Ok(()) => zip_and_persist(&jobs, &jid, output_dir, &world_dir, &world_name),
            Err(e) => set_job_error(&jobs, &jid, format!("Conversion failed: {e}")),
        }
    });

    Ok(Json(json!({ "job_id": job_id })))
}

// ── Elevation helpers ──────────────────────────────────────────────────────

/// Download SRTM tiles covering the bounding box of an uploaded PBF file
/// into the persistent cache directory.  Returns the cache `PathBuf`.
fn download_elevation_for_pbf(
    pbf_path: &Path,
    jobs: &Jobs,
    jid: &str,
) -> anyhow::Result<std::path::PathBuf> {
    let osm_data = crate::osm::parse_pbf(pbf_path)?;
    let (min_lat, min_lon, max_lat, max_lon) = osm_data.bounds.ok_or_else(|| {
        anyhow::anyhow!("PBF has no bounding box — cannot determine elevation tiles")
    })?;
    download_elevation_for_bbox(min_lat, min_lon, max_lat, max_lon, jobs, jid)
}

/// Ensure SRTM tiles covering the given bounding box are present in the
/// persistent cache directory (already-downloaded tiles are skipped).
/// Returns the cache `PathBuf` to pass as the elevation path.
fn download_elevation_for_bbox(
    min_lat: f64,
    min_lon: f64,
    max_lat: f64,
    max_lon: f64,
    jobs: &Jobs,
    jid: &str,
) -> anyhow::Result<std::path::PathBuf> {
    let cache = crate::srtm::cache_dir();
    log::info!("SRTM cache: {}", cache.display());
    crate::srtm::download_tiles_for_bbox(
        min_lat,
        min_lon,
        max_lat,
        max_lon,
        &cache,
        &|i, total: usize, name| {
            let mut jobs = jobs.lock().expect("jobs lock poisoned");
            jobs.insert(
                jid.to_string(),
                JobState::Running {
                    progress: i as f32 / total.max(1) as f32 * 0.2,
                    message: format!("Downloading elevation tile {name} ({}/{total})", i + 1),
                },
            );
        },
    )?;
    Ok(cache)
}

// ── Router / entry-point ───────────────────────────────────────────────────

/// `GET /cache/areas` — list all cached Overpass areas.
/// Never errors; returns an empty array if the cache dir is empty or doesn't exist.
async fn cache_areas_handler() -> impl IntoResponse {
    let entries = tokio::task::spawn_blocking(crate::osm_cache::list_areas)
        .await
        .unwrap_or_default();
    Json(entries)
}

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

/// Build the Axum router with a fresh state (useful for tests).
#[allow(dead_code)]
pub fn build_router() -> Router {
    let (state, _) = build_state();
    build_router_with_state(state)
}

/// Background task that periodically evicts completed/errored jobs older than
/// [`JOB_TTL`] and deletes their associated persisted temp directories.
///
/// Runs every 15 minutes.  The loop exits naturally when the server shuts down.
async fn job_eviction_task(jobs: Jobs) {
    let interval = Duration::from_secs(15 * 60);
    loop {
        tokio::time::sleep(interval).await;

        let now = Instant::now();
        let mut to_evict: Vec<(String, PathBuf)> = Vec::new();

        {
            let guard = jobs.lock().expect("jobs lock poisoned");
            for (id, state) in guard.iter() {
                let (age, path) = match state {
                    JobState::Done { created, path } => {
                        (now.duration_since(*created), path.clone())
                    }
                    JobState::Error { created, .. } => {
                        (now.duration_since(*created), PathBuf::new())
                    }
                    JobState::Running { .. } => continue,
                };
                if age >= JOB_TTL {
                    to_evict.push((id.clone(), path));
                }
            }
        }

        if to_evict.is_empty() {
            continue;
        }

        let mut guard = jobs.lock().expect("jobs lock poisoned");
        for (id, path) in to_evict {
            guard.remove(&id);
            if path.as_os_str().is_empty() {
                continue;
            }
            // The .mcworld file lives inside a temp dir; remove the whole parent dir.
            if let Some(parent) = path.parent() {
                if let Err(e) = std::fs::remove_dir_all(parent) {
                    log::warn!(
                        "Job eviction: could not remove temp dir {}: {e}",
                        parent.display()
                    );
                } else {
                    log::info!("Job eviction: removed temp dir for job {id}");
                }
            }
        }
    }
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

/// Create application state with a shared jobs map and concurrency semaphore.
fn build_state() -> (AppState, Jobs) {
    let jobs: Jobs = Arc::new(Mutex::new(HashMap::new()));
    let state = AppState {
        jobs: jobs.clone(),
        semaphore: Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_JOBS)),
    };
    (state, jobs)
}

/// Build the Axum router from an existing [`AppState`].
///
/// Separated from [`build_router`] so callers (e.g. `run`) can share the
/// same `Jobs` reference with the eviction task.
fn build_router_with_state(state: AppState) -> Router {
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

    Router::new()
        .route("/health", get(health))
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
        .route("/fetch-preview", post(fetch_preview_handler))
        .route("/fetch-block-preview", post(fetch_block_preview_handler))
        .route("/fetch-convert", post(fetch_convert_handler))
        .route("/terrain-convert", post(terrain_convert_handler))
        .route("/overture-convert", post(overture_convert_handler))
        .route("/cache/areas", get(cache_areas_handler))
        .route("/status/{id}", get(status_handler))
        .route("/download/{id}", get(download_handler))
        .layer(cors)
        .with_state(state)
}

/// Start the HTTP server and block until it exits.
pub async fn run(host: &str, port: u16) -> Result<()> {
    cleanup_orphaned_temp_dirs();

    // Build shared state so the eviction task and router share the same Jobs map.
    let (state, jobs) = build_state();

    // Spawn the job TTL eviction background task.
    tokio::spawn(job_eviction_task(jobs));

    let addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    println!("API server listening on http://{addr}");
    axum::serve(listener, build_router_with_state(state)).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::sanitize_world_name;

    #[test]
    fn normal_name_passes_through() {
        assert_eq!(sanitize_world_name("My City"), "My City");
    }

    #[test]
    fn path_traversal_dots_removed() {
        assert_eq!(sanitize_world_name("../../../etc/passwd"), "etcpasswd");
    }

    #[test]
    fn forward_slashes_removed() {
        assert_eq!(sanitize_world_name("foo/bar"), "foobar");
    }

    #[test]
    fn backslashes_removed() {
        assert_eq!(sanitize_world_name("foo\\bar"), "foobar");
    }

    #[test]
    fn dot_dot_literal_becomes_default() {
        assert_eq!(sanitize_world_name(".."), "OSM World");
    }

    #[test]
    fn empty_string_becomes_default() {
        assert_eq!(sanitize_world_name(""), "OSM World");
    }

    #[test]
    fn whitespace_only_becomes_default() {
        assert_eq!(sanitize_world_name("   "), "OSM World");
    }

    #[test]
    fn control_characters_removed() {
        assert_eq!(sanitize_world_name("hello\x00world\x1f!"), "helloworld!");
    }

    #[test]
    fn internal_whitespace_collapsed() {
        assert_eq!(sanitize_world_name("My   World"), "My World");
    }

    #[test]
    fn header_injection_chars_removed() {
        // Newlines and CRs inside a name could inject extra HTTP header lines
        assert_eq!(
            sanitize_world_name("world\r\nX-Evil: injected"),
            "worldX-Evil: injected"
        );
    }
}
