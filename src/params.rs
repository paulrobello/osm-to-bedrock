//! Conversion parameters, decoupled from CLI argument structs.
//!
//! These types are passed through the pipeline functions and also used
//! by the HTTP server to drive conversions without going through `clap`.

use std::path::PathBuf;

pub use par_osm_rust::overture::{OvertureParams, OvertureTheme, ThemePriority};
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
