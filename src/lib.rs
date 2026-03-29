//! OSM to Minecraft Bedrock Edition world converter — library crate.
//!
//! This crate provides the core conversion logic for transforming
//! OpenStreetMap `.osm.pbf` files into playable Minecraft Bedrock Edition
//! worlds.  It can be used as a dependency by other Rust projects without
//! the CLI.
//!
//! # Quick start
//!
//! ```no_run
//! use osm_to_bedrock::params::ConvertParams;
//! use osm_to_bedrock::pipeline::run_conversion;
//! use osm_to_bedrock::filter::FeatureFilter;
//! use std::path::PathBuf;
//!
//! let params = ConvertParams {
//!     input: Some(PathBuf::from("map.osm.pbf")),
//!     output: PathBuf::from("MyWorld"),
//!     scale: 1.0,
//!     sea_level: 65,
//!     building_height: 8,
//!     wall_straighten_threshold: 1,
//!     spawn_x: None,
//!     spawn_y: None,
//!     spawn_z: None,
//!     spawn_lat: None,
//!     spawn_lon: None,
//!     signs: false,
//!     address_signs: false,
//!     poi_markers: false,
//!     poi_decorations: true,
//!     nature_decorations: true,
//!     filter: FeatureFilter::default(),
//!     elevation: None,
//!     vertical_scale: 1.0,
//!     elevation_smoothing: 1,
//!     surface_thickness: 4,
//! };
//!
//! run_conversion(&params, &|progress, msg| {
//!     println!("[{:3.0}%] {msg}", progress * 100.0);
//! }).expect("conversion failed");
//! ```

pub mod bedrock;
pub mod blocks;
pub mod config;
pub mod convert;
pub mod elevation;
pub mod filter;
pub mod geojson_export;
pub mod geometry;
pub mod metadata;
pub mod nbt;
pub mod osm;
pub mod osm_cache;
pub mod overpass;
pub mod overture;
pub mod params;
pub mod pipeline;
pub mod server;
pub mod sign;
pub mod spatial;
pub mod srtm;
