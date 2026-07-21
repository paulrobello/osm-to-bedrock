//! Conversion parameters, decoupled from CLI argument structs.
//!
//! These types are passed through the pipeline functions and also used
//! by the HTTP server to drive conversions without going through `clap`.

use std::path::PathBuf;

pub use par_osm_rust::overture::{OvertureParams, OvertureTheme};
pub use par_osm_rust::sources::{OvertureFailureMode, PoiSourceMode, SourceOptions, SourceStatus};

/// Parameters for the OSM-to-Bedrock conversion pipeline.
///
/// Created by the CLI (`ConvertArgs` / `FetchConvertArgs`) and by server
/// handlers.  Decoupled from `clap` so the pipeline functions can be called
/// from any context.
pub struct ConvertParams {
    /// Input file path.  `None` when data is provided directly (e.g. from Overpass).
    pub input: Option<PathBuf>,
    pub output: PathBuf,
    pub edition: crate::world::Edition,
    pub scale: f64,
    pub sea_level: i32,
    pub building_height: i32,
    /// Snap building walls within this many blocks of axis-aligned to straight.
    /// 0 = disabled.  Default: 1.
    pub wall_straighten_threshold: i32,
    /// Spawn block X — takes priority over spawn_lat/lon.
    pub spawn_x: Option<i32>,
    pub spawn_y: Option<i32>,
    /// Spawn block Z — takes priority over spawn_lat/lon.
    pub spawn_z: Option<i32>,
    /// Spawn latitude — converted to block coords via CoordConverter when spawn_x is None.
    pub spawn_lat: Option<f64>,
    /// Spawn longitude — converted to block coords via CoordConverter when spawn_z is None.
    pub spawn_lon: Option<f64>,
    pub signs: bool,
    /// Place address signs on building facades using addr:housenumber/addr:street tags.
    pub address_signs: bool,
    /// Place POI markers at amenity/shop/tourism nodes and ways.
    pub poi_markers: bool,
    /// Place decorative blocks at POI locations (benches, mailboxes, etc.)
    pub poi_decorations: bool,
    /// Place individual trees from tree node data (OSM natural=tree, Overture land/tree)
    pub nature_decorations: bool,
    pub filter: crate::filter::FeatureFilter,
    /// Optional path to SRTM HGT file(s) for real-world terrain elevation.
    pub elevation: Option<PathBuf>,
    /// Blocks per metre of elevation (default 1.0).
    pub vertical_scale: f64,
    /// Median-filter radius for elevation smoothing (0=off, 1=3x3 default, 2=5x5).
    pub elevation_smoothing: i32,
    /// Terrain fill depth below surface in blocks (default 4).
    pub surface_thickness: i32,
    /// User-supplied OSM tag → Block overrides (loaded from `--block-mapping`).
    /// `None` (no overrides) on the server and in `terrain-convert`.
    pub block_overrides: Option<crate::blocks::BlockOverrides>,
}

/// Parameters for the terrain-only pipeline (SRTM elevation → Bedrock world).
pub struct TerrainParams {
    /// Bounding box: (min_lat, min_lon, max_lat, max_lon).
    pub bbox: (f64, f64, f64, f64),
    /// Output world directory.
    pub output: PathBuf,
    pub edition: crate::world::Edition,
    /// Metres per block.
    pub scale: f64,
    /// Y coordinate for sea level.
    pub sea_level: i32,
    /// Blocks per metre of elevation change.
    pub vertical_scale: f64,
    /// Blocks above sea_level where stone+snow replaces grass (default 80).
    pub snow_line: i32,
    /// Median-filter radius for elevation smoothing (0=off, 1=3x3 default, 2=5x5).
    pub elevation_smoothing: i32,
    /// Terrain fill depth below surface in blocks (default 4).
    pub surface_thickness: i32,
    /// Explicit spawn block X (overrides spawn_lat/lon).
    pub spawn_x: Option<i32>,
    pub spawn_y: Option<i32>,
    /// Explicit spawn block Z (overrides spawn_lat/lon).
    pub spawn_z: Option<i32>,
    /// Spawn as geographic coordinates.
    pub spawn_lat: Option<f64>,
    pub spawn_lon: Option<f64>,
    /// Path to SRTM HGT file(s).  If None, terrain is flat at sea_level.
    pub elevation: Option<PathBuf>,
}

// ── Tests ─────────────────────────────────────────────────────────────────
//
// `params.rs` is intentionally a plain data module: two structs + re-exports
// from `par-osm-rust`. These tests pin the *public shape* of the structs so
// that adding a required field without a default is caught at the call sites
// in `main.rs`, `server.rs`, and the lib.rs doctest — all of which compile
// against the patterns asserted here. The re-export tests guard against
// silent breakage when `par-osm-rust` shifts its public `sources`/`overture`
// API.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::Edition;

    // ── ConvertParams shape ──────────────────────────────────────────────

    #[test]
    fn convert_params_constructs_with_all_documented_fields() {
        // This is the same shape required by the lib.rs doctest; if a field
        // is added without updating this construction, compilation fails here
        // AND in the doctest, surfacing the contract change.
        let params = ConvertParams {
            input: Some(PathBuf::from("map.osm.pbf")),
            output: PathBuf::from("out"),
            edition: Edition::Bedrock,
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
            poi_decorations: true,
            nature_decorations: true,
            filter: crate::filter::FeatureFilter::default(),
            elevation: None,
            vertical_scale: 1.0,
            elevation_smoothing: 1,
            surface_thickness: 4,
            block_overrides: None,
        };
        // Spot-check the values that downstream pipeline math depends on.
        assert_eq!(params.sea_level, 65);
        assert_eq!(params.building_height, 8);
        assert!(params.scale > 0.0);
        assert!(params.vertical_scale > 0.0);
        assert_eq!(params.edition, Edition::Bedrock);
        // The filter must default to all-categories-enabled.
        assert!(params.filter.roads && params.filter.buildings);
    }

    #[test]
    fn convert_params_supports_java_edition() {
        let params = ConvertParams {
            edition: Edition::Java,
            ..minimal_convert_params()
        };
        assert_eq!(params.edition, Edition::Java);
    }

    // ── TerrainParams shape ──────────────────────────────────────────────

    #[test]
    fn terrain_params_constructs_with_all_documented_fields() {
        let params = TerrainParams {
            bbox: (1.0, 2.0, 3.0, 4.0),
            output: PathBuf::from("terrain_out"),
            edition: Edition::Bedrock,
            scale: 1.0,
            sea_level: 65,
            vertical_scale: 1.0,
            snow_line: 80,
            elevation_smoothing: 1,
            surface_thickness: 4,
            spawn_x: None,
            spawn_y: None,
            spawn_z: None,
            spawn_lat: None,
            spawn_lon: None,
            elevation: None,
        };
        // bbox ordering is (min_lat, min_lon, max_lat, max_lon).
        assert_eq!(params.bbox, (1.0, 2.0, 3.0, 4.0));
        assert_eq!(params.snow_line, 80);
    }

    // ── par-osm-rust re-exports ──────────────────────────────────────────

    #[test]
    fn source_options_default_re_export_compiles() {
        // SourceOptions comes from par_osm_rust::sources. If the upstream
        // crate removes it or renames it, this fails to compile, surfacing
        // the breakage here rather than at a downstream call site.
        let _opts: SourceOptions = SourceOptions::default();
    }

    #[test]
    fn overture_params_re_export_compiles() {
        // OvertureParams is re-exported from par_osm_rust::overture. Touch
        // the type name so a rename upstream surfaces here.
        fn _accepts(_p: OvertureParams) {}
        let _ = std::any::TypeId::of::<OvertureParams>();
    }

    #[test]
    fn poi_source_mode_and_overture_failure_mode_re_exports_compile() {
        // These two enums are the most commonly used re-exports from the
        // `sources` module; pin them at the type level.
        let _ = std::any::TypeId::of::<PoiSourceMode>();
        let _ = std::any::TypeId::of::<OvertureFailureMode>();
        let _ = std::any::TypeId::of::<SourceStatus>();
        let _ = std::any::TypeId::of::<OvertureTheme>();
    }

    // ── Test helper ──────────────────────────────────────────────────────

    /// Minimal `ConvertParams` for tests that only care about one field.
    /// Mirrors the lib.rs doctest defaults.
    fn minimal_convert_params() -> ConvertParams {
        ConvertParams {
            input: None,
            output: PathBuf::from("out"),
            edition: Edition::default(),
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
            filter: crate::filter::FeatureFilter::default(),
            elevation: None,
            vertical_scale: 1.0,
            elevation_smoothing: 1,
            surface_thickness: 4,
            block_overrides: None,
        }
    }
}
