//! Command-line interface for `osm-to-bedrock`.
//!
//! This module owns the binary's entry point and dispatch. The clap struct
//! definitions live in [`args`], the convert-family dispatch helpers in
//! [`convert`], and the `cache` subcommand in [`cache`]. The binary crate
//! (`src/main.rs`) is a one-line shim that delegates here:
//!
//! ```text
//! fn main() -> anyhow::Result<()> { osm_to_bedrock::cli::main() }
//! ```
//!
//! Keeping the CLI in the library (rather than in the binary) lets the
//! argument structs and dispatch functions be unit-tested alongside the rest
//! of the crate, and lets other embedders drive conversions without going
//! through a subprocess.

pub mod args;
pub mod cache;
pub mod convert;

use anyhow::{Result, bail};
use clap::Parser;

use self::args::{Cli, Commands, ServeArgs};
use crate::config::Config;
use crate::{osm_cache, overture, server};

/// Binary entry point — parses CLI args, loads config, and dispatches.
pub fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let cli = Cli::parse();
    let config = Config::load(cli.config.as_deref())?;

    if cli.dump_config {
        return config.dump();
    }

    match cli.command {
        Commands::Convert(args) => convert::run_convert(&args, &config),
        Commands::Serve(args) => run_serve(args),
        Commands::FetchConvert(args) => convert::run_fetch_convert(&args, &config),
        Commands::TerrainConvert(args) => convert::run_terrain_convert(&args, &config),
        Commands::OvertureConvert(args) => convert::run_overture_convert(&args, &config),
        Commands::Cache(args) => cache::run_cache(&args),
    }
}

/// `serve` dispatch — handles optional pre-start cache clear, then starts
/// the HTTP API server. The `--api-key` flag (Phase 1) is forwarded to
/// [`server::run`] as the `api_key_flag` parameter.
fn run_serve(args: ServeArgs) -> Result<()> {
    if let Some(age_opt) = &args.clear_cache {
        let min_age = match age_opt {
            None => None,
            Some(s) => Some(parse_cache_age(s)?),
        };
        let n = osm_cache::clear(min_age)?;
        let n2 = overture::clear_overture_cache(min_age)?;
        log::info!("Cleared {n} Overpass + {n2} Overture cache entries");
    }
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(server::run(&args.host, args.port, args.api_key))
}

// ── Shared helpers ─────────────────────────────────────────────────────────

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

/// Parse a cache-age string like "7d", "24h", "30m" into a `chrono::Duration`.
fn parse_cache_age(s: &str) -> Result<chrono::Duration> {
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
        use crate::filter::FeatureFilter;
        use crate::osm::{OsmData, OsmNode, OsmWay};
        use crate::params::ConvertParams;
        use crate::pipeline::run_conversion_from_data;
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
        tags.insert("highway".into(), "residential".to_string());
        let way = OsmWay {
            id: 1,
            tags,
            node_refs: vec![1, 2],
        };
        // ways_by_id is rebuilt from each way's `id` by `with_ways`.
        let data = OsmData::default()
            .with_nodes(nodes)
            .with_ways(vec![way])
            .with_bounds(Some((51.5, -0.1, 51.5, -0.09)));

        let tmp = TempDir::new().unwrap();
        let convert_params = ConvertParams {
            input: None,
            output: tmp.path().to_path_buf(),
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
            block_overrides: None,
        };
        let result = run_conversion_from_data(data, &convert_params, &|_, _| {});
        assert!(
            result.is_ok(),
            "conversion should succeed even with roads disabled"
        );
    }

    #[test]
    fn spatial_index_type_buckets() {
        use crate::osm::OsmWay;
        use crate::spatial::SpatialIndex;

        let make_way = |tags: Vec<(&str, &str)>| -> OsmWay {
            OsmWay {
                id: 0,
                tags: tags
                    .into_iter()
                    .map(|(k, v)| (k.into(), v.to_string()))
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
        use crate::osm::OsmWay;
        use crate::spatial::SpatialIndex;

        let make_way = |tags: Vec<(&str, &str)>| -> OsmWay {
            OsmWay {
                id: 0,
                tags: tags
                    .into_iter()
                    .map(|(k, v)| (k.into(), v.to_string()))
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
