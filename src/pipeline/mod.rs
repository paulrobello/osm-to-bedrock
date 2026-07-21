//! OSM-to-Minecraft conversion pipeline.
//!
//! ## Pipeline variants
//!
//! | Function | Use case | Memory model |
//! |----------|----------|-------------|
//! | [`run_conversion`] | CLI `convert` subcommand | Streaming (tile-based) |
//! | [`run_conversion_from_data`] | Server / Overpass flow | Streaming (tile-based) |
//! | [`run_conversion_preview`] | Server preview endpoint | In-memory world |
//! | [`run_preview_from_data`] | Server preview (pre-fetched data) | In-memory world |
//! | [`run_surface_preview`] | Lightweight surface grid preview | Flat `Vec` (no chunks) |
//! | [`run_terrain_only`] | In-memory terrain (legacy) | In-memory world |
//! | [`run_terrain_only_to_disk`] | CLI `terrain-convert` / server | Streaming (tile-based) |
//!
//! ### Why the `*_from_data` and `*_preview` variants exist
//!
//! They are NOT redundant — each pair differs in both input source and
//! output shape:
//!
//! - **`run_conversion` vs `run_conversion_from_data`** — same streaming
//!   pipeline and disk output; the first reads `.osm.pbf` from
//!   `params.input` (with a `scale > 0` guard and `source_info` in
//!   metadata), the second takes pre-fetched `OsmData` (used by the
//!   Overpass server flow, where the file path is unknown so no
//!   `source_info` is recorded).
//! - **`run_conversion_preview` vs `run_preview_from_data`** — same
//!   in-memory pipeline and `Box<dyn WorldWriter>` output; same input
//!   distinction as above.
//! - **`run_surface_preview`** — distinct output shape
//!   (`Vec<(x,z,y,name)>` with no `ChunkData`), used for the fast
//!   block-type-map endpoint.
//!
//! ## Shared rendering
//!
//! Both the in-memory (`preview::run_pipeline`) and streaming
//! (`run_pipeline_streaming`) paths call
//! [`render::render_osm_features`] to avoid code duplication.  The
//! streaming path is edition-agnostic at the outer tile loop: each tile is
//! processed through a [`WorldWriter`] that exposes
//! [`WorldWriter::flush_tile`] to drain the current tile's state. Both
//! backends override `flush_tile` to bound peak memory to one tile — Bedrock
//! ships encoded SubChunks to a background LevelDB writer thread, and Java
//! (ARC-001) drains the tile's chunks into 32×32 region buffers and lazily
//! writes each `.mca` once its completing tile has flushed.
//!
//! ## Module layout
//!
//! | Submodule | Contents |
//! |-----------|----------|
//! | [`util`] | `zip_directory`, `format_bytes`, `is_closed_way`, `coord_hash` |
//! | `render` | `RenderContext`, `TileWays`, `render_osm_features` |
//! | `decoration` | POI / tree decoration helpers (`pub(super)`) |
//! | `terrain` | terrain fill, geometry helpers, `process_tile`, terrain-only entry points |
//! | `preview` | in-memory preview entry points |
//!
//! This file holds the top-level streaming dispatch
//! (`run_pipeline_streaming`, [`run_conversion`],
//! [`run_conversion_from_data`]) and re-exports the public API from the
//! submodules so callers see no difference from the pre-split flat module.

use anyhow::{Context, Result, bail};
use rayon::prelude::*;

use crate::bedrock;
use crate::osm;
use crate::params::ConvertParams;
use crate::spatial::{HeightMap, SpatialIndex, TILE_CHUNKS};
use crate::world::{Edition, WorldWriter};

mod decoration;
mod preview;
mod progress;
mod render;
mod terrain;
pub mod util;

pub use preview::{run_conversion_preview, run_preview_from_data, run_surface_preview};
pub use progress::{ProgressReport, format_duration, format_rate};
pub use render::{RenderContext, TileWays, render_osm_features};
pub use terrain::{process_tile, run_terrain_only, run_terrain_only_to_disk};
pub use util::{coord_hash, is_closed_way, zip_directory};

// ── Streaming dispatch + top-level entry points ───────────────────────────────

/// Run the full OSM-to-Bedrock conversion pipeline.
///
/// Uses the streaming (tile-based) pipeline so that only one
/// `TILE_CHUNKS × TILE_CHUNKS` tile of chunk data lives in memory at a time.
pub fn run_conversion(params: &ConvertParams, progress_cb: &dyn Fn(f32, &str)) -> Result<()> {
    if params.scale <= 0.0 {
        bail!("scale must be positive");
    }
    let timer = crate::metadata::MetadataTimer::start();

    progress_cb(0.0, "Parsing OSM data");
    let path = params.input.as_deref().ok_or_else(|| {
        anyhow::anyhow!("ConvertParams.input is required for file-based conversion")
    })?;
    log::info!("Reading {}", path.display());
    let source_info = crate::metadata::source_info(path).ok();
    let data = crate::osm::parse_osm_file(path)?;
    if data.ways().is_empty() {
        bail!("No ways found in OSM file.");
    }

    let metadata = crate::metadata::build_metadata(params, &data, &timer, source_info);
    let (spawn_x, spawn_y, spawn_z) = run_pipeline_streaming(data, params, progress_cb)?;

    // Write metadata after successful conversion (re-compute timing)
    let metadata = crate::metadata::WorldMetadata {
        timing: timer.finish(),
        ..metadata
    };
    if let Err(e) = crate::metadata::write_metadata(&params.output, &metadata) {
        log::warn!("Failed to write world_info.json: {e}");
    }

    progress_cb(1.0, "Conversion complete");
    log::info!(
        "Done! Open the '{}' folder in Minecraft Bedrock.",
        params.output.display()
    );
    let _ = (spawn_x, spawn_y, spawn_z);
    Ok(())
}

/// Run the full conversion pipeline from pre-fetched `OsmData` and save to disk.
///
/// Used by Overpass-based flows where OSM data has already been fetched and
/// does not need to be read from a file.
///
/// # Example
///
/// ```no_run
/// use osm_to_bedrock::filter::FeatureFilter;
/// use osm_to_bedrock::osm::OsmData;
/// use osm_to_bedrock::params::ConvertParams;
/// use osm_to_bedrock::pipeline::run_conversion_from_data;
/// use osm_to_bedrock::world::Edition;
/// use std::path::PathBuf;
///
/// // `data` would normally come from `osm::parse_osm_file` or an Overpass fetch.
/// let data: OsmData = todo!();
/// let params = ConvertParams {
///     input: None,
///     output: PathBuf::from("MyWorld"),
///     edition: Edition::default(),
///     scale: 1.0,
///     sea_level: 65,
///     building_height: 8,
///     wall_straighten_threshold: 1,
///     spawn_x: None,
///     spawn_y: None,
///     spawn_z: None,
///     spawn_lat: None,
///     spawn_lon: None,
///     signs: false,
///     address_signs: false,
///     poi_markers: false,
///     poi_decorations: true,
///     nature_decorations: true,
///     filter: FeatureFilter::default(),
///     elevation: None,
///     vertical_scale: 1.0,
///     elevation_smoothing: 1,
///     surface_thickness: 4,
///     block_overrides: None,
/// };
///
/// run_conversion_from_data(data, &params, &|progress, msg| {
///     println!("[{:3.0}%] {msg}", progress * 100.0);
/// }).expect("conversion failed");
/// ```
pub fn run_conversion_from_data(
    data: osm::OsmData,
    params: &ConvertParams,
    progress_cb: &dyn Fn(f32, &str),
) -> Result<()> {
    if data.ways().is_empty() {
        bail!("No ways found in OSM data.");
    }
    let timer = crate::metadata::MetadataTimer::start();
    let metadata = crate::metadata::build_metadata(params, &data, &timer, None);

    run_pipeline_streaming(data, params, progress_cb)?;

    // Write metadata with final timing
    let metadata = crate::metadata::WorldMetadata {
        timing: timer.finish(),
        ..metadata
    };
    if let Err(e) = crate::metadata::write_metadata(&params.output, &metadata) {
        log::warn!("Failed to write world_info.json: {e}");
    }

    progress_cb(1.0, "Conversion complete");
    Ok(())
}

/// Tile-based streaming conversion pipeline.
///
/// Processes the world in `TILE_CHUNKS × TILE_CHUNKS` chunk tiles so that
/// only one tile's chunk data lives in memory at a time.  Each tile is
/// encoded and sent to a background writer thread before
/// the next tile begins, pipelining CPU encoding with disk I/O.
///
/// `pub(crate)` so the intra-doc link from `src/world.rs` resolves.
pub(crate) fn run_pipeline_streaming(
    data: osm::OsmData,
    params: &ConvertParams,
    progress_cb: &dyn Fn(f32, &str),
) -> Result<(i32, i32, i32)> {
    // ── Determine origin ─────────────────────────────────────────────────────
    let (origin_lat, origin_lon) = {
        let (min_lat, min_lon, max_lat, max_lon) = data
            .bounds()
            .ok_or_else(|| anyhow::anyhow!("OSM file has no nodes"))?;
        ((min_lat + max_lat) / 2.0, (min_lon + max_lon) / 2.0)
    };

    let conv = crate::convert::CoordConverter::new(origin_lat, origin_lon, params.scale);
    let elevation_data =
        terrain::load_elevation(params.elevation.as_deref(), params.vertical_scale);
    let surface_thickness =
        terrain::effective_thickness(params.surface_thickness, elevation_data.is_some());

    // Pass 1: compute terrain bounding box
    progress_cb(0.10, "Computing terrain bounds");
    let (min_x, max_x, min_z, max_z) = terrain::compute_terrain_bounds(&data, &conv);

    let min_cx = min_x.div_euclid(16);
    let max_cx = max_x.div_euclid(16);
    let min_cz = min_z.div_euclid(16);
    let max_cz = max_z.div_euclid(16);

    log::info!(
        "Terrain bounds: x=[{}..{}] z=[{}..{}] ({} x {} blocks, chunks cx=[{}..{}] cz=[{}..{}])",
        min_x,
        max_x,
        min_z,
        max_z,
        max_x - min_x,
        max_z - min_z,
        min_cx,
        max_cx,
        min_cz,
        max_cz
    );

    // Both backends now stream tile-by-tile (Bedrock to LevelDB, Java to
    // lazily-written region files — ARC-001), so the world chunk rectangle no
    // longer needs an up-front memory guard. Peak RAM is bounded to one tile's
    // worth of ChunkData plus a small frontier of region buffers.

    // Pass 2: pre-compute global HeightMap (parallel, no ChunkData)
    progress_cb(0.20, "Computing height map");
    let surface = params.sea_level;
    let mut height_map: HeightMap = {
        let all_cols: Vec<(i32, i32)> = (min_cx..=max_cx)
            .flat_map(|cx| {
                (min_cz..=max_cz).flat_map(move |cz| {
                    (0..16i32)
                        .flat_map(move |lx| (0..16i32).map(move |lz| (cx * 16 + lx, cz * 16 + lz)))
                })
            })
            .collect();

        let heights: Vec<((i32, i32), i32)> = all_cols
            .par_iter()
            .map(|&(bx, bz)| {
                let sy = crate::spatial::compute_surface_y(
                    bx,
                    bz,
                    &elevation_data,
                    &conv,
                    surface,
                    params.vertical_scale,
                );
                ((bx, bz), sy)
            })
            .collect();

        let mut hm = HeightMap::with_bounds(
            min_cx * 16,
            min_cz * 16,
            max_cx * 16 + 15,
            max_cz * 16 + 15,
            surface,
        );
        for ((bx, bz), sy) in heights {
            hm.insert(bx, bz, sy);
        }
        hm
    };
    log::info!("Height map complete (bounded Vec)");

    if params.elevation_smoothing > 0 && elevation_data.is_some() {
        log::info!(
            "Smoothing height map (radius={})",
            params.elevation_smoothing
        );
        height_map.smooth(params.elevation_smoothing);
    }

    // Build resolved ways + spatial index
    progress_cb(0.30, "Building spatial index");
    let resolved_ways = terrain::resolve_ways(&data, &conv);
    let resolved_relations = terrain::resolve_relations(&data, &conv);
    let spatial_index = SpatialIndex::build(&resolved_ways);
    log::info!(
        "Spatial index: {} highway, {} building, {} landuse, {} water, {} railway, {} barrier ways",
        spatial_index.highways.len(),
        spatial_index.buildings.len(),
        spatial_index.landuse.len(),
        spatial_index.waterways.len(),
        spatial_index.railways.len(),
        spatial_index.barriers.len(),
    );

    // Compute spawn point
    let (spawn_x, spawn_y, spawn_z) =
        terrain::resolve_spawn(params, &conv, &height_map, min_cx, max_cx, min_cz, max_cz);
    log::info!("Spawn point: ({}, {}, {})", spawn_x, spawn_y, spawn_z);

    // Open writer and process tiles
    std::fs::create_dir_all(&params.output)
        .with_context(|| format!("creating output dir {}", params.output.display()))?;

    // Construct the persistent writer for the whole pipeline. Both editions
    // stream tile-by-tile through the same `WorldWriter` seam: Bedrock's
    // streaming backend owns a background LevelDB writer; Java's streaming
    // backend (ARC-001) lazily writes 32×32 region files as tiles flush.
    let mut world: Box<dyn WorldWriter> = match params.edition {
        Edition::Bedrock => {
            let db_path = params.output.join("db");
            std::fs::create_dir_all(&db_path)?;
            Box::new(bedrock::BedrockWorld::new_streaming(
                params.output.clone(),
                db_path,
            )?)
        }
        Edition::Java => Box::new(crate::anvil::JavaWorld::new_streaming(
            &params.output,
            min_cx,
            max_cx,
            min_cz,
            max_cz,
        )?),
    };

    progress_cb(0.35, "Converting in tiles");

    let tile_cx_count = (max_cx - min_cx + TILE_CHUNKS) / TILE_CHUNKS;
    let tile_cz_count = (max_cz - min_cz + TILE_CHUNKS) / TILE_CHUNKS;
    let total_tiles = tile_cx_count * tile_cz_count;
    log::info!(
        "Processing {total_tiles} tiles ({tile_cx_count}×{tile_cz_count}, each up to {}×{} chunks)",
        TILE_CHUNKS,
        TILE_CHUNKS
    );

    let mut tile_num = 0i32;
    let mut last_logged_pct = 0;
    let mut tile_cx0 = min_cx;
    while tile_cx0 <= max_cx {
        let tile_cx1 = (tile_cx0 + TILE_CHUNKS - 1).min(max_cx);
        let mut tile_cz0 = min_cz;
        while tile_cz0 <= max_cz {
            let tile_cz1 = (tile_cz0 + TILE_CHUNKS - 1).min(max_cz);
            tile_num += 1;

            let tile_progress = 0.35 + 0.50 * (tile_num as f32 / total_tiles as f32);
            progress_cb(tile_progress, &format!("Tile {tile_num}/{total_tiles}"));

            // Log at every 10% increment.
            let pct = tile_num * 100 / total_tiles.max(1);
            if pct / 10 > last_logged_pct / 10 {
                last_logged_pct = pct;
                log::info!("Tile progress: {pct}% ({tile_num}/{total_tiles})");
            }

            // Scope the writer to this tile. Bedrock enforces the bounds
            // on every `set_block`; Java's default-impl no-op leaves the
            // writer unbounded so it accumulates across tiles.
            world.set_tile_bounds(tile_cx0, tile_cx1, tile_cz0, tile_cz1);

            // Tile body (terrain fill + spatial filter + render).
            terrain::process_tile(
                &mut *world,
                tile_cx0,
                tile_cx1,
                tile_cz0,
                tile_cz1,
                &height_map,
                surface,
                surface_thickness,
                &spatial_index,
                &resolved_ways,
                &resolved_relations,
                &data,
                params,
                &conv,
            )
            .with_context(|| {
                format!("rendering tile ({tile_cx0}..{tile_cx1}, {tile_cz0}..{tile_cz1})")
            })?;

            // Drain the tile (Bedrock: ship to LevelDB; Java: no-op).
            world.flush_tile().with_context(|| {
                format!("flushing tile ({tile_cx0}..{tile_cx1}, {tile_cz0}..{tile_cz1})")
            })?;

            tile_cz0 += TILE_CHUNKS;
        }
        tile_cx0 += TILE_CHUNKS;
    }

    // Close-out (edition-specific only in the progress message; `save`
    // does the right thing for both backends).
    let finalize_msg = match params.edition {
        Edition::Bedrock => "Flushing LevelDB",
        Edition::Java => "Saving world",
    };
    progress_cb(0.88, finalize_msg);
    world.save(spawn_x, spawn_y, spawn_z)?;
    progress_cb(0.95, "Writing level.dat");

    progress_cb(0.99, "Streaming conversion complete");
    log::info!("Streamed tiles → {}", params.output.display());

    Ok((spawn_x, spawn_y, spawn_z))
}
