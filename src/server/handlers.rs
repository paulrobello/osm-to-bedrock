//! HTTP handlers for the API server, plus the QA-004 helpers that collapse
//! the duplicated job-control boilerplate shared by the four conversion
//! endpoints.
//!
//! ## Job lifecycle (QA-004)
//!
//! `convert`, `fetch-convert`, `terrain-convert`, and `overture-convert`
//! all follow the same shape: validate input → claim a semaphore slot → mint
//! a job ID → insert `JobState::Running` → spawn blocking work → write
//! `JobState::Done` (via [`finalize_conversion`]) or `JobState::Error` (via
//! [`state::set_job_error`]). The three helpers here own the truly
//! identical parts so each handler body shrinks to the work that's actually
//! unique to it:
//!
//! - [`spawn_conversion_job`] — semaphore acquire + job ID + insert
//!   `Running` + `spawn_blocking`. Takes a closure that receives `(jobs,
//!   jid)` and does the endpoint-specific work; the semaphore permit is
//!   moved into the closure and released on return.
//! - [`prepare_world_dir`] — create the temp output dir, sanitise the world
//!   name, `create_dir_all` the world dir. Returns `None` after recording a
//!   `"conversion failed"` job error.
//! - [`finalize_conversion`] — branch on the pipeline result and call
//!   [`state::zip_and_persist`] on `Ok` or [`state::set_job_error`] on `Err`.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::Result;
use axum::{
    Json,
    extract::{Multipart, State},
    response::IntoResponse,
};
use geojson::GeoJson;
use serde_json::json;
use tokio::sync::OwnedSemaphorePermit;
use uuid::Uuid;

use crate::geojson_export;
use crate::osm;
use crate::params::{ConvertParams, TerrainParams};
use crate::pipeline::{
    run_conversion, run_conversion_preview, run_surface_preview, run_terrain_only_to_disk,
};

use super::error::ApiError;
use super::options::{
    Bounds, ConvertOptions, FetchConvertOptions, FetchConvertRequest, FetchPreviewRequest,
    OvertureConvertRequest, ParseResponse, ParseStats, PreviewBlock, PreviewBounds,
    PreviewResponse, PreviewSpawn, TerrainConvertRequest, build_filter,
    fetch_convert_elevation_phase_progress, fetch_convert_phase_progress,
    parse_fetch_convert_source_options, validate_bbox, validate_convert_options,
    validate_fetch_convert_options, validate_terrain_convert_options,
};
use super::state::{
    AppState, JobState, Jobs, lock_jobs, sanitize_world_name, set_job_error, zip_and_persist,
};

// ── Job-control helpers (QA-004) ───────────────────────────────────────────

/// Claim a semaphore slot, mint a job ID, insert the initial `Running`
/// state, and spawn `work` on a blocking thread. Returns the job ID on
/// success or an [`ApiError`] (HTTP 503-equivalent) when the concurrency
/// cap is exhausted.
///
/// `work` runs inside `tokio::task::spawn_blocking` and receives the
/// per-job `(Jobs, jid)` pair. The semaphore permit is moved into the
/// blocking closure and released when the closure returns, bounding the
/// number of simultaneously running conversions to
/// [`state::MAX_CONCURRENT_JOBS`].
fn spawn_conversion_job<F>(state: &AppState, work: F) -> Result<String, ApiError>
where
    F: FnOnce(Jobs, String) + Send + 'static,
{
    let permit = state.semaphore.clone().try_acquire_owned().map_err(|_| {
        anyhow::anyhow!("server is busy — too many concurrent conversions; retry later")
    })?;
    let job_id = Uuid::new_v4().to_string();
    {
        let mut jobs = lock_jobs(&state.jobs);
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
        // Hold the permit for the lifetime of the blocking work.
        let _permit: OwnedSemaphorePermit = permit;
        work(jobs, jid);
    });
    Ok(job_id)
}

/// Create the temp output directory, sanitise the world name, and
/// `create_dir_all` the world dir inside it.
///
/// On any failure, records a `"conversion failed"` [`JobState::Error`]
/// against `jid` and returns `None`; the caller should `return` from the
/// blocking closure so the permit is released.
fn prepare_world_dir(
    jobs: &Jobs,
    jid: &str,
    world_name_raw: &str,
    prefix: &str,
) -> Option<(tempfile::TempDir, PathBuf, String)> {
    let output_dir = match tempfile::Builder::new().prefix(prefix).tempdir() {
        Ok(d) => d,
        Err(e) => {
            set_job_error(
                jobs,
                jid,
                "conversion failed",
                format!("Failed to create output dir: {e}"),
            );
            return None;
        }
    };
    let world_name = sanitize_world_name(world_name_raw);
    let world_dir = output_dir.path().join(&world_name);
    if let Err(e) = std::fs::create_dir_all(&world_dir) {
        set_job_error(
            jobs,
            jid,
            "conversion failed",
            format!("Failed to create world dir: {e}"),
        );
        return None;
    }
    Some((output_dir, world_dir, world_name))
}

/// Branch on a pipeline result: on `Ok`, persist the archive via
/// [`zip_and_persist`]; on `Err`, record a generic `"conversion failed"`
/// error message.
fn finalize_conversion(
    jobs: &Jobs,
    jid: &str,
    result: std::result::Result<(), anyhow::Error>,
    output_dir: tempfile::TempDir,
    world_dir: &Path,
    world_name: &str,
    edition: crate::world::Edition,
) {
    match result {
        Ok(()) => zip_and_persist(jobs, jid, output_dir, world_dir, world_name, edition),
        Err(e) => set_job_error(
            jobs,
            jid,
            "conversion failed",
            format!("Conversion failed: {e}"),
        ),
    }
}

// ── Simple endpoints ───────────────────────────────────────────────────────

/// `GET /health` — liveness probe.
pub(crate) async fn health() -> impl IntoResponse {
    Json(json!({
        "status": "ok",
        "overture_available": crate::overture::is_cli_available()
    }))
}

/// `GET /cache/areas` — list all cached Overpass areas.
/// Never errors; returns an empty array if the cache dir is empty or doesn't exist.
pub(crate) async fn cache_areas_handler() -> impl IntoResponse {
    let entries = tokio::task::spawn_blocking(crate::osm_cache::list_areas)
        .await
        .unwrap_or_default();
    Json(entries)
}

// ── /parse + /fetch-preview (sync JSON responses) ──────────────────────────

/// `POST /parse` — accept a multipart upload with one or more `file` fields,
/// parse the PBF data, merge results, convert to GeoJSON, and return bounds + stats.
pub(crate) async fn parse_pbf_handler(
    mut multipart: Multipart,
) -> Result<impl IntoResponse, ApiError> {
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
        return Err(ApiError::bad_request("multipart field 'file' is missing"));
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

/// `POST /fetch-preview` — fetch OSM data from Overpass (cache-aware) and return
/// GeoJSON + bounds + stats, the same shape as `/parse`.
pub(crate) async fn fetch_preview_handler(
    Json(req): Json<FetchPreviewRequest>,
) -> Result<impl IntoResponse, ApiError> {
    // Validate bbox ranges + block-extent budget (preview uses scale=1.0).
    validate_bbox(req.bbox, 1.0).map_err(|e| ApiError::bad_request(e.to_string()))?;

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

// ── /convert (multipart upload) ────────────────────────────────────────────

/// `POST /convert` — accept a multipart upload with `file` and `options` fields.
/// Spawns a background conversion task and returns a job ID immediately.
pub(crate) async fn convert_handler(
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

    let bytes =
        file_bytes.ok_or_else(|| ApiError::bad_request("multipart field 'file' is missing"))?;
    if bytes.is_empty() {
        return Err(ApiError::bad_request("uploaded file is empty"));
    }

    let options: ConvertOptions = match options_str {
        Some(s) => {
            serde_json::from_str(&s).map_err(|e| anyhow::anyhow!("invalid options JSON: {e}"))?
        }
        None => ConvertOptions::default(),
    };

    // Validate numeric parameters before accepting the job.
    validate_convert_options(&options).map_err(|e| anyhow::anyhow!("{e}"))?;

    let job_id = spawn_conversion_job(&state, move |jobs, jid| {
        // Write PBF to temp file
        let tmp_file = match tempfile::Builder::new().suffix(".osm.pbf").tempfile() {
            Ok(mut f) => {
                if let Err(e) = f.write_all(&bytes).and_then(|_| f.flush()) {
                    set_job_error(
                        &jobs,
                        &jid,
                        "failed to process upload",
                        format!("Failed to write temp file: {e}"),
                    );
                    return;
                }
                f
            }
            Err(e) => {
                set_job_error(
                    &jobs,
                    &jid,
                    "failed to process upload",
                    format!("Failed to create temp file: {e}"),
                );
                return;
            }
        };
        let (_, tmp_path) = tmp_file.into_parts();

        let Some((output_dir, world_dir, world_name)) =
            prepare_world_dir(&jobs, &jid, &options.world_name, "osm-world-")
        else {
            return;
        };

        // Optional: download SRTM elevation tiles for the uploaded PBF's bbox.
        let elevation_dir = if options.use_elevation {
            match download_elevation_for_pbf(&tmp_path, &jobs, &jid) {
                Ok(dir) => Some(dir),
                Err(e) => {
                    set_job_error(
                        &jobs,
                        &jid,
                        "elevation download failed",
                        format!("Elevation download failed: {e}"),
                    );
                    return;
                }
            }
        } else {
            None
        };

        let params = ConvertParams {
            input: Some(tmp_path.to_path_buf()),
            output: world_dir.clone(),
            edition: options.edition,
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
            let mut map = lock_jobs(&jobs_for_progress);
            map.insert(
                jid_for_progress.clone(),
                JobState::Running {
                    progress,
                    message: msg.to_string(),
                },
            );
        });

        finalize_conversion(
            &jobs,
            &jid,
            result,
            output_dir,
            &world_dir,
            &world_name,
            options.edition,
        );
    })?;

    Ok(Json(json!({ "job_id": job_id })))
}

// ── /preview + /fetch-block-preview (sync JSON responses) ──────────────────

/// `POST /preview` — accept a multipart upload with `file` and optional
/// `options` fields, run the conversion in memory, and return the surface
/// block grid as JSON (downsampled if large).
pub(crate) async fn preview_handler(
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

    let bytes =
        file_bytes.ok_or_else(|| ApiError::bad_request("multipart field 'file' is missing"))?;
    if bytes.is_empty() {
        return Err(ApiError::bad_request("uploaded file is empty"));
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
            edition: options.edition,
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
pub(crate) async fn fetch_block_preview_handler(
    Json(req): Json<FetchPreviewRequest>,
) -> Result<impl IntoResponse, ApiError> {
    // Validate bbox ranges + block-extent budget (preview uses scale=1.0).
    validate_bbox(req.bbox, 1.0).map_err(|e| ApiError::bad_request(e.to_string()))?;

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
            edition: Default::default(),
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

// ── /fetch-convert (Overpass → conversion) ─────────────────────────────────

/// `POST /fetch-convert` — fetch OSM data from Overpass and convert to .mcworld.
/// Request body is JSON (not multipart).
pub(crate) async fn fetch_convert_handler(
    State(state): State<AppState>,
    Json(req): Json<FetchConvertRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let bbox = (req.bbox[0], req.bbox[1], req.bbox[2], req.bbox[3]);

    // Validate bbox ranges + block-extent budget before acquiring the semaphore
    // so abusive bboxes fail fast with 400 instead of consuming a job slot.
    validate_bbox(req.bbox, req.options.scale).map_err(|e| ApiError::bad_request(e.to_string()))?;

    // Validate request parameters before accepting the job, so invalid source
    // controls fail the request immediately instead of becoming async job errors.
    validate_fetch_convert_options(&req.options)
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
    let parsed_source_options = parse_fetch_convert_source_options(&req)
        .map_err(|e| ApiError::bad_request(format!("{e:#}")))?;

    let filter = req.filter;
    let options = req.options;
    let force_refresh = req.force_refresh;
    let overpass_url = req
        .overpass_url
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned);
    let req_overture = req.overture;
    let req_overture_timeout = req.overture_timeout;
    let req_use_elevation = options.use_elevation;

    let job_id = spawn_conversion_job(&state, move |jobs, jid| {
        let source_options = crate::params::SourceOptions {
            filter: filter.clone(),
            overpass_url,
            use_overpass_cache: !force_refresh,
            overture: crate::params::OvertureParams {
                enabled: req_overture,
                themes: parsed_source_options.themes,
                priority: parsed_source_options.priority,
                timeout_secs: req_overture_timeout,
            },
            poi_source_mode: if req_overture {
                parsed_source_options.requested_poi_source_mode
            } else {
                crate::params::PoiSourceMode::OsmOnly
            },
            overture_failure_mode: parsed_source_options.overture_failure_mode,
        };
        let jobs_fetch = jobs.clone();
        let jid_fetch = jid.clone();
        let source_result = match par_osm_rust::sources::fetch_map_data(
            bbox,
            &source_options,
            &mut |progress, msg| {
                let mut map = lock_jobs(&jobs_fetch);
                map.insert(
                    jid_fetch.clone(),
                    JobState::Running {
                        progress: progress * 0.3,
                        message: msg.to_string(),
                    },
                );
            },
        ) {
            Ok(result) => result,
            Err(e) => {
                set_job_error(
                    &jobs,
                    &jid,
                    "map data fetch failed",
                    format!("Map data fetch failed: {e}"),
                );
                return;
            }
        };
        for warning in &source_result.warnings {
            log::warn!("{warning}");
        }
        let data = source_result.data;

        let Some((output_dir, world_dir, world_name)) =
            prepare_world_dir(&jobs, &jid, &options.world_name, "osm-world-")
        else {
            return;
        };

        // Optional: download SRTM elevation tiles for the requested bbox.
        let elevation_dir = if options.use_elevation {
            match download_elevation_for_bbox_mapped(
                bbox.0,
                bbox.1,
                bbox.2,
                bbox.3,
                &jobs,
                &jid,
                fetch_convert_elevation_phase_progress,
            ) {
                Ok(dir) => Some(dir),
                Err(e) => {
                    set_job_error(
                        &jobs,
                        &jid,
                        "elevation download failed",
                        format!("Elevation download failed: {e}"),
                    );
                    return;
                }
            }
        } else {
            None
        };

        let params = ConvertParams {
            input: None,
            output: world_dir.clone(),
            edition: options.edition,
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
            let mut map = lock_jobs(&jobs_for_progress);
            map.insert(
                jid_for_progress.clone(),
                JobState::Running {
                    progress: fetch_convert_phase_progress(progress, req_use_elevation),
                    message: msg.to_string(),
                },
            );
        });

        finalize_conversion(
            &jobs,
            &jid,
            result,
            output_dir,
            &world_dir,
            &world_name,
            options.edition,
        );
    })?;

    Ok(Json(json!({ "job_id": job_id })))
}

// ── /terrain-convert (SRTM-only world) ─────────────────────────────────────

/// `POST /terrain-convert` — generate a terrain-only world from SRTM elevation.
/// Accepts a JSON body with `bbox` and `options`; returns a job ID immediately.
pub(crate) async fn terrain_convert_handler(
    State(state): State<AppState>,
    Json(req): Json<TerrainConvertRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let bbox = (req.bbox[0], req.bbox[1], req.bbox[2], req.bbox[3]);
    let options = req.options;

    // Validate bbox + scale before acquiring the semaphore.
    validate_bbox(req.bbox, options.scale).map_err(|e| ApiError::bad_request(e.to_string()))?;

    // Validate numeric parameters before accepting the job.
    validate_terrain_convert_options(&options).map_err(|e| anyhow::anyhow!("{e}"))?;

    let job_id = spawn_conversion_job(&state, move |jobs, jid| {
        // Download SRTM elevation tiles when requested.
        let elevation_path = if options.use_elevation {
            match download_elevation_for_bbox(bbox.0, bbox.1, bbox.2, bbox.3, &jobs, &jid) {
                Ok(dir) => Some(dir),
                Err(e) => {
                    set_job_error(
                        &jobs,
                        &jid,
                        "elevation download failed",
                        format!("Elevation download failed: {e}"),
                    );
                    return;
                }
            }
        } else {
            None
        };

        let Some((output_dir, world_dir, world_name)) =
            prepare_world_dir(&jobs, &jid, &options.world_name, "terrain-world-")
        else {
            return;
        };

        let params = TerrainParams {
            bbox,
            output: world_dir.clone(),
            edition: options.edition,
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
            let mut map = lock_jobs(&jobs_for_progress);
            map.insert(
                jid_for_progress.clone(),
                JobState::Running {
                    progress,
                    message: msg.to_string(),
                },
            );
        });

        match result {
            Ok(()) => zip_and_persist(
                &jobs,
                &jid,
                output_dir,
                &world_dir,
                &world_name,
                options.edition,
            ),
            Err(e) => set_job_error(
                &jobs,
                &jid,
                "terrain generation failed",
                format!("Terrain generation failed: {e:#}"),
            ),
        }
    })?;

    Ok(Json(json!({ "job_id": job_id })))
}

// ── /overture-convert (Overture-only → conversion) ─────────────────────────

/// `POST /overture-convert` — fetch Overture Maps data and convert to .mcworld.
/// Request body is JSON. Returns a job ID immediately.
pub(crate) async fn overture_convert_handler(
    State(state): State<AppState>,
    Json(req): Json<OvertureConvertRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let bbox = (req.bbox[0], req.bbox[1], req.bbox[2], req.bbox[3]);
    let options: FetchConvertOptions = req.options;
    let themes_raw = req.themes;
    let timeout_secs = req.timeout;

    // Validate bbox + scale before acquiring the semaphore.
    validate_bbox(req.bbox, options.scale).map_err(|e| ApiError::bad_request(e.to_string()))?;

    // Validate numeric parameters before accepting the job.
    validate_fetch_convert_options(&options).map_err(|e| anyhow::anyhow!("{e}"))?;

    let job_id = spawn_conversion_job(&state, move |jobs, jid| {
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
                let mut map = lock_jobs(&jobs_ov);
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
                set_job_error(
                    &jobs,
                    &jid,
                    "overture fetch failed",
                    format!("Overture fetch failed: {e}"),
                );
                return;
            }
        };

        // Check if any data was actually returned.
        if data.ways.is_empty() && data.poi_nodes.is_empty() && data.addr_nodes.is_empty() {
            set_job_error(
                &jobs,
                &jid,
                "no overture data found for this area",
                "No Overture data found for this area",
            );
            return;
        }

        let Some((output_dir, world_dir, world_name)) =
            prepare_world_dir(&jobs, &jid, &options.world_name, "osm-world-")
        else {
            return;
        };

        // Optional: download SRTM elevation tiles for the requested bbox.
        let elevation_dir = if options.use_elevation {
            match download_elevation_for_bbox(bbox.0, bbox.1, bbox.2, bbox.3, &jobs, &jid) {
                Ok(dir) => Some(dir),
                Err(e) => {
                    set_job_error(
                        &jobs,
                        &jid,
                        "elevation download failed",
                        format!("Elevation download failed: {e}"),
                    );
                    return;
                }
            }
        } else {
            None
        };

        let params = ConvertParams {
            input: None,
            output: world_dir.clone(),
            edition: options.edition,
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
            let mut map = lock_jobs(&jobs_for_progress);
            map.insert(
                jid_for_progress.clone(),
                JobState::Running {
                    progress: 0.3 + progress * 0.6,
                    message: msg.to_string(),
                },
            );
        });

        finalize_conversion(
            &jobs,
            &jid,
            result,
            output_dir,
            &world_dir,
            &world_name,
            options.edition,
        );
    })?;

    Ok(Json(json!({ "job_id": job_id })))
}

// ── /status + /download (job-state reads) ──────────────────────────────────

/// `GET /status/{id}` — poll conversion progress.
pub(crate) async fn status_handler(
    State(state): State<AppState>,
    axum::extract::Path(job_id): axum::extract::Path<String>,
) -> Result<impl IntoResponse, axum::response::Response> {
    let jobs = lock_jobs(&state.jobs);
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
        Some(JobState::Error { public_message, .. }) => Ok(Json(json!({
            "state": "error",
            "progress": 0.0,
            "message": public_message,
        }))),
        None => Err((
            axum::http::StatusCode::NOT_FOUND,
            Json(json!({ "error": "unknown job ID" })),
        )
            .into_response()),
    }
}

/// `GET /download/{id}` — serve the `.mcworld` file for a completed job.
pub(crate) async fn download_handler(
    State(state): State<AppState>,
    axum::extract::Path(job_id): axum::extract::Path<String>,
) -> Result<axum::response::Response, axum::response::Response> {
    let path = {
        let jobs = lock_jobs(&state.jobs);
        match jobs.get(&job_id) {
            Some(JobState::Done { path, .. }) => path.clone(),
            Some(JobState::Running { .. }) => {
                return Err((
                    axum::http::StatusCode::CONFLICT,
                    Json(json!({ "error": "conversion still in progress" })),
                )
                    .into_response());
            }
            Some(JobState::Error { public_message, .. }) => {
                return Err((
                    axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                    Json(json!({ "error": public_message })),
                )
                    .into_response());
            }
            None => {
                return Err((
                    axum::http::StatusCode::NOT_FOUND,
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
        Err(e) => {
            log::error!("Failed to read mcworld file {}: {e}", path.display());
            Err((
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "failed to read mcworld file" })),
            )
                .into_response())
        }
    }
}

// ── Elevation download helpers ─────────────────────────────────────────────

/// Download SRTM tiles covering the bounding box of an uploaded PBF file
/// into the persistent cache directory.  Returns the cache `PathBuf`.
fn download_elevation_for_pbf(pbf_path: &Path, jobs: &Jobs, jid: &str) -> anyhow::Result<PathBuf> {
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
) -> anyhow::Result<PathBuf> {
    download_elevation_for_bbox_mapped(min_lat, min_lon, max_lat, max_lon, jobs, jid, |progress| {
        progress * 0.2
    })
}

fn download_elevation_for_bbox_mapped(
    min_lat: f64,
    min_lon: f64,
    max_lat: f64,
    max_lon: f64,
    jobs: &Jobs,
    jid: &str,
    map_progress: impl Fn(f32) -> f32,
) -> anyhow::Result<PathBuf> {
    let cache = crate::srtm::cache_dir();
    log::info!("SRTM cache: {}", cache.display());
    crate::srtm::download_tiles_for_bbox(
        min_lat,
        min_lon,
        max_lat,
        max_lon,
        &cache,
        &|i, total: usize, name| {
            let mut jobs = lock_jobs(jobs);
            jobs.insert(
                jid.to_string(),
                JobState::Running {
                    progress: map_progress(i as f32 / total.max(1) as f32),
                    message: format!("Downloading elevation tile {name} ({}/{total})", i + 1),
                },
            );
        },
    )?;
    Ok(cache)
}
