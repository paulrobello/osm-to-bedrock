//! Conversion parameters, decoupled from CLI argument structs.
//!
//! These types are passed through the pipeline functions and also used
//! by the HTTP server to drive conversions without going through `clap`.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Parameters for the OSM-to-Bedrock conversion pipeline.
///
/// Created by the CLI (`ConvertArgs` / `FetchConvertArgs`) and by server
/// handlers.  Decoupled from `clap` so the pipeline functions can be called
/// from any context.
pub struct ConvertParams {
    /// Input file path.  `None` when data is provided directly (e.g. from Overpass).
    pub input: Option<PathBuf>,
    pub output: PathBuf,
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

/// Overture Maps theme selector.
///
/// Each variant corresponds to one or more Overture CLI `-t` types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OvertureTheme {
    /// Buildings and structures.
    Building,
    /// Roads, paths, and transport links.
    Transportation,
    /// Named places (amenities, shops, tourism, etc.).
    Place,
    /// Land cover, land use, and water bodies.
    Base,
    /// Address points.
    Address,
}

impl OvertureTheme {
    /// Return all theme variants.
    pub fn all() -> Vec<Self> {
        vec![
            Self::Building,
            Self::Transportation,
            Self::Place,
            Self::Base,
            Self::Address,
        ]
    }

    /// Return the Overture CLI `-t` type string(s) for this theme.
    ///
    /// Most themes map to a single type, but `Base` maps to several.
    pub fn cli_types(&self) -> Vec<&'static str> {
        match self {
            Self::Building => vec!["building"],
            Self::Transportation => vec!["segment"],
            Self::Place => vec!["place"],
            Self::Base => vec!["land", "land_use", "water"],
            Self::Address => vec!["address"],
        }
    }

    /// Parse a theme name flexibly, accepting plural forms (e.g. "buildings").
    pub fn from_str_loose(s: &str) -> Option<Self> {
        match s.to_lowercase().trim_end_matches('s') {
            "building" => Some(Self::Building),
            "transportation" | "transport" | "road" | "segment" => Some(Self::Transportation),
            "place" => Some(Self::Place),
            "base" | "land" | "land_use" | "landuse" | "water" => Some(Self::Base),
            "address" | "addr" => Some(Self::Address),
            _ => None,
        }
    }
}

impl std::fmt::Display for OvertureTheme {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Building => write!(f, "building"),
            Self::Transportation => write!(f, "transportation"),
            Self::Place => write!(f, "place"),
            Self::Base => write!(f, "base"),
            Self::Address => write!(f, "address"),
        }
    }
}

/// Which data source wins when Overture and OSM both cover the same theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemePriority {
    /// Use only Overture data for this theme.
    Overture,
    /// Use only OSM data for this theme.
    Osm,
    /// Merge both data sources (default).
    #[default]
    Both,
}

/// Parameters controlling Overture Maps data integration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OvertureParams {
    /// Whether to fetch and merge Overture Maps data at all.
    pub enabled: bool,
    /// Which Overture themes to fetch.  Defaults to all themes.
    pub themes: Vec<OvertureTheme>,
    /// Per-theme priority override.  Missing keys default to [`ThemePriority::Both`].
    pub priority: HashMap<OvertureTheme, ThemePriority>,
    /// Timeout in seconds for the `overturemaps` CLI subprocess.
    pub timeout_secs: u64,
}

impl Default for OvertureParams {
    fn default() -> Self {
        Self {
            enabled: false,
            themes: OvertureTheme::all(),
            priority: HashMap::new(),
            timeout_secs: 120,
        }
    }
}

impl OvertureParams {
    /// Return the priority for a given theme, defaulting to [`ThemePriority::Both`].
    pub fn priority_for(&self, theme: OvertureTheme) -> ThemePriority {
        self.priority
            .get(&theme)
            .copied()
            .unwrap_or(ThemePriority::Both)
    }
}

/// Parameters for the terrain-only pipeline (SRTM elevation → Bedrock world).
pub struct TerrainParams {
    /// Bounding box: (min_lat, min_lon, max_lat, max_lon).
    pub bbox: (f64, f64, f64, f64),
    /// Output world directory.
    pub output: PathBuf,
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
