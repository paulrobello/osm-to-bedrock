//! Dispatch helpers for the convert-family subcommands
//! (`convert`, `fetch-convert`, `terrain-convert`, `overture-convert`).
//!
//! Each helper translates a `clap` arg struct into the library-level
//! `ConvertParams` / `TerrainParams` and invokes the matching `pipeline`
//! entry point. CLI-only concerns (Overpass fetch orchestration, watch mode,
//! SRTM auto-download, world-name joining) live here, keeping the pipeline
//! itself free of clap types.

use anyhow::Result;
use par_osm_rust::bbox::BBox;

use crate::cli::args::{ConvertArgs, FetchConvertArgs, OvertureConvertArgs, TerrainConvertArgs};
use crate::cli::parse_bbox;
use crate::config::Config;
use crate::filter;
use crate::params::{ConvertParams, TerrainParams};
use crate::pipeline::{run_conversion, run_conversion_from_data, run_terrain_only_to_disk};
use crate::source_options;
use crate::{overture, srtm};

/// `convert` — convert an OSM PBF file on disk into a Minecraft world.
pub fn run_convert(args: &ConvertArgs, config: &Config) -> Result<()> {
    let block_overrides =
        crate::block_mapping::load_block_overrides_arg(&args.building.block_mapping)?;
    let convert_params = ConvertParams {
        input: Some(args.input.clone()),
        output: args.output.clone(),
        edition: args.common.edition,
        scale: args.scale.or(config.scale).unwrap_or(1.0),
        sea_level: args.sea_level.or(config.sea_level).unwrap_or(65),
        building_height: args
            .building
            .building_height
            .or(config.building_height)
            .unwrap_or(8),
        wall_straighten_threshold: args
            .building
            .wall_straighten_threshold
            .or(config.wall_straighten_threshold)
            .unwrap_or(1),
        spawn_x: args.common.spawn_x,
        spawn_y: args.common.spawn_y,
        spawn_z: args.common.spawn_z,
        spawn_lat: args.spawn_lat,
        spawn_lon: args.spawn_lon,
        signs: args.building.signs || config.signs.unwrap_or(false),
        address_signs: args.building.address_signs || config.address_signs.unwrap_or(false),
        poi_markers: args.building.poi_markers || config.poi_markers.unwrap_or(false),
        poi_decorations: config.poi_decorations.unwrap_or(true),
        nature_decorations: config.nature_decorations.unwrap_or(true),
        filter: filter::FeatureFilter::default(),
        elevation: args.elevation.clone().or(config.elevation.clone()),
        vertical_scale: args.vertical_scale.or(config.vertical_scale).unwrap_or(1.0),
        elevation_smoothing: args
            .common
            .elevation_smoothing
            .or(config.elevation_smoothing)
            .unwrap_or(1),
        surface_thickness: args
            .common
            .surface_thickness
            .or(config.surface_thickness)
            .unwrap_or(4),
        block_overrides,
    };

    run_conversion(&convert_params, &log_progress)?;

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

        match run_conversion(&convert_params, &log_progress) {
            Ok(()) => log::info!("[watch] Conversion complete. Watching for changes\u{2026}"),
            Err(e) => {
                log::error!("[watch] Conversion failed: {e:#} \u{2014} will retry on next change")
            }
        }
    }
}

/// `fetch-convert` — fetch OSM data from Overpass (optionally merged with
/// Overture) and convert it into a Minecraft world in one step.
pub fn run_fetch_convert(args: &FetchConvertArgs, config: &Config) -> Result<()> {
    let bbox = parse_bbox(&args.bbox)?;
    let filter = filter::FeatureFilter {
        roads: !(args.no_roads || config.no_roads.unwrap_or(false)),
        buildings: !(args.no_buildings || config.no_buildings.unwrap_or(false)),
        water: !(args.no_water || config.no_water.unwrap_or(false)),
        landuse: !(args.no_landuse || config.no_landuse.unwrap_or(false)),
        railways: !(args.no_railways || config.no_railways.unwrap_or(false)),
    };

    let url = args
        .overpass_url
        .as_deref()
        .or(config.overpass_url.as_deref())
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned);
    let overture_enabled = args.overture || config.overture.unwrap_or(false);
    let overture_timeout = args
        .overture_timeout
        .or(config.overture_timeout)
        .unwrap_or(120);
    let (themes, poi_source_mode, overture_failure_mode) = if overture_enabled {
        let overture_themes = args
            .overture_themes
            .as_deref()
            .or(config.overture_themes.as_deref())
            .unwrap_or("building,transportation,place,base,address");
        let poi_source = args
            .poi_source
            .as_deref()
            .or(config.poi_source.as_deref())
            .unwrap_or("overture-preferred");
        let overture_failure = args
            .overture_failure
            .as_deref()
            .or(config.overture_failure.as_deref())
            .unwrap_or("fallback-to-osm");
        (
            source_options::parse_overture_themes(overture_themes)?,
            source_options::parse_poi_source_mode(poi_source)?,
            source_options::parse_overture_failure_mode(overture_failure)?,
        )
    } else {
        (
            crate::params::OvertureTheme::all(),
            crate::params::PoiSourceMode::OsmOnly,
            crate::params::OvertureFailureMode::FallbackToOsm,
        )
    };
    let source_options = crate::params::SourceOptions {
        filter: filter.clone(),
        overpass_url: url,
        use_overpass_cache: true,
        overture: crate::params::OvertureParams {
            enabled: overture_enabled,
            themes,
            timeout_secs: overture_timeout,
            cache_ttl_secs: None,
            ..Default::default()
        },
        poi_source_mode,
        overture_failure_mode,
        extra_allowed_hosts: Vec::new(),
    };
    let source_result = par_osm_rust::sources::fetch_map_data(
        &BBox::new(bbox.0, bbox.1, bbox.2, bbox.3)?,
        &source_options,
        &mut |progress, msg| println!("[{:3.0}%] {msg}", progress * 100.0),
    )?;
    for warning in &source_result.warnings {
        log::warn!("{warning}");
    }
    let data = source_result.data;

    let output = args.output.join(&args.world_name);
    std::fs::create_dir_all(&output)?;

    let block_overrides =
        crate::block_mapping::load_block_overrides_arg(&args.building.block_mapping)?;
    let convert_params = ConvertParams {
        input: None,
        output,
        edition: args.common.edition,
        scale: args.scale.or(config.scale).unwrap_or(1.0),
        sea_level: args.sea_level.or(config.sea_level).unwrap_or(65),
        building_height: args
            .building
            .building_height
            .or(config.building_height)
            .unwrap_or(8),
        wall_straighten_threshold: args
            .building
            .wall_straighten_threshold
            .or(config.wall_straighten_threshold)
            .unwrap_or(1),
        spawn_x: args.common.spawn_x,
        spawn_y: args.common.spawn_y,
        spawn_z: args.common.spawn_z,
        spawn_lat: args.spawn_lat,
        spawn_lon: args.spawn_lon,
        signs: args.building.signs || config.signs.unwrap_or(false),
        address_signs: args.building.address_signs || config.address_signs.unwrap_or(false),
        poi_markers: args.building.poi_markers || config.poi_markers.unwrap_or(false),
        poi_decorations: config.poi_decorations.unwrap_or(true),
        nature_decorations: config.nature_decorations.unwrap_or(true),
        filter,
        elevation: args.elevation.clone().or(config.elevation.clone()),
        vertical_scale: args.vertical_scale.or(config.vertical_scale).unwrap_or(1.0),
        elevation_smoothing: args
            .common
            .elevation_smoothing
            .or(config.elevation_smoothing)
            .unwrap_or(1),
        surface_thickness: args
            .common
            .surface_thickness
            .or(config.surface_thickness)
            .unwrap_or(4),
        block_overrides,
    };

    run_conversion_from_data(data, &convert_params, &print_progress)?;
    Ok(())
}

/// `overture-convert` — build a world from Overture Maps data only
/// (no OSM/Overpass required).
pub fn run_overture_convert(args: &OvertureConvertArgs, config: &Config) -> Result<()> {
    let bbox = parse_bbox(&args.bbox)?;
    let themes = source_options::parse_overture_themes(&args.themes)?;
    let overture_params = crate::params::OvertureParams {
        enabled: true,
        themes,
        timeout_secs: args.overture_timeout,
        cache_ttl_secs: None,
        ..Default::default()
    };

    let data = overture::fetch_overture_data(
        &BBox::new(bbox.0, bbox.1, bbox.2, bbox.3)?,
        &overture_params,
        &mut |progress, msg| {
            println!("[{:3.0}%] {msg}", progress * 100.0);
        },
    )?;

    let output = args.output.join(&args.world_name);
    std::fs::create_dir_all(&output)?;

    let block_overrides =
        crate::block_mapping::load_block_overrides_arg(&args.building.block_mapping)?;
    let convert_params = ConvertParams {
        input: None,
        output,
        edition: args.common.edition,
        scale: args.scale.or(config.scale).unwrap_or(1.0),
        sea_level: args.sea_level.or(config.sea_level).unwrap_or(65),
        building_height: args
            .building
            .building_height
            .or(config.building_height)
            .unwrap_or(8),
        wall_straighten_threshold: args
            .building
            .wall_straighten_threshold
            .or(config.wall_straighten_threshold)
            .unwrap_or(1),
        spawn_x: args.common.spawn_x,
        spawn_y: args.common.spawn_y,
        spawn_z: args.common.spawn_z,
        spawn_lat: args.spawn_lat,
        spawn_lon: args.spawn_lon,
        signs: args.building.signs || config.signs.unwrap_or(false),
        address_signs: args.building.address_signs || config.address_signs.unwrap_or(false),
        poi_markers: args.building.poi_markers || config.poi_markers.unwrap_or(false),
        poi_decorations: config.poi_decorations.unwrap_or(true),
        nature_decorations: config.nature_decorations.unwrap_or(true),
        filter: filter::FeatureFilter::default(),
        elevation: args.elevation.clone().or(config.elevation.clone()),
        vertical_scale: args.vertical_scale.or(config.vertical_scale).unwrap_or(1.0),
        elevation_smoothing: args
            .common
            .elevation_smoothing
            .or(config.elevation_smoothing)
            .unwrap_or(1),
        surface_thickness: args
            .common
            .surface_thickness
            .or(config.surface_thickness)
            .unwrap_or(4),
        block_overrides,
    };

    run_conversion_from_data(data, &convert_params, &print_progress)?;
    Ok(())
}

/// `terrain-convert` — generate a terrain-only world from SRTM elevation
/// data, with no OSM features.
pub fn run_terrain_convert(args: &TerrainConvertArgs, config: &Config) -> Result<()> {
    let bbox = parse_bbox(&args.bbox)?;

    // Auto-download SRTM tiles when no local path is provided.
    let elevation_path = if let Some(p) = args.elevation.clone() {
        Some(p)
    } else {
        let cache = srtm::cache_dir();
        log::info!("Downloading SRTM tiles to {}…", cache.display());
        srtm::download_tiles_for_bbox(
            &BBox::new(bbox.0, bbox.1, bbox.2, bbox.3)?,
            &cache,
            &mut |_fraction, message| {
                log::info!("  {message}");
            },
        )?;
        Some(cache)
    };

    let output = args.output.join(&args.world_name);
    std::fs::create_dir_all(&output)?;

    let terrain_params = TerrainParams {
        bbox,
        output,
        edition: args.common.edition,
        scale: args.scale.or(config.scale).unwrap_or(1.0),
        sea_level: args.sea_level.or(config.sea_level).unwrap_or(65),
        vertical_scale: args.vertical_scale.or(config.vertical_scale).unwrap_or(1.0),
        snow_line: args.snow_line.or(config.snow_line).unwrap_or(80),
        elevation_smoothing: args
            .common
            .elevation_smoothing
            .or(config.elevation_smoothing)
            .unwrap_or(1),
        surface_thickness: args
            .common
            .surface_thickness
            .or(config.surface_thickness)
            .unwrap_or(4),
        spawn_x: args.common.spawn_x,
        spawn_y: args.common.spawn_y,
        spawn_z: args.common.spawn_z,
        spawn_lat: args.spawn_lat,
        spawn_lon: args.spawn_lon,
        elevation: elevation_path,
    };

    run_terrain_only_to_disk(&terrain_params, &print_progress)
}

fn log_progress(_progress: f32, msg: &str) {
    log::info!("[progress] {}", msg);
}

fn print_progress(progress: f32, msg: &str) {
    println!("[{:3.0}%] {msg}", progress * 100.0);
}
