//! Request/response shapes and validation for the HTTP API.
//!
//! This module owns every type that crosses the wire: the `*Request` /
//! `*Options` / `*Response` structs, the `default_*` serde functions they
//! reference, the [`validate_bbox`] / `validate_*_options` guards that
//! reject out-of-range or continent-scale inputs up front (SEC-004), and the
//! source-option parsing helpers used by `/fetch-convert` and
//! `/overture-convert` (Overture themes/priorities/POI modes).
//!
//! The [`phase_progress`] helpers translate inner-pipeline progress reports
//! into the outer per-job progress that `/status` returns.

use std::collections::HashMap;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

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

// ── ConvertOptions (`/convert` + `/preview`) ───────────────────────────────

/// Conversion options sent as JSON in the multipart `options` field.
#[derive(Debug, Deserialize)]
pub(crate) struct ConvertOptions {
    #[serde(default = "default_scale")]
    pub(crate) scale: f64,
    #[serde(default = "default_sea_level")]
    pub(crate) sea_level: i32,
    #[serde(default = "default_building_height")]
    pub(crate) building_height: i32,
    #[serde(default = "default_wall_straighten_threshold")]
    pub(crate) wall_straighten_threshold: i32,
    #[serde(default = "default_world_name")]
    pub(crate) world_name: String,
    /// Explicit spawn block coordinates — take priority over spawn_lat/lon.
    pub(crate) spawn_x: Option<i32>,
    pub(crate) spawn_y: Option<i32>,
    pub(crate) spawn_z: Option<i32>,
    /// Spawn position as geographic coordinates — converted to block coords by the converter.
    pub(crate) spawn_lat: Option<f64>,
    pub(crate) spawn_lon: Option<f64>,
    #[serde(default)]
    pub(crate) signs: Option<bool>,
    #[serde(default)]
    pub(crate) address_signs: Option<bool>,
    #[serde(default)]
    pub(crate) poi_markers: Option<bool>,
    // Feature filter fields
    #[serde(default = "default_true")]
    pub(crate) roads: bool,
    #[serde(default = "default_true")]
    pub(crate) buildings: bool,
    #[serde(default = "default_true")]
    pub(crate) water: bool,
    #[serde(default = "default_true")]
    pub(crate) landuse: bool,
    #[serde(default = "default_true")]
    pub(crate) railways: bool,
    #[serde(default)]
    pub(crate) use_elevation: bool,
    #[serde(default = "default_vertical_scale")]
    pub(crate) vertical_scale: f64,
    #[serde(default = "default_elevation_smoothing")]
    pub(crate) elevation_smoothing: i32,
    #[serde(default = "default_surface_thickness")]
    pub(crate) surface_thickness: i32,
    #[serde(default = "default_true")]
    pub(crate) poi_decorations: bool,
    #[serde(default = "default_true")]
    pub(crate) nature_decorations: bool,
    #[serde(default)]
    pub(crate) edition: crate::world::Edition,
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
            edition: Default::default(),
        }
    }
}

// ── FetchConvert (`/fetch-convert` + `/overture-convert`) ──────────────────

/// Request body for `POST /fetch-convert`.
#[derive(Debug, Deserialize)]
pub(crate) struct FetchConvertRequest {
    /// Bounding box [south, west, north, east].
    pub(crate) bbox: [f64; 4],
    #[serde(default)]
    pub(crate) options: FetchConvertOptions,
    #[serde(default)]
    pub(crate) filter: crate::filter::FeatureFilter,
    /// If true, bypass cache read — always fetch from Overpass but still write result to cache.
    #[serde(default)]
    pub(crate) force_refresh: bool,
    /// Optional Overpass API URL override. Falls back to OVERPASS_URL env var or default.
    #[serde(default)]
    pub(crate) overpass_url: Option<String>,
    /// If true, also fetch and merge Overture Maps data.
    #[serde(default)]
    pub(crate) overture: bool,
    /// Overture themes to fetch (empty = all themes).
    #[serde(default)]
    pub(crate) overture_themes: Vec<String>,
    /// Per-theme priority override map (theme name → "overture" | "osm" | "both").
    #[serde(default)]
    pub(crate) overture_priority: HashMap<String, String>,
    /// Timeout in seconds for the overturemaps CLI subprocess.
    #[serde(default = "default_overture_timeout")]
    pub(crate) overture_timeout: u64,
    /// POI source mode: osm-only, overture-only, both, or overture-preferred.
    #[serde(default)]
    pub(crate) poi_source: Option<String>,
    /// Overture failure behavior: fallback-to-osm or fail.
    #[serde(default)]
    pub(crate) overture_failure: Option<String>,
}

/// Conversion options for `POST /fetch-convert` (also reused by
/// `/overture-convert`).
#[derive(Debug, Deserialize)]
pub(crate) struct FetchConvertOptions {
    #[serde(default = "default_scale")]
    pub(crate) scale: f64,
    #[serde(default = "default_sea_level")]
    pub(crate) sea_level: i32,
    #[serde(default = "default_building_height")]
    pub(crate) building_height: i32,
    #[serde(default = "default_wall_straighten_threshold")]
    pub(crate) wall_straighten_threshold: i32,
    #[serde(default = "default_world_name")]
    pub(crate) world_name: String,
    pub(crate) spawn_x: Option<i32>,
    pub(crate) spawn_y: Option<i32>,
    pub(crate) spawn_z: Option<i32>,
    pub(crate) spawn_lat: Option<f64>,
    pub(crate) spawn_lon: Option<f64>,
    #[serde(default)]
    pub(crate) signs: Option<bool>,
    #[serde(default)]
    pub(crate) address_signs: Option<bool>,
    #[serde(default)]
    pub(crate) poi_markers: Option<bool>,
    #[serde(default)]
    pub(crate) use_elevation: bool,
    #[serde(default = "default_vertical_scale")]
    pub(crate) vertical_scale: f64,
    #[serde(default = "default_elevation_smoothing")]
    pub(crate) elevation_smoothing: i32,
    #[serde(default = "default_surface_thickness")]
    pub(crate) surface_thickness: i32,
    #[serde(default = "default_true")]
    pub(crate) poi_decorations: bool,
    #[serde(default = "default_true")]
    pub(crate) nature_decorations: bool,
    #[serde(default)]
    pub(crate) edition: crate::world::Edition,
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
            edition: Default::default(),
        }
    }
}

// ── TerrainConvert (`/terrain-convert`) ────────────────────────────────────

/// Request body for `POST /terrain-convert`.
#[derive(Debug, Deserialize)]
pub(crate) struct TerrainConvertRequest {
    /// Bounding box [south, west, north, east].
    pub(crate) bbox: [f64; 4],
    #[serde(default)]
    pub(crate) options: TerrainConvertOptions,
}

/// Conversion options for `POST /terrain-convert`.
#[derive(Debug, Deserialize)]
pub(crate) struct TerrainConvertOptions {
    #[serde(default = "default_world_name")]
    pub(crate) world_name: String,
    #[serde(default = "default_scale")]
    pub(crate) scale: f64,
    #[serde(default = "default_sea_level")]
    pub(crate) sea_level: i32,
    #[serde(default = "default_vertical_scale")]
    pub(crate) vertical_scale: f64,
    #[serde(default = "default_snow_line")]
    pub(crate) snow_line: i32,
    #[serde(default = "default_elevation_smoothing")]
    pub(crate) elevation_smoothing: i32,
    #[serde(default = "default_surface_thickness")]
    pub(crate) surface_thickness: i32,
    pub(crate) spawn_x: Option<i32>,
    pub(crate) spawn_y: Option<i32>,
    pub(crate) spawn_z: Option<i32>,
    pub(crate) spawn_lat: Option<f64>,
    pub(crate) spawn_lon: Option<f64>,
    /// When true, auto-download SRTM tiles for the bbox. Defaults to true.
    #[serde(default = "default_true")]
    pub(crate) use_elevation: bool,
    #[serde(default)]
    pub(crate) edition: crate::world::Edition,
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
            edition: Default::default(),
        }
    }
}

// ── OvertureConvert (`/overture-convert`) ──────────────────────────────────

/// Request body for `POST /overture-convert`.
#[derive(Debug, Deserialize)]
pub(crate) struct OvertureConvertRequest {
    /// Bounding box [south, west, north, east].
    pub(crate) bbox: [f64; 4],
    #[serde(default)]
    pub(crate) options: FetchConvertOptions,
    /// Overture themes to fetch (empty = all themes).
    #[serde(default)]
    pub(crate) themes: Vec<String>,
    /// Timeout in seconds for the overturemaps CLI subprocess.
    #[serde(default = "default_overture_timeout")]
    pub(crate) timeout: u64,
}

// ── FetchPreview (`/fetch-preview` + `/fetch-block-preview`) ───────────────

/// Request body for `POST /fetch-preview`.
#[derive(Debug, Deserialize)]
pub(crate) struct FetchPreviewRequest {
    /// Bounding box [south, west, north, east].
    pub(crate) bbox: [f64; 4],
    #[serde(default)]
    pub(crate) filter: crate::filter::FeatureFilter,
    /// Optional Overpass API URL override.
    #[serde(default)]
    pub(crate) overpass_url: Option<String>,
}

// ── Preview response types (`/preview` + `/fetch-block-preview`) ───────────

/// A single surface block in the preview response.
#[derive(Debug, Serialize)]
pub(crate) struct PreviewBlock {
    pub(crate) x: i32,
    pub(crate) z: i32,
    pub(crate) y: i32,
    #[serde(rename = "type")]
    pub(crate) block_type: String,
}

/// Bounding box in block coordinates for the preview response.
#[derive(Debug, Serialize)]
pub(crate) struct PreviewBounds {
    pub(crate) min_x: i32,
    pub(crate) max_x: i32,
    pub(crate) min_z: i32,
    pub(crate) max_z: i32,
}

#[derive(Debug, Serialize)]
pub(crate) struct PreviewSpawn {
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) z: i32,
}

/// The full response body returned by `POST /preview`.
#[derive(Debug, Serialize)]
pub(crate) struct PreviewResponse {
    pub(crate) blocks: Vec<PreviewBlock>,
    pub(crate) bounds: PreviewBounds,
    pub(crate) spawn: PreviewSpawn,
}

// ── serde default functions ────────────────────────────────────────────────
//
// These remain as free functions (rather than `Default` impls on each field
// type) because each one returns a non-`T::default()` value (e.g. scale=1.0,
// sea_level=65) and `#[serde(default)]` on a field would invoke
// `T::default()` (0.0, 0). The struct-level `Default` impls above call
// these same functions, so the two paths stay in sync.

pub(crate) fn default_scale() -> f64 {
    1.0
}
pub(crate) fn default_sea_level() -> i32 {
    65
}
pub(crate) fn default_building_height() -> i32 {
    8
}
pub(crate) fn default_wall_straighten_threshold() -> i32 {
    1
}
pub(crate) fn default_elevation_smoothing() -> i32 {
    1
}
pub(crate) fn default_surface_thickness() -> i32 {
    4
}
pub(crate) fn default_world_name() -> String {
    "OSM World".to_string()
}
pub(crate) fn default_true() -> bool {
    true
}
pub(crate) fn default_vertical_scale() -> f64 {
    1.0
}
pub(crate) fn default_overture_timeout() -> u64 {
    120
}
pub(crate) fn default_snow_line() -> i32 {
    80
}

// ── Validation ─────────────────────────────────────────────────────────────

/// Approximate equatorial meters-per-degree for the equirectangular projection
/// used by `CoordConverter`. We use the equatorial value as a conservative
/// upper bound — at higher latitudes a degree of longitude is shorter, so
/// using the equator overestimates the resulting block count, which is the
/// safe direction for a guardrail.
const METERS_PER_DEGREE: f64 = 111_320.0;

/// Maximum block extent per axis that the in-memory conversion pipeline can
/// handle without risking OOM or runaway rasterisation time.
///
/// 250_000 blocks per axis ≈ 15_625 chunk-columns per axis. This is a
/// coarse abuse-guardrail that rejects obvious overreach (continent- and
/// country-spanning bboxes, scale-bumped metro extracts) up front, before
/// any work starts. It is layered on top of, not in lieu of, ARC-001's
/// edition-specific Java memory guard in
/// [`osm_to_bedrock::world::enforce_java_memory_budget`]: even when this
/// coarse check passes, the pipeline still refuses Java conversions whose
/// estimated chunk count exceeds the ~1.5 GB in-memory budget (Java has no
/// streaming Anvil writer yet).
///
/// At `scale = 1` (default): max bbox span ≈ 2.25° per axis (≈ 250 km at the
/// equator) — permits every example bbox in `README.md` / `docs/CLI.md`
/// (central London 0.03°, Paris 0.03°, Mt Rainier terrain 1°).
/// At `scale = 100` (max): max bbox span ≈ 0.0225° per axis (≈ 2.5 km).
const MAX_BLOCK_EXTENT: f64 = 250_000.0;

/// Validate an OSM bounding box (`[south, west, north, east]`) for range,
/// ordering, finite-ness, and resulting block extent at the given `scale`.
///
/// Returns `Err(&'static str)` suitable for an HTTP 400 body when the bbox
/// is malformed or would produce an unsafe block count.
pub(crate) fn validate_bbox(bbox: [f64; 4], scale: f64) -> Result<(), &'static str> {
    let [south, west, north, east] = bbox;
    if !south.is_finite()
        || !west.is_finite()
        || !north.is_finite()
        || !east.is_finite()
        || !scale.is_finite()
    {
        return Err("bbox and scale must be finite numbers");
    }
    if !(-90.0..=90.0).contains(&south) || !(-90.0..=90.0).contains(&north) {
        return Err("latitudes must be in range -90 .. 90");
    }
    if !(-180.0..=180.0).contains(&west) || !(-180.0..=180.0).contains(&east) {
        return Err("longitudes must be in range -180 .. 180");
    }
    if south > north {
        return Err("south latitude must be <= north latitude");
    }
    if west > east {
        return Err("west longitude must be <= east longitude");
    }
    // Block extent = span_degrees * meters_per_degree * scale.
    // Reject any bbox that would exceed MAX_BLOCK_EXTENT per axis.
    let lat_blocks = (north - south) * METERS_PER_DEGREE * scale;
    let lon_blocks = (east - west) * METERS_PER_DEGREE * scale;
    if lat_blocks > MAX_BLOCK_EXTENT || lon_blocks > MAX_BLOCK_EXTENT {
        return Err(
            "bbox × scale exceeds maximum supported block extent (250000 blocks per axis); \
             reduce the bounding box or lower scale",
        );
    }
    Ok(())
}

/// Validate numeric bounds on `ConvertOptions`.
///
/// Returns an error string if any value is outside the accepted range.
/// Prevents crafted inputs like `scale = 1e300` from causing near-infinite
/// rasterization loops or memory exhaustion.
pub(crate) fn validate_convert_options(opts: &ConvertOptions) -> Result<(), &'static str> {
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

/// Validate numeric bounds on `FetchConvertOptions`.
pub(crate) fn validate_fetch_convert_options(
    opts: &FetchConvertOptions,
) -> Result<(), &'static str> {
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
pub(crate) fn validate_terrain_convert_options(
    opts: &TerrainConvertOptions,
) -> Result<(), &'static str> {
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

pub(crate) fn build_filter(opts: &ConvertOptions) -> crate::filter::FeatureFilter {
    crate::filter::FeatureFilter {
        roads: opts.roads,
        buildings: opts.buildings,
        water: opts.water,
        landuse: opts.landuse,
        railways: opts.railways,
    }
}

// ── Source-option parsing (Overture themes/priorities/POI modes) ───────────

pub(crate) fn parse_poi_source_mode_for_server(
    value: Option<&str>,
) -> Result<crate::params::PoiSourceMode> {
    crate::source_options::parse_poi_source_mode(value.unwrap_or("overture-preferred"))
}

pub(crate) fn parse_overture_failure_mode_for_server(
    value: Option<&str>,
) -> Result<crate::params::OvertureFailureMode> {
    crate::source_options::parse_overture_failure_mode(value.unwrap_or("fallback-to-osm"))
}

#[derive(Debug)]
pub(crate) struct ParsedFetchConvertSourceOptions {
    pub(crate) themes: Vec<crate::params::OvertureTheme>,
    pub(crate) priority: HashMap<crate::params::OvertureTheme, crate::params::ThemePriority>,
    pub(crate) requested_poi_source_mode: crate::params::PoiSourceMode,
    pub(crate) overture_failure_mode: crate::params::OvertureFailureMode,
}

pub(crate) fn parse_fetch_convert_source_options(
    req: &FetchConvertRequest,
) -> Result<ParsedFetchConvertSourceOptions> {
    Ok(ParsedFetchConvertSourceOptions {
        themes: crate::source_options::parse_overture_theme_list(&req.overture_themes)
            .context("Invalid overture_themes")?,
        priority: crate::source_options::parse_overture_priority_map(&req.overture_priority)
            .context("Invalid overture_priority")?,
        requested_poi_source_mode: parse_poi_source_mode_for_server(req.poi_source.as_deref())
            .context("Invalid POI source mode")?,
        overture_failure_mode: parse_overture_failure_mode_for_server(
            req.overture_failure.as_deref(),
        )
        .context("Invalid Overture failure mode")?,
    })
}

// ── Progress mapping helpers ───────────────────────────────────────────────

pub(crate) fn phase_progress(start: f32, end: f32, progress: f32) -> f32 {
    start + progress.clamp(0.0, 1.0) * (end - start)
}

pub(crate) fn fetch_convert_elevation_phase_progress(elevation_progress: f32) -> f32 {
    phase_progress(0.3, 0.45, elevation_progress)
}

pub(crate) fn fetch_convert_phase_progress(
    conversion_progress: f32,
    elevation_enabled: bool,
) -> f32 {
    if elevation_enabled {
        phase_progress(0.45, 1.0, conversion_progress)
    } else {
        phase_progress(0.3, 1.0, conversion_progress)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        FetchConvertRequest, fetch_convert_elevation_phase_progress, fetch_convert_phase_progress,
        parse_fetch_convert_source_options, parse_overture_failure_mode_for_server,
        parse_poi_source_mode_for_server, validate_bbox,
    };
    use crate::params::{OvertureFailureMode, PoiSourceMode};

    #[test]
    fn fetch_convert_source_options_deserialize_from_top_level() {
        let req: FetchConvertRequest = serde_json::from_value(serde_json::json!({
            "bbox": [51.5, -0.13, 51.52, -0.10],
            "poi_source": "both",
            "overture_failure": "strict",
            "options": {
                "world_name": "Source Test"
            }
        }))
        .unwrap();

        assert_eq!(req.poi_source.as_deref(), Some("both"));
        assert_eq!(req.overture_failure.as_deref(), Some("strict"));
    }

    #[test]
    fn server_source_mode_parser_accepts_aliases_and_rejects_bad_values() {
        assert_eq!(
            parse_poi_source_mode_for_server(Some("osm")).unwrap(),
            PoiSourceMode::OsmOnly
        );
        assert_eq!(
            parse_poi_source_mode_for_server(Some("overture_only")).unwrap(),
            PoiSourceMode::OvertureOnly
        );
        assert_eq!(
            parse_poi_source_mode_for_server(Some("preferred")).unwrap(),
            PoiSourceMode::OverturePreferred
        );
        assert!(parse_poi_source_mode_for_server(Some("bad-source")).is_err());
    }

    #[test]
    fn server_failure_mode_parser_accepts_aliases_and_rejects_bad_values() {
        assert_eq!(
            parse_overture_failure_mode_for_server(Some("fallback")).unwrap(),
            OvertureFailureMode::FallbackToOsm
        );
        assert_eq!(
            parse_overture_failure_mode_for_server(Some("strict")).unwrap(),
            OvertureFailureMode::Fail
        );
        assert!(parse_overture_failure_mode_for_server(Some("ignore")).is_err());
    }

    #[test]
    fn fetch_convert_phase_progress_maps_conversion_after_fetch_window_without_elevation() {
        assert_eq!(fetch_convert_phase_progress(0.0, false), 0.3);
        assert_eq!(fetch_convert_phase_progress(0.5, false), 0.65);
        assert_eq!(fetch_convert_phase_progress(1.0, false), 1.0);
        assert_eq!(fetch_convert_phase_progress(-0.5, false), 0.3);
        assert_eq!(fetch_convert_phase_progress(1.5, false), 1.0);
    }

    #[test]
    fn fetch_convert_phase_progress_maps_elevation_and_conversion_monotonically() {
        assert_eq!(fetch_convert_elevation_phase_progress(0.0), 0.3);
        assert_eq!(fetch_convert_elevation_phase_progress(0.5), 0.375);
        assert_eq!(fetch_convert_elevation_phase_progress(1.0), 0.45);
        assert_eq!(fetch_convert_phase_progress(0.0, true), 0.45);
        assert_eq!(fetch_convert_phase_progress(0.5, true), 0.725);
        assert_eq!(fetch_convert_phase_progress(1.0, true), 1.0);
    }

    #[test]
    fn fetch_convert_source_options_reject_invalid_overture_themes() {
        let req: FetchConvertRequest = serde_json::from_value(serde_json::json!({
            "bbox": [51.5, -0.13, 51.52, -0.10],
            "overture_themes": ["building", "not-a-theme"]
        }))
        .unwrap();

        let err = format!(
            "{:#}",
            parse_fetch_convert_source_options(&req).unwrap_err()
        );

        assert!(err.contains("unknown Overture theme 'not-a-theme'"));
    }

    #[test]
    fn fetch_convert_source_options_reject_invalid_overture_priority_theme() {
        let req: FetchConvertRequest = serde_json::from_value(serde_json::json!({
            "bbox": [51.5, -0.13, 51.52, -0.10],
            "overture_priority": {
                "not-a-theme": "osm"
            }
        }))
        .unwrap();

        let err = format!(
            "{:#}",
            parse_fetch_convert_source_options(&req).unwrap_err()
        );

        assert!(err.contains("unknown Overture theme 'not-a-theme'"));
    }

    #[test]
    fn fetch_convert_source_options_reject_invalid_overture_priority_value() {
        let req: FetchConvertRequest = serde_json::from_value(serde_json::json!({
            "bbox": [51.5, -0.13, 51.52, -0.10],
            "overture_priority": {
                "building": "bad-priority"
            }
        }))
        .unwrap();

        let err = format!(
            "{:#}",
            parse_fetch_convert_source_options(&req).unwrap_err()
        );

        assert!(err.contains("unknown priority 'bad-priority'"));
    }

    // ── SEC-004: validate_bbox ────────────────────────────────────────────

    #[test]
    fn validate_bbox_accepts_typical_metropolitan_bbox() {
        // Central London extract from README.md, at default scale=1.
        assert!(validate_bbox([51.50, -0.13, 51.52, -0.10], 1.0).is_ok());
    }

    #[test]
    fn validate_bbox_accepts_paris_extract() {
        // Paris extract from docs/CLI.md, at default scale=1.
        assert!(validate_bbox([48.85, 2.33, 48.87, 2.36], 1.0).is_ok());
    }

    #[test]
    fn validate_bbox_accepts_mt_rainier_terrain_extract() {
        // 1° × 1° terrain example from README.md at scale=1.
        assert!(validate_bbox([47.0, -122.5, 48.0, -121.5], 1.0).is_ok());
    }

    #[test]
    fn validate_bbox_rejects_latitude_out_of_range() {
        assert!(validate_bbox([91.0, 0.0, 92.0, 1.0], 1.0).is_err());
        assert!(validate_bbox([-91.0, 0.0, -90.0, 1.0], 1.0).is_err());
    }

    #[test]
    fn validate_bbox_rejects_longitude_out_of_range() {
        assert!(validate_bbox([0.0, -181.0, 1.0, -180.0], 1.0).is_err());
        assert!(validate_bbox([0.0, 180.0, 1.0, 181.0], 1.0).is_err());
    }

    #[test]
    fn validate_bbox_rejects_inverted_ordering() {
        assert!(validate_bbox([10.0, 0.0, 5.0, 1.0], 1.0).is_err()); // south > north
        assert!(validate_bbox([0.0, 10.0, 1.0, 5.0], 1.0).is_err()); // west > east
    }

    #[test]
    fn validate_bbox_rejects_non_finite() {
        assert!(validate_bbox([f64::NAN, 0.0, 1.0, 1.0], 1.0).is_err());
        assert!(validate_bbox([0.0, f64::INFINITY, 1.0, 1.0], 1.0).is_err());
        assert!(validate_bbox([0.0, 0.0, 1.0, 1.0], f64::NAN).is_err());
    }

    #[test]
    fn validate_bbox_rejects_continent_scale_at_scale_1() {
        // 50° × 50° span — continent-scale, clearly abusive.
        let err = validate_bbox([0.0, 0.0, 50.0, 50.0], 1.0).unwrap_err();
        assert!(err.contains("exceeds maximum"));
    }

    #[test]
    fn validate_bbox_rejects_world_span() {
        // Whole-world bbox — must be rejected.
        assert!(validate_bbox([-90.0, -180.0, 90.0, 180.0], 1.0).is_err());
    }

    #[test]
    fn validate_bbox_rejects_metropolitan_at_max_scale() {
        // A bbox that's fine at scale=1 becomes unsafe at scale=100
        // (scale=100 multiplies the block count by 100 per axis).
        assert!(validate_bbox([51.50, -0.13, 51.52, -0.10], 100.0).is_err());
    }
}
