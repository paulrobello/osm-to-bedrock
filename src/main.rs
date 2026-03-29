//! OSM to Minecraft Bedrock Edition world converter.
//!
//! ## Usage
//! ```text
//! osm-to-bedrock convert --input map.osm.pbf --output MyWorld/
//! osm-to-bedrock convert --input map.osm.pbf --output MyWorld/ --scale 2.0 --sea-level 62
//! osm-to-bedrock serve --port 3002 --host 127.0.0.1
//! ```

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use std::collections::HashMap;
use std::path::PathBuf;

use osm_to_bedrock::{
    config::Config, filter, osm_cache, overpass, overture, params, pipeline, server, srtm,
};

use params::{ConvertParams, TerrainParams};
use pipeline::{run_conversion, run_conversion_from_data, run_terrain_only_to_disk};

// ── CLI ────────────────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(
    name = "osm-to-bedrock",
    about = "Convert OpenStreetMap data to Minecraft Bedrock Edition worlds",
    version
)]
struct Cli {
    /// Path to a YAML config file (overrides default search locations)
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    /// Print the resolved configuration as YAML and exit
    #[arg(long, global = true)]
    dump_config: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Convert an OSM PBF file to a Minecraft Bedrock world
    Convert(ConvertArgs),
    /// Run the HTTP API server
    Serve(ServeArgs),
    /// Fetch OSM data from Overpass and convert to a Minecraft Bedrock world
    FetchConvert(FetchConvertArgs),
    /// Generate a terrain-only world from SRTM elevation data (no OSM required)
    TerrainConvert(TerrainConvertArgs),
    /// Build a world from Overture Maps data only (no OSM/Overpass required)
    OvertureConvert(OvertureConvertArgs),
    /// Manage the Overpass and Overture disk caches
    Cache(CacheArgs),
}

/// Arguments for the `convert` subcommand.
#[derive(Parser, Debug)]
struct ConvertArgs {
    /// Input OSM PBF file path
    #[arg(short, long)]
    input: PathBuf,

    /// Output Bedrock world directory
    #[arg(short, long)]
    output: PathBuf,

    /// Metres per block (default: 1.0 — 1:1 scale)
    #[arg(long)]
    scale: Option<f64>,

    /// Y coordinate for ground surface (default: 65)
    #[arg(long)]
    sea_level: Option<i32>,

    /// Origin latitude (defaults to centre of OSM bounding box)
    #[arg(long)]
    origin_lat: Option<f64>,

    /// Origin longitude (defaults to centre of OSM bounding box)
    #[arg(long)]
    origin_lon: Option<f64>,

    /// Building height in blocks
    #[arg(long)]
    building_height: Option<i32>,

    /// Snap building walls within this many blocks of axis-aligned to straight (0=off)
    #[arg(long)]
    wall_straighten_threshold: Option<i32>,

    /// Spawn latitude (defaults to centre of map)
    #[arg(long)]
    spawn_lat: Option<f64>,

    /// Spawn longitude (defaults to centre of map)
    #[arg(long)]
    spawn_lon: Option<f64>,

    /// Spawn X block coordinate (overrides --spawn-lat/lon)
    #[arg(long, allow_negative_numbers = true)]
    spawn_x: Option<i32>,

    /// Spawn Y block coordinate
    #[arg(long)]
    spawn_y: Option<i32>,

    /// Spawn Z block coordinate (overrides --spawn-lat/lon)
    #[arg(long, allow_negative_numbers = true)]
    spawn_z: Option<i32>,

    /// Place street name signs along named roads
    #[arg(long, default_value = "false")]
    signs: bool,

    /// Place address signs on building facades
    #[arg(long, default_value = "false")]
    address_signs: bool,

    /// Place POI markers (signs) at amenities, shops, and tourism nodes
    #[arg(long, default_value = "false")]
    poi_markers: bool,

    /// Path to an SRTM HGT elevation file (e.g. N48W123.hgt) or a directory
    /// containing multiple .hgt files.  When supplied, terrain follows
    /// real-world elevation instead of being flat.
    #[arg(long)]
    elevation: Option<PathBuf>,

    /// Blocks per metre of elevation change (default: 1.0).
    /// Reduce for mountainous regions (e.g. 0.2) to keep peaks within the
    /// Bedrock world height limit of 319.
    #[arg(long)]
    vertical_scale: Option<f64>,

    /// Median-filter radius for elevation smoothing (0=off, 1=3x3 default, 2=5x5)
    #[arg(long)]
    elevation_smoothing: Option<i32>,

    /// Terrain fill depth below surface (default 4, higher = more underground)
    #[arg(long)]
    surface_thickness: Option<i32>,

    /// Watch the input file for changes and re-convert automatically
    #[arg(long, default_value = "false")]
    watch: bool,
}

/// Arguments for the `serve` subcommand.
#[derive(Parser, Debug)]
struct ServeArgs {
    /// Port to listen on
    #[arg(long, default_value = "3002")]
    port: u16,

    /// Host address to bind to
    #[arg(long, default_value = "127.0.0.1")]
    host: String,

    /// Clear cached Overpass data before starting.
    /// Optionally specify a minimum age (e.g. 7d, 24h, 30m) to only
    /// remove entries older than that. Without an age, all entries are removed.
    #[arg(long, value_name = "AGE", num_args = 0..=1)]
    clear_cache: Option<Option<String>>,
}

/// Parse a cache-age string like "7d", "24h", "30m" into a `chrono::Duration`.
fn parse_cache_age(s: &str) -> anyhow::Result<chrono::Duration> {
    if let Some(days) = s.strip_suffix('d') {
        let n: i64 = days
            .parse()
            .map_err(|_| anyhow::anyhow!("invalid days: {s}"))?;
        return Ok(chrono::Duration::days(n));
    }
    if let Some(hours) = s.strip_suffix('h') {
        let n: i64 = hours
            .parse()
            .map_err(|_| anyhow::anyhow!("invalid hours: {s}"))?;
        return Ok(chrono::Duration::hours(n));
    }
    if let Some(mins) = s.strip_suffix('m') {
        let n: i64 = mins
            .parse()
            .map_err(|_| anyhow::anyhow!("invalid minutes: {s}"))?;
        return Ok(chrono::Duration::minutes(n));
    }
    anyhow::bail!("invalid age format '{s}' — expected Nd, Nh, or Nm (e.g. 7d, 24h, 30m)")
}

/// Arguments for the `fetch-convert` subcommand.
#[derive(Parser, Debug)]
struct FetchConvertArgs {
    /// Bounding box as "south,west,north,east" (decimal degrees)
    /// Example: "51.5,-0.13,51.52,-0.10"
    #[arg(long)]
    bbox: String,

    /// Output Bedrock world directory
    #[arg(short, long)]
    output: PathBuf,

    /// Metres per block (default: 1.0)
    #[arg(long)]
    scale: Option<f64>,

    /// Y coordinate for ground surface (default: 65)
    #[arg(long)]
    sea_level: Option<i32>,

    /// Building height in blocks
    #[arg(long)]
    building_height: Option<i32>,

    /// Snap building walls within this many blocks of axis-aligned to straight (0=off)
    #[arg(long)]
    wall_straighten_threshold: Option<i32>,

    /// World name
    #[arg(long, default_value = "OSM World")]
    world_name: String,

    /// Overpass API URL (default: https://overpass-api.de/api/interpreter).
    /// Useful for pointing at a mirror when the default is overloaded.
    /// Can also be set via the OVERPASS_URL environment variable.
    #[arg(long)]
    overpass_url: Option<String>,

    /// Spawn latitude
    #[arg(long)]
    spawn_lat: Option<f64>,

    /// Spawn longitude
    #[arg(long)]
    spawn_lon: Option<f64>,

    /// Spawn X block coordinate (overrides --spawn-lat/lon)
    #[arg(long, allow_negative_numbers = true)]
    spawn_x: Option<i32>,

    /// Spawn Y block coordinate
    #[arg(long)]
    spawn_y: Option<i32>,

    /// Spawn Z block coordinate (overrides --spawn-lat/lon)
    #[arg(long, allow_negative_numbers = true)]
    spawn_z: Option<i32>,

    /// Exclude roads from the output
    #[arg(long, default_value = "false")]
    no_roads: bool,

    /// Exclude buildings from the output
    #[arg(long, default_value = "false")]
    no_buildings: bool,

    /// Exclude water from the output
    #[arg(long, default_value = "false")]
    no_water: bool,

    /// Exclude landuse areas from the output
    #[arg(long, default_value = "false")]
    no_landuse: bool,

    /// Exclude railways from the output
    #[arg(long, default_value = "false")]
    no_railways: bool,

    /// Place street name signs along named roads
    #[arg(long, default_value = "false")]
    signs: bool,

    /// Place address signs on building facades
    #[arg(long, default_value = "false")]
    address_signs: bool,

    /// Place POI markers (signs) at amenities, shops, and tourism nodes
    #[arg(long, default_value = "false")]
    poi_markers: bool,

    /// Path to an SRTM HGT elevation file or directory of .hgt files.
    #[arg(long)]
    elevation: Option<PathBuf>,

    /// Blocks per metre of elevation change (default: 1.0).
    #[arg(long)]
    vertical_scale: Option<f64>,

    /// Median-filter radius for elevation smoothing (0=off, 1=3x3 default, 2=5x5)
    #[arg(long)]
    elevation_smoothing: Option<i32>,

    /// Terrain fill depth below surface (default 4, higher = more underground)
    #[arg(long)]
    surface_thickness: Option<i32>,

    /// Also fetch and merge Overture Maps data with the OSM data
    #[arg(long, default_value = "false")]
    overture: bool,

    /// Comma-separated Overture themes to fetch (used when --overture is set)
    #[arg(long, default_value = "building,transportation,place,base,address")]
    overture_themes: String,

    /// Per-theme priority overrides, e.g. "building=overture,transportation=osm"
    #[arg(long, default_value = "")]
    overture_priority: String,

    /// Timeout in seconds for the overturemaps CLI subprocess
    #[arg(long, default_value = "120")]
    overture_timeout: u64,
}

/// Arguments for the `overture-convert` subcommand.
#[derive(Parser, Debug)]
struct OvertureConvertArgs {
    /// Bounding box as "south,west,north,east" (decimal degrees)
    #[arg(long)]
    bbox: String,

    /// Output Bedrock world directory
    #[arg(short, long)]
    output: PathBuf,

    /// Comma-separated Overture themes to fetch
    #[arg(long, default_value = "building,transportation,place,base,address")]
    themes: String,

    /// Metres per block (default: 1.0)
    #[arg(long)]
    scale: Option<f64>,

    /// Y coordinate for ground surface (default: 65)
    #[arg(long)]
    sea_level: Option<i32>,

    /// Building height in blocks
    #[arg(long)]
    building_height: Option<i32>,

    /// Snap building walls within this many blocks of axis-aligned to straight (0=off)
    #[arg(long)]
    wall_straighten_threshold: Option<i32>,

    /// World name
    #[arg(long, default_value = "Overture World")]
    world_name: String,

    /// Spawn latitude (defaults to bbox centre)
    #[arg(long)]
    spawn_lat: Option<f64>,

    /// Spawn longitude (defaults to bbox centre)
    #[arg(long)]
    spawn_lon: Option<f64>,

    /// Spawn X block coordinate (overrides --spawn-lat/lon)
    #[arg(long, allow_negative_numbers = true)]
    spawn_x: Option<i32>,

    /// Spawn Y block coordinate
    #[arg(long)]
    spawn_y: Option<i32>,

    /// Spawn Z block coordinate (overrides --spawn-lat/lon)
    #[arg(long, allow_negative_numbers = true)]
    spawn_z: Option<i32>,

    /// Place street name signs along named roads
    #[arg(long, default_value = "false")]
    signs: bool,

    /// Place address signs on building facades
    #[arg(long, default_value = "false")]
    address_signs: bool,

    /// Place POI markers (signs) at amenities, shops, and tourism nodes
    #[arg(long, default_value = "false")]
    poi_markers: bool,

    /// Path to an SRTM HGT elevation file or directory of .hgt files.
    #[arg(long)]
    elevation: Option<PathBuf>,

    /// Blocks per metre of elevation change (default: 1.0).
    #[arg(long)]
    vertical_scale: Option<f64>,

    /// Median-filter radius for elevation smoothing (0=off, 1=3x3 default, 2=5x5)
    #[arg(long)]
    elevation_smoothing: Option<i32>,

    /// Terrain fill depth below surface (default 4, higher = more underground)
    #[arg(long)]
    surface_thickness: Option<i32>,

    /// Timeout in seconds for the overturemaps CLI subprocess
    #[arg(long, default_value = "120")]
    overture_timeout: u64,
}

/// Arguments for the `terrain-convert` subcommand.
#[derive(Parser, Debug)]
struct TerrainConvertArgs {
    /// Bounding box as "south,west,north,east" (decimal degrees)
    #[arg(long)]
    bbox: String,

    /// Output Bedrock world directory
    #[arg(short, long)]
    output: PathBuf,

    /// World name (used as the subdirectory and level name)
    #[arg(long, default_value = "Terrain World")]
    world_name: String,

    /// Metres per block (default: 1.0)
    #[arg(long)]
    scale: Option<f64>,

    /// Y coordinate for sea level / ground baseline (default: 65)
    #[arg(long)]
    sea_level: Option<i32>,

    /// Blocks per metre of elevation change (default: 1.0)
    #[arg(long)]
    vertical_scale: Option<f64>,

    /// Blocks above sea level where snow starts (default: 80)
    #[arg(long)]
    snow_line: Option<i32>,

    /// Median-filter radius for elevation smoothing (0=off, 1=3x3 default, 2=5x5)
    #[arg(long)]
    elevation_smoothing: Option<i32>,

    /// Terrain fill depth below surface (default 4, higher = more underground)
    #[arg(long)]
    surface_thickness: Option<i32>,

    /// Path to pre-downloaded SRTM HGT file or directory; auto-downloads if omitted
    #[arg(long)]
    elevation: Option<PathBuf>,

    /// Spawn latitude (defaults to bbox centre)
    #[arg(long)]
    spawn_lat: Option<f64>,

    /// Spawn longitude (defaults to bbox centre)
    #[arg(long)]
    spawn_lon: Option<f64>,

    /// Spawn X block coordinate (overrides --spawn-lat/lon)
    #[arg(long, allow_negative_numbers = true)]
    spawn_x: Option<i32>,

    /// Spawn Y block coordinate
    #[arg(long)]
    spawn_y: Option<i32>,

    /// Spawn Z block coordinate (overrides --spawn-lat/lon)
    #[arg(long, allow_negative_numbers = true)]
    spawn_z: Option<i32>,
}

/// Arguments for the `cache` subcommand.
#[derive(Parser, Debug)]
struct CacheArgs {
    #[command(subcommand)]
    action: CacheAction,
}

#[derive(Subcommand, Debug)]
enum CacheAction {
    /// List all cached entries (Overpass + Overture)
    List,
    /// Show cache statistics (entry counts, total size, directory paths)
    Stats,
    /// Clear cached entries, optionally only those older than a given age
    Clear(CacheClearArgs),
}

#[derive(Parser, Debug)]
struct CacheClearArgs {
    /// Clear only entries older than this age (e.g. 7d, 24h, 30m).
    /// Without this flag, all entries are removed.
    #[arg(long, value_name = "AGE")]
    older_than: Option<String>,

    /// Clear only Overpass cache entries
    #[arg(long)]
    overpass_only: bool,

    /// Clear only Overture cache entries
    #[arg(long)]
    overture_only: bool,
}

// ── Main ───────────────────────────────────────────────────────────────────

/// Parse `"south,west,north,east"` into `(f64, f64, f64, f64)`.
fn parse_bbox(s: &str) -> Result<(f64, f64, f64, f64)> {
    let parts: Vec<&str> = s.split(',').collect();
    if parts.len() != 4 {
        bail!("bbox must be 4 comma-separated values: south,west,north,east — got '{s}'");
    }
    let vals: Vec<f64> = parts
        .iter()
        .map(|p| {
            p.trim()
                .parse::<f64>()
                .map_err(|e| anyhow::anyhow!("invalid bbox value '{p}': {e}"))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok((vals[0], vals[1], vals[2], vals[3]))
}

/// Parse a `ThemePriority` from a string ("overture", "osm", or "both").
fn parse_theme_priority(s: &str) -> Result<params::ThemePriority> {
    match s.to_lowercase().as_str() {
        "overture" => Ok(params::ThemePriority::Overture),
        "osm" => Ok(params::ThemePriority::Osm),
        "both" => Ok(params::ThemePriority::Both),
        _ => bail!("unknown priority '{s}' — expected overture, osm, or both"),
    }
}

/// Parse `"building=overture,transportation=osm"` into a priority map.
fn parse_overture_priority(
    s: &str,
) -> Result<HashMap<params::OvertureTheme, params::ThemePriority>> {
    let mut map = HashMap::new();
    if s.is_empty() {
        return Ok(map);
    }
    for entry in s.split(',') {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        let parts: Vec<&str> = entry.splitn(2, '=').collect();
        if parts.len() != 2 {
            bail!("invalid overture-priority entry '{entry}' — expected 'theme=priority'");
        }
        let theme = params::OvertureTheme::from_str_loose(parts[0].trim())
            .ok_or_else(|| anyhow::anyhow!("unknown Overture theme '{}'", parts[0].trim()))?;
        let priority = parse_theme_priority(parts[1].trim())?;
        map.insert(theme, priority);
    }
    Ok(map)
}

/// Parse `"building,transportation,place"` into a `Vec<OvertureTheme>`.
fn parse_overture_themes(s: &str) -> Result<Vec<params::OvertureTheme>> {
    if s.is_empty() {
        return Ok(params::OvertureTheme::all());
    }
    s.split(',')
        .map(|t| {
            let t = t.trim();
            params::OvertureTheme::from_str_loose(t)
                .ok_or_else(|| anyhow::anyhow!("unknown Overture theme '{t}'"))
        })
        .collect()
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let cli = Cli::parse();
    let config = Config::load(cli.config.as_deref())?;

    if cli.dump_config {
        return config.dump();
    }

    match cli.command {
        Commands::Convert(args) => run_convert(&args, &config),
        Commands::Serve(args) => {
            // ── Optional cache clear ────────────────────────────────────────────
            if let Some(age_opt) = &args.clear_cache {
                let min_age = match age_opt {
                    None => None,
                    Some(s) => Some(parse_cache_age(s)?),
                };
                let n = osm_cache::clear(min_age)?;
                let n2 = overture::clear_overture_cache(min_age)?;
                log::info!("Cleared {n} Overpass + {n2} Overture cache entries");
            }
            // ── Start server ────────────────────────────────────────────────────
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(server::run(&args.host, args.port))
        }
        Commands::TerrainConvert(args) => {
            let bbox = parse_bbox(&args.bbox)?;

            // Auto-download SRTM tiles when no local path is provided.
            let elevation_path = if let Some(p) = args.elevation {
                Some(p)
            } else {
                let cache = srtm::cache_dir();
                log::info!("Downloading SRTM tiles to {}…", cache.display());
                srtm::download_tiles_for_bbox(
                    bbox.0,
                    bbox.1,
                    bbox.2,
                    bbox.3,
                    &cache,
                    &|i, total, name| {
                        log::info!("  [{}/{}] {}", i + 1, total, name);
                    },
                )?;
                Some(cache)
            };

            let output = args.output.join(&args.world_name);
            std::fs::create_dir_all(&output)?;

            let terrain_params = TerrainParams {
                bbox,
                output,
                scale: args.scale.or(config.scale).unwrap_or(1.0),
                sea_level: args.sea_level.or(config.sea_level).unwrap_or(65),
                vertical_scale: args.vertical_scale.or(config.vertical_scale).unwrap_or(1.0),
                snow_line: args.snow_line.or(config.snow_line).unwrap_or(80),
                elevation_smoothing: args
                    .elevation_smoothing
                    .or(config.elevation_smoothing)
                    .unwrap_or(1),
                surface_thickness: args
                    .surface_thickness
                    .or(config.surface_thickness)
                    .unwrap_or(4),
                spawn_x: args.spawn_x,
                spawn_y: args.spawn_y,
                spawn_z: args.spawn_z,
                spawn_lat: args.spawn_lat,
                spawn_lon: args.spawn_lon,
                elevation: elevation_path,
            };

            run_terrain_only_to_disk(&terrain_params, &|progress, msg| {
                println!("[{:3.0}%] {msg}", progress * 100.0);
            })
        }
        Commands::FetchConvert(args) => {
            let bbox = parse_bbox(&args.bbox)?;
            let filter = filter::FeatureFilter {
                roads: !(args.no_roads || config.no_roads.unwrap_or(false)),
                buildings: !(args.no_buildings || config.no_buildings.unwrap_or(false)),
                water: !(args.no_water || config.no_water.unwrap_or(false)),
                landuse: !(args.no_landuse || config.no_landuse.unwrap_or(false)),
                railways: !(args.no_railways || config.no_railways.unwrap_or(false)),
            };

            let url = match args
                .overpass_url
                .as_deref()
                .or(config.overpass_url.as_deref())
                .filter(|s| !s.is_empty())
            {
                Some(u) => u.to_string(),
                None => overpass::default_overpass_url().to_string(),
            };
            let mut data = overpass::fetch_osm_data(bbox, &filter, true, &url)?;

            // Optionally merge Overture Maps data.
            let overture_enabled = args.overture || config.overture.unwrap_or(false);
            if overture_enabled {
                let themes = parse_overture_themes(&args.overture_themes)?;
                let priority = parse_overture_priority(&args.overture_priority)?;
                let overture_params = params::OvertureParams {
                    enabled: true,
                    themes,
                    priority,
                    timeout_secs: args.overture_timeout,
                };
                let overture_data =
                    overture::fetch_overture_data(bbox, &overture_params, &mut |progress, msg| {
                        println!("[{:3.0}%] {msg}", progress * 100.0);
                    })?;
                data.merge(overture_data);
            }

            // Clip to requested bbox so cached larger areas don't bloat the world.
            data.clip_to_bbox(bbox);

            let output = args.output.join(&args.world_name);
            std::fs::create_dir_all(&output)?;

            let convert_params = ConvertParams {
                input: None,
                output,
                scale: args.scale.or(config.scale).unwrap_or(1.0),
                sea_level: args.sea_level.or(config.sea_level).unwrap_or(65),
                building_height: args.building_height.or(config.building_height).unwrap_or(8),
                wall_straighten_threshold: args
                    .wall_straighten_threshold
                    .or(config.wall_straighten_threshold)
                    .unwrap_or(1),
                spawn_x: args.spawn_x,
                spawn_y: args.spawn_y,
                spawn_z: args.spawn_z,
                spawn_lat: args.spawn_lat,
                spawn_lon: args.spawn_lon,
                signs: args.signs || config.signs.unwrap_or(false),
                address_signs: args.address_signs || config.address_signs.unwrap_or(false),
                poi_markers: args.poi_markers || config.poi_markers.unwrap_or(false),
                poi_decorations: config.poi_decorations.unwrap_or(true),
                nature_decorations: config.nature_decorations.unwrap_or(true),
                filter,
                elevation: args.elevation.clone().or(config.elevation.clone()),
                vertical_scale: args.vertical_scale.or(config.vertical_scale).unwrap_or(1.0),
                elevation_smoothing: args
                    .elevation_smoothing
                    .or(config.elevation_smoothing)
                    .unwrap_or(1),
                surface_thickness: args
                    .surface_thickness
                    .or(config.surface_thickness)
                    .unwrap_or(4),
            };

            run_conversion_from_data(data, &convert_params, &|progress, msg| {
                println!("[{:3.0}%] {msg}", progress * 100.0);
            })?;
            Ok(())
        }
        Commands::OvertureConvert(args) => {
            let bbox = parse_bbox(&args.bbox)?;
            let themes = parse_overture_themes(&args.themes)?;
            let overture_params = params::OvertureParams {
                enabled: true,
                themes,
                priority: HashMap::new(),
                timeout_secs: args.overture_timeout,
            };

            let data =
                overture::fetch_overture_data(bbox, &overture_params, &mut |progress, msg| {
                    println!("[{:3.0}%] {msg}", progress * 100.0);
                })?;

            let output = args.output.join(&args.world_name);
            std::fs::create_dir_all(&output)?;

            let convert_params = ConvertParams {
                input: None,
                output,
                scale: args.scale.or(config.scale).unwrap_or(1.0),
                sea_level: args.sea_level.or(config.sea_level).unwrap_or(65),
                building_height: args.building_height.or(config.building_height).unwrap_or(8),
                wall_straighten_threshold: args
                    .wall_straighten_threshold
                    .or(config.wall_straighten_threshold)
                    .unwrap_or(1),
                spawn_x: args.spawn_x,
                spawn_y: args.spawn_y,
                spawn_z: args.spawn_z,
                spawn_lat: args.spawn_lat,
                spawn_lon: args.spawn_lon,
                signs: args.signs || config.signs.unwrap_or(false),
                address_signs: args.address_signs || config.address_signs.unwrap_or(false),
                poi_markers: args.poi_markers || config.poi_markers.unwrap_or(false),
                poi_decorations: config.poi_decorations.unwrap_or(true),
                nature_decorations: config.nature_decorations.unwrap_or(true),
                filter: filter::FeatureFilter::default(),
                elevation: args.elevation.clone().or(config.elevation.clone()),
                vertical_scale: args.vertical_scale.or(config.vertical_scale).unwrap_or(1.0),
                elevation_smoothing: args
                    .elevation_smoothing
                    .or(config.elevation_smoothing)
                    .unwrap_or(1),
                surface_thickness: args
                    .surface_thickness
                    .or(config.surface_thickness)
                    .unwrap_or(4),
            };

            run_conversion_from_data(data, &convert_params, &|progress, msg| {
                println!("[{:3.0}%] {msg}", progress * 100.0);
            })?;
            Ok(())
        }
        Commands::Cache(args) => run_cache(&args),
    }
}

fn run_cache(args: &CacheArgs) -> Result<()> {
    match &args.action {
        CacheAction::List => {
            let overpass_entries = osm_cache::list_areas();
            let overture_entries = overture::list_overture_areas();

            if overpass_entries.is_empty() && overture_entries.is_empty() {
                println!("No cached entries.");
                return Ok(());
            }

            if !overpass_entries.is_empty() {
                println!("Overpass cache ({} entries):", overpass_entries.len());
                println!(
                    "  {:<10} {:<45} {:<10} AGE",
                    "TYPE", "BBOX (S,W,N,E)", "SIZE"
                );
                for entry in &overpass_entries {
                    let [s, w, n, e] = entry.bbox;
                    let bbox_str = format!("{s:.4},{w:.4},{n:.4},{e:.4}");
                    let size = format_size(entry.size_bytes);
                    let age = format_age(entry.created_at);
                    println!("  {:<10} {:<45} {:<10} {}", "overpass", bbox_str, size, age);
                }
                println!();
            }

            if !overture_entries.is_empty() {
                println!("Overture cache ({} entries):", overture_entries.len());
                println!(
                    "  {:<10} {:<45} {:<10} AGE",
                    "TYPE", "BBOX (S,W,N,E)", "SIZE"
                );
                for entry in &overture_entries {
                    let [s, w, n, e] = entry.bbox;
                    let bbox_str = format!("{s:.4},{w:.4},{n:.4},{e:.4}");
                    let size = format_size(entry.size_bytes);
                    let age = format_age(entry.created_at);
                    println!(
                        "  {:<10} {:<45} {:<10} {}",
                        entry.cli_type, bbox_str, size, age
                    );
                }
            }
            Ok(())
        }
        CacheAction::Stats => {
            let overpass_dir = osm_cache::cache_dir();
            let overture_dir = overture::overture_cache_dir();
            let overpass_entries = osm_cache::list_areas();
            let overture_entries = overture::list_overture_areas();

            let overpass_total: u64 = overpass_entries.iter().map(|e| e.size_bytes).sum();
            let overture_total: u64 = overture_entries.iter().map(|e| e.size_bytes).sum();

            println!("Cache Statistics");
            println!("────────────────────────────────────────");
            println!(
                "Overpass:  {} entries, {} total",
                overpass_entries.len(),
                format_size(overpass_total)
            );
            println!("  dir: {}", overpass_dir.display());
            println!(
                "Overture:  {} entries, {} total",
                overture_entries.len(),
                format_size(overture_total)
            );
            println!("  dir: {}", overture_dir.display());
            println!("────────────────────────────────────────");
            println!(
                "Total:     {} entries, {}",
                overpass_entries.len() + overture_entries.len(),
                format_size(overpass_total + overture_total)
            );
            Ok(())
        }
        CacheAction::Clear(clear_args) => {
            let min_age = match &clear_args.older_than {
                Some(s) => Some(parse_cache_age(s)?),
                None => None,
            };

            let clear_overpass = !clear_args.overture_only;
            let clear_overture = !clear_args.overpass_only;

            let mut total_deleted = 0usize;

            if clear_overpass {
                let n = osm_cache::clear(min_age)?;
                total_deleted += n;
                if n > 0 {
                    println!("Cleared {n} Overpass cache entries");
                }
            }
            if clear_overture {
                let n = overture::clear_overture_cache(min_age)?;
                total_deleted += n;
                if n > 0 {
                    println!("Cleared {n} Overture cache entries");
                }
            }

            if total_deleted == 0 {
                println!("No entries to clear.");
            }
            Ok(())
        }
    }
}

fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

fn format_age(created_at: chrono::DateTime<chrono::Utc>) -> String {
    let now = chrono::Utc::now();
    let diff = now.signed_duration_since(created_at);
    let secs = diff.num_seconds();
    if secs < 60 {
        "just now".to_string()
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86400)
    }
}

fn run_convert(args: &ConvertArgs, config: &Config) -> Result<()> {
    let convert_params = ConvertParams {
        input: Some(args.input.clone()),
        output: args.output.clone(),
        scale: args.scale.or(config.scale).unwrap_or(1.0),
        sea_level: args.sea_level.or(config.sea_level).unwrap_or(65),
        building_height: args.building_height.or(config.building_height).unwrap_or(8),
        wall_straighten_threshold: args
            .wall_straighten_threshold
            .or(config.wall_straighten_threshold)
            .unwrap_or(1),
        spawn_x: args.spawn_x,
        spawn_y: args.spawn_y,
        spawn_z: args.spawn_z,
        spawn_lat: args.spawn_lat,
        spawn_lon: args.spawn_lon,
        signs: args.signs || config.signs.unwrap_or(false),
        address_signs: args.address_signs || config.address_signs.unwrap_or(false),
        poi_markers: args.poi_markers || config.poi_markers.unwrap_or(false),
        poi_decorations: config.poi_decorations.unwrap_or(true),
        nature_decorations: config.nature_decorations.unwrap_or(true),
        filter: filter::FeatureFilter::default(),
        elevation: args.elevation.clone().or(config.elevation.clone()),
        vertical_scale: args.vertical_scale.or(config.vertical_scale).unwrap_or(1.0),
        elevation_smoothing: args
            .elevation_smoothing
            .or(config.elevation_smoothing)
            .unwrap_or(1),
        surface_thickness: args
            .surface_thickness
            .or(config.surface_thickness)
            .unwrap_or(4),
    };

    // Initial conversion
    run_conversion(&convert_params, &|_progress, msg| {
        log::info!("[progress] {}", msg);
    })?;

    if !args.watch {
        return Ok(());
    }

    log::info!(
        "[watch] Watching {} for changes (Ctrl+C to stop)\u{2026}",
        args.input.display()
    );

    let (tx, rx) = std::sync::mpsc::channel();
    let mut watcher = notify::recommended_watcher(
        move |res: std::result::Result<notify::Event, notify::Error>| {
            if let Ok(event) = res
                && matches!(
                    event.kind,
                    notify::EventKind::Modify(_) | notify::EventKind::Create(_)
                )
            {
                let _ = tx.send(());
            }
        },
    )?;

    // Watch the parent directory (editors like JOSM delete + recreate files)
    let watch_dir = args
        .input
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    notify::Watcher::watch(&mut watcher, watch_dir, notify::RecursiveMode::NonRecursive)?;

    loop {
        // Wait for a change
        rx.recv()?;
        // Debounce: drain queued events, wait 500ms
        std::thread::sleep(std::time::Duration::from_millis(500));
        while rx.try_recv().is_ok() {}

        if !args.input.exists() {
            log::warn!(
                "[watch] {} was deleted \u{2014} waiting for it to reappear\u{2026}",
                args.input.display()
            );
            continue;
        }

        let filename = args
            .input
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_else(|| args.input.display().to_string());
        log::info!("[watch] Detected change in {filename} \u{2014} re-converting\u{2026}");

        match run_conversion(&convert_params, &|_progress, msg| {
            log::info!("[progress] {}", msg);
        }) {
            Ok(()) => log::info!("[watch] Conversion complete. Watching for changes\u{2026}"),
            Err(e) => {
                log::error!("[watch] Conversion failed: {e:#} \u{2014} will retry on next change")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_cache_age_days() {
        let d = parse_cache_age("7d").unwrap();
        assert_eq!(d, chrono::Duration::days(7));
    }

    #[test]
    fn parse_cache_age_hours() {
        let d = parse_cache_age("24h").unwrap();
        assert_eq!(d, chrono::Duration::hours(24));
    }

    #[test]
    fn parse_cache_age_minutes() {
        let d = parse_cache_age("30m").unwrap();
        assert_eq!(d, chrono::Duration::minutes(30));
    }

    #[test]
    fn parse_cache_age_invalid_suffix_errors() {
        assert!(parse_cache_age("10s").is_err());
        assert!(parse_cache_age("abc").is_err());
        assert!(parse_cache_age("").is_err());
    }

    #[test]
    fn parse_cache_age_non_numeric_prefix_errors() {
        assert!(parse_cache_age("xd").is_err());
        assert!(parse_cache_age("d").is_err());
    }

    #[test]
    fn parse_bbox_valid() {
        let (s, w, n, e) = parse_bbox("51.5,-0.13,51.52,-0.10").unwrap();
        assert!((s - 51.5).abs() < 0.001);
        assert!((w - -0.13).abs() < 0.001);
        assert!((n - 51.52).abs() < 0.001);
        assert!((e - -0.10).abs() < 0.001);
    }

    #[test]
    fn parse_bbox_wrong_count() {
        assert!(parse_bbox("51.5,-0.13,51.52").is_err());
    }

    #[test]
    fn parse_bbox_non_numeric() {
        assert!(parse_bbox("51.5,abc,51.52,-0.10").is_err());
    }

    #[test]
    fn roads_disabled_skips_road_rendering() {
        use filter::FeatureFilter;
        use osm_to_bedrock::osm::{OsmData, OsmNode, OsmWay};
        use osm_to_bedrock::params::ConvertParams;
        use osm_to_bedrock::pipeline::run_conversion_from_data;
        use std::collections::HashMap;
        use tempfile::TempDir;

        let mut nodes = HashMap::new();
        nodes.insert(
            1,
            OsmNode {
                lat: 51.5,
                lon: -0.1,
            },
        );
        nodes.insert(
            2,
            OsmNode {
                lat: 51.5,
                lon: -0.09,
            },
        );
        let mut tags = HashMap::new();
        tags.insert("highway".to_string(), "residential".to_string());
        let way = OsmWay {
            tags,
            node_refs: vec![1, 2],
        };
        // ways_by_id now maps way_id -> index into ways
        let ways_by_id = [(1i64, 0usize)].into_iter().collect();
        let data = OsmData {
            nodes,
            ways: vec![way],
            ways_by_id,
            relations: vec![],
            bounds: Some((51.5, -0.1, 51.5, -0.09)),
            poi_nodes: vec![],
            addr_nodes: vec![],
            tree_nodes: vec![],
        };

        let tmp = TempDir::new().unwrap();
        let convert_params = ConvertParams {
            input: None,
            output: tmp.path().to_path_buf(),
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
            filter: FeatureFilter {
                roads: false,
                ..FeatureFilter::default()
            },
            elevation: None,
            vertical_scale: 1.0,
            elevation_smoothing: 1,
            surface_thickness: 4,
        };
        let result = run_conversion_from_data(data, &convert_params, &|_, _| {});
        assert!(
            result.is_ok(),
            "conversion should succeed even with roads disabled"
        );
    }

    #[test]
    fn spatial_index_type_buckets() {
        use osm_to_bedrock::osm::OsmWay;
        use osm_to_bedrock::spatial::SpatialIndex;

        let make_way = |tags: Vec<(&str, &str)>| -> OsmWay {
            OsmWay {
                tags: tags
                    .into_iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
                node_refs: vec![],
            }
        };

        let w0 = make_way(vec![("highway", "residential"), ("name", "Main St")]);
        let w1 = make_way(vec![
            ("building", "yes"),
            ("addr:housenumber", "42"),
            ("addr:street", "Main St"),
        ]);
        let w2 = make_way(vec![("amenity", "restaurant"), ("name", "The Pub")]);
        let w3 = make_way(vec![("landuse", "park")]);
        let w4 = make_way(vec![("waterway", "river")]);
        let w5 = make_way(vec![("railway", "rail")]);
        let w6 = make_way(vec![("barrier", "fence")]);

        let resolved: Vec<(&OsmWay, Vec<(i32, i32)>)> = vec![
            (&w0, vec![(0, 0), (10, 0)]),
            (&w1, vec![(20, 20), (30, 20), (30, 30), (20, 30), (20, 20)]),
            (&w2, vec![(50, 50), (60, 50), (60, 60), (50, 60), (50, 50)]),
            (
                &w3,
                vec![(0, 100), (50, 100), (50, 150), (0, 150), (0, 100)],
            ),
            (&w4, vec![(0, 200), (100, 200)]),
            (&w5, vec![(0, 300), (100, 300)]),
            (&w6, vec![(0, 400), (100, 400)]),
        ];

        let idx = SpatialIndex::build(&resolved);

        assert_eq!(idx.highways, vec![0]);
        assert_eq!(idx.buildings, vec![1]);
        assert_eq!(idx.pois, vec![2]);
        assert_eq!(idx.landuse, vec![3]);
        assert_eq!(idx.waterways, vec![4]);
        assert_eq!(idx.railways, vec![5]);
        assert_eq!(idx.barriers, vec![6]);
        assert_eq!(idx.address, vec![1]);
    }

    #[test]
    fn spatial_index_query_rect_returns_overlapping() {
        use osm_to_bedrock::osm::OsmWay;
        use osm_to_bedrock::spatial::SpatialIndex;

        let make_way = |tags: Vec<(&str, &str)>| -> OsmWay {
            OsmWay {
                tags: tags
                    .into_iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
                node_refs: vec![],
            }
        };

        let w0 = make_way(vec![("highway", "primary")]);
        let w1 = make_way(vec![("highway", "secondary")]);

        let resolved: Vec<(&OsmWay, Vec<(i32, i32)>)> = vec![
            (&w0, vec![(0, 0), (10, 0)]),
            (&w1, vec![(500, 500), (600, 500)]),
        ];

        let idx = SpatialIndex::build(&resolved);

        let nearby = idx.query_rect(0, 0, 20, 20);
        assert!(nearby.contains(&0));
        assert!(!nearby.contains(&1));

        let far = idx.query_rect(490, 490, 610, 510);
        assert!(!far.contains(&0));
        assert!(far.contains(&1));
    }
}
