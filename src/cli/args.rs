//! clap CLI struct definitions for `osm-to-bedrock`.
//!
//! Contains the top-level [`Cli`], the [`Commands`] enum, all per-subcommand
//! argument structs, and the shared flag groups ([`ConvertCommonArgs`],
//! [`BuildingArgs`]) embedded via `#[command(flatten)]` in the
//! convert-family subcommands.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

use crate::world::Edition;

// ── Top-level ───────────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(
    name = "osm-to-bedrock",
    about = "Convert OpenStreetMap data to Minecraft Bedrock or Java Edition worlds",
    version
)]
pub struct Cli {
    /// Path to a YAML config file (overrides default search locations)
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,

    /// Print the resolved configuration as YAML and exit
    #[arg(long, global = true)]
    pub dump_config: bool,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Convert an OSM PBF file to a Minecraft world
    Convert(ConvertArgs),
    /// Run the HTTP API server
    Serve(ServeArgs),
    /// Fetch OSM data from Overpass and convert to a Minecraft world
    FetchConvert(FetchConvertArgs),
    /// Generate a terrain-only world from SRTM elevation data (no OSM required)
    TerrainConvert(TerrainConvertArgs),
    /// Build a world from Overture Maps data only (no OSM/Overpass required)
    OvertureConvert(OvertureConvertArgs),
    /// Manage the Overpass and Overture disk caches
    Cache(CacheArgs),
}

// ── Shared flag groups ─────────────────────────────────────────────────────

/// Flags shared by all four convert-family subcommands
/// (`convert`, `fetch-convert`, `terrain-convert`, `overture-convert`).
///
/// Only fields whose help text is already identical across every subcommand
/// live here, so flattening this group into each subcommand preserves the
/// per-flag help output exactly.
#[derive(Args, Debug)]
pub struct ConvertCommonArgs {
    /// Spawn X block coordinate (overrides --spawn-lat/lon)
    #[arg(long, allow_negative_numbers = true)]
    pub spawn_x: Option<i32>,

    /// Spawn Y block coordinate
    #[arg(long)]
    pub spawn_y: Option<i32>,

    /// Spawn Z block coordinate (overrides --spawn-lat/lon)
    #[arg(long, allow_negative_numbers = true)]
    pub spawn_z: Option<i32>,

    /// Median-filter radius for elevation smoothing (0=off, 1=3x3 default, 2=5x5)
    #[arg(long)]
    pub elevation_smoothing: Option<i32>,

    /// Terrain fill depth below surface (default 4, higher = more underground)
    #[arg(long)]
    pub surface_thickness: Option<i32>,

    /// Output edition: bedrock or java
    #[arg(long, value_enum, default_value = "bedrock")]
    pub edition: Edition,
}

/// Building- and decoration-related flags shared by the three subcommands
/// that render OSM features (`convert`, `fetch-convert`, `overture-convert`).
/// `terrain-convert` does not render buildings or signs and so does not embed
/// this group.
#[derive(Args, Debug)]
pub struct BuildingArgs {
    /// Building height in blocks
    #[arg(long)]
    pub building_height: Option<i32>,

    /// Snap building walls within this many blocks of axis-aligned to straight (0=off)
    #[arg(long)]
    pub wall_straighten_threshold: Option<i32>,

    /// Place street name signs along named roads
    #[arg(long, default_value = "false")]
    pub signs: bool,

    /// Place address signs on building facades
    #[arg(long, default_value = "false")]
    pub address_signs: bool,

    /// Place POI markers (signs) at amenities, shops, and tourism nodes
    #[arg(long, default_value = "false")]
    pub poi_markers: bool,

    /// Path to a YAML file overriding default OSM tag → block mappings
    /// (keys: building, highway, landuse, natural; values: Block variant names).
    #[arg(long, value_name = "PATH")]
    pub block_mapping: Option<PathBuf>,
}

// ── Per-subcommand args ────────────────────────────────────────────────────

/// Arguments for the `convert` subcommand.
#[derive(Parser, Debug)]
pub struct ConvertArgs {
    /// Input OSM PBF file path
    #[arg(short, long)]
    pub input: PathBuf,

    /// Output world directory
    #[arg(short, long)]
    pub output: PathBuf,

    /// Metres per block (default: 1.0 — 1:1 scale)
    #[arg(long)]
    pub scale: Option<f64>,

    /// Y coordinate for ground surface (default: 65)
    #[arg(long)]
    pub sea_level: Option<i32>,

    /// Origin latitude (defaults to centre of OSM bounding box)
    #[arg(long)]
    pub origin_lat: Option<f64>,

    /// Origin longitude (defaults to centre of OSM bounding box)
    #[arg(long)]
    pub origin_lon: Option<f64>,

    #[command(flatten)]
    pub building: BuildingArgs,

    /// Spawn latitude (defaults to centre of map)
    #[arg(long)]
    pub spawn_lat: Option<f64>,

    /// Spawn longitude (defaults to centre of map)
    #[arg(long)]
    pub spawn_lon: Option<f64>,

    #[command(flatten)]
    pub common: ConvertCommonArgs,

    /// Path to an SRTM HGT elevation file (e.g. N48W123.hgt) or a directory
    /// containing multiple .hgt files.  When supplied, terrain follows
    /// real-world elevation instead of being flat.
    #[arg(long)]
    pub elevation: Option<PathBuf>,

    /// Blocks per metre of elevation change (default: 1.0).
    /// Reduce for mountainous regions (e.g. 0.2) to keep peaks within the
    /// Bedrock world height limit of 319.
    #[arg(long)]
    pub vertical_scale: Option<f64>,

    /// Watch the input file for changes and re-convert automatically
    #[arg(long, default_value = "false")]
    pub watch: bool,
}

/// Arguments for the `serve` subcommand.
#[derive(Parser, Debug)]
pub struct ServeArgs {
    /// Port to listen on
    #[arg(long, default_value = "3002")]
    pub port: u16,

    /// Host address to bind to
    #[arg(long, default_value = "127.0.0.1")]
    pub host: String,

    /// Optional shared-secret API key for protected routes (mutating routes,
    /// /download, /status, /cache). Falls back to the
    /// `OSM_TO_BEDROCK_API_KEY` env var. When both are unset the server runs
    /// unauthenticated (loopback-dev mode); in that case binding a
    /// non-loopback host is refused unless `OSM_TO_BEDROCK_ALLOW_INSECURE_BIND=1`
    /// is set. Never hardcode a real key in a Dockerfile or shell history —
    /// pass it via env var or a secret mount.
    #[arg(long)]
    pub api_key: Option<String>,

    /// Clear cached Overpass data before starting.
    /// Optionally specify a minimum age (e.g. 7d, 24h, 30m) to only
    /// remove entries older than that. Without an age, all entries are removed.
    #[arg(long, value_name = "AGE", num_args = 0..=1)]
    pub clear_cache: Option<Option<String>>,
}

/// Arguments for the `fetch-convert` subcommand.
#[derive(Parser, Debug)]
pub struct FetchConvertArgs {
    /// Bounding box as "south,west,north,east" (decimal degrees)
    /// Example: "51.5,-0.13,51.52,-0.10"
    #[arg(long)]
    pub bbox: String,

    /// Output world directory
    #[arg(short, long)]
    pub output: PathBuf,

    /// Metres per block (default: 1.0)
    #[arg(long)]
    pub scale: Option<f64>,

    /// Y coordinate for ground surface (default: 65)
    #[arg(long)]
    pub sea_level: Option<i32>,

    #[command(flatten)]
    pub building: BuildingArgs,

    /// World name
    #[arg(long, default_value = "OSM World")]
    pub world_name: String,

    /// Overpass API URL (default: <https://overpass-api.de/api/interpreter>).
    /// Useful for pointing at a mirror when the default is overloaded.
    /// Can also be set via the OVERPASS_URL environment variable.
    #[arg(long)]
    pub overpass_url: Option<String>,

    /// Spawn latitude
    #[arg(long)]
    pub spawn_lat: Option<f64>,

    /// Spawn longitude
    #[arg(long)]
    pub spawn_lon: Option<f64>,

    #[command(flatten)]
    pub common: ConvertCommonArgs,

    /// Exclude roads from the output
    #[arg(long, default_value = "false")]
    pub no_roads: bool,

    /// Exclude buildings from the output
    #[arg(long, default_value = "false")]
    pub no_buildings: bool,

    /// Exclude water from the output
    #[arg(long, default_value = "false")]
    pub no_water: bool,

    /// Exclude landuse areas from the output
    #[arg(long, default_value = "false")]
    pub no_landuse: bool,

    /// Exclude railways from the output
    #[arg(long, default_value = "false")]
    pub no_railways: bool,

    /// Path to an SRTM HGT elevation file or directory of .hgt files.
    #[arg(long)]
    pub elevation: Option<PathBuf>,

    /// Blocks per metre of elevation change (default: 1.0).
    #[arg(long)]
    pub vertical_scale: Option<f64>,

    /// Also fetch and merge Overture Maps data with the OSM data
    #[arg(long, default_value = "false")]
    pub overture: bool,

    /// Comma-separated Overture themes to fetch (used when --overture is set)
    #[arg(long)]
    pub overture_themes: Option<String>,

    /// POI source mode: osm-only, overture-only, both, or overture-preferred
    #[arg(long)]
    pub poi_source: Option<String>,

    /// Overture failure behavior: fallback-to-osm or fail
    #[arg(long)]
    pub overture_failure: Option<String>,

    /// Timeout in seconds for the overturemaps CLI subprocess
    #[arg(long)]
    pub overture_timeout: Option<u64>,
}

/// Arguments for the `overture-convert` subcommand.
#[derive(Parser, Debug)]
pub struct OvertureConvertArgs {
    /// Bounding box as "south,west,north,east" (decimal degrees)
    #[arg(long)]
    pub bbox: String,

    /// Output world directory
    #[arg(short, long)]
    pub output: PathBuf,

    /// Comma-separated Overture themes to fetch
    #[arg(long, default_value = "building,transportation,place,base,address")]
    pub themes: String,

    /// Metres per block (default: 1.0)
    #[arg(long)]
    pub scale: Option<f64>,

    /// Y coordinate for ground surface (default: 65)
    #[arg(long)]
    pub sea_level: Option<i32>,

    #[command(flatten)]
    pub building: BuildingArgs,

    /// World name
    #[arg(long, default_value = "Overture World")]
    pub world_name: String,

    /// Spawn latitude (defaults to bbox centre)
    #[arg(long)]
    pub spawn_lat: Option<f64>,

    /// Spawn longitude (defaults to bbox centre)
    #[arg(long)]
    pub spawn_lon: Option<f64>,

    #[command(flatten)]
    pub common: ConvertCommonArgs,

    /// Path to an SRTM HGT elevation file or directory of .hgt files.
    #[arg(long)]
    pub elevation: Option<PathBuf>,

    /// Blocks per metre of elevation change (default: 1.0).
    #[arg(long)]
    pub vertical_scale: Option<f64>,

    /// Timeout in seconds for the overturemaps CLI subprocess
    #[arg(long, default_value = "120")]
    pub overture_timeout: u64,
}

/// Arguments for the `terrain-convert` subcommand.
#[derive(Parser, Debug)]
pub struct TerrainConvertArgs {
    /// Bounding box as "south,west,north,east" (decimal degrees)
    #[arg(long)]
    pub bbox: String,

    /// Output world directory
    #[arg(short, long)]
    pub output: PathBuf,

    /// World name (used as the subdirectory and level name)
    #[arg(long, default_value = "Terrain World")]
    pub world_name: String,

    /// Metres per block (default: 1.0)
    #[arg(long)]
    pub scale: Option<f64>,

    /// Y coordinate for sea level / ground baseline (default: 65)
    #[arg(long)]
    pub sea_level: Option<i32>,

    /// Blocks per metre of elevation change (default: 1.0)
    #[arg(long)]
    pub vertical_scale: Option<f64>,

    /// Blocks above sea level where snow starts (default: 80)
    #[arg(long)]
    pub snow_line: Option<i32>,

    /// Path to pre-downloaded SRTM HGT file or directory; auto-downloads if omitted
    #[arg(long)]
    pub elevation: Option<PathBuf>,

    /// Spawn latitude (defaults to bbox centre)
    #[arg(long)]
    pub spawn_lat: Option<f64>,

    /// Spawn longitude (defaults to bbox centre)
    #[arg(long)]
    pub spawn_lon: Option<f64>,

    #[command(flatten)]
    pub common: ConvertCommonArgs,
}

// ── cache subcommand ───────────────────────────────────────────────────────

/// Arguments for the `cache` subcommand.
#[derive(Parser, Debug)]
pub struct CacheArgs {
    #[command(subcommand)]
    pub action: CacheAction,
}

#[derive(Subcommand, Debug)]
pub enum CacheAction {
    /// List all cached entries (Overpass + Overture)
    List,
    /// Show cache statistics (entry counts, total size, directory paths)
    Stats,
    /// Clear cached entries, optionally only those older than a given age
    Clear(CacheClearArgs),
}

#[derive(Parser, Debug)]
pub struct CacheClearArgs {
    /// Clear only entries older than this age (e.g. 7d, 24h, 30m).
    /// Without this flag, all entries are removed.
    #[arg(long, value_name = "AGE")]
    pub older_than: Option<String>,

    /// Clear only Overpass cache entries
    #[arg(long)]
    pub overpass_only: bool,

    /// Clear only Overture cache entries
    #[arg(long)]
    pub overture_only: bool,
}
