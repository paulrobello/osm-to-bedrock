//! World metadata — writes a `world_info.json` alongside the output world.
//!
//! Records conversion parameters, source file info, bounding box, feature
//! counts, timing, and crate version for reproducibility.

use anyhow::Result;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;
use std::time::Instant;

use crate::filter::FeatureFilter;
use crate::osm::OsmData;
use crate::params::ConvertParams;

/// Metadata written to `world_info.json` after a successful conversion.
#[derive(Debug, Serialize)]
pub struct WorldMetadata {
    /// Crate version that performed the conversion.
    pub version: String,
    /// Conversion parameters.
    pub params: ParamsInfo,
    /// Source file information (absent for Overpass-based conversions).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceInfo>,
    /// Geographic bounding box of the input data.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bounds: Option<BoundsInfo>,
    /// Feature counts from the input OSM data.
    pub features: FeatureCounts,
    /// Timing information.
    pub timing: TimingInfo,
}

/// Conversion parameters recorded for reproducibility.
#[derive(Debug, Serialize)]
pub struct ParamsInfo {
    pub scale: f64,
    pub sea_level: i32,
    pub building_height: i32,
    pub wall_straighten_threshold: i32,
    pub signs: bool,
    pub address_signs: bool,
    pub poi_markers: bool,
    pub filter: FeatureFilter,
    pub elevation: bool,
    pub vertical_scale: f64,
    pub elevation_smoothing: i32,
    pub surface_thickness: i32,
}

/// Source file information.
#[derive(Debug, Serialize)]
pub struct SourceInfo {
    /// Original file path.
    pub path: String,
    /// File size in bytes.
    pub size_bytes: u64,
    /// SHA-256 hex digest of the file.
    pub sha256: String,
}

/// Geographic bounding box (min_lat, min_lon, max_lat, max_lon).
#[derive(Debug, Serialize)]
pub struct BoundsInfo {
    pub south: f64,
    pub west: f64,
    pub north: f64,
    pub east: f64,
}

/// Counts of OSM features in the input data.
#[derive(Debug, Serialize)]
pub struct FeatureCounts {
    pub nodes: usize,
    pub ways: usize,
    pub relations: usize,
    pub poi_nodes: usize,
    pub addr_nodes: usize,
}

/// Timing information for the conversion.
#[derive(Debug, Serialize)]
pub struct TimingInfo {
    /// ISO 8601 timestamp when conversion started.
    pub started_at: String,
    /// Total wall-clock duration in seconds.
    pub duration_secs: f64,
}

/// Tracks conversion timing. Create one at the start, call [`finish`] at the end.
pub struct MetadataTimer {
    start: Instant,
    started_at: String,
}

impl MetadataTimer {
    /// Start tracking conversion time.
    pub fn start() -> Self {
        Self {
            start: Instant::now(),
            started_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    /// Finish timing and return the `TimingInfo`.
    pub fn finish(&self) -> TimingInfo {
        TimingInfo {
            started_at: self.started_at.clone(),
            duration_secs: self.start.elapsed().as_secs_f64(),
        }
    }
}

/// Compute the SHA-256 hex digest of a file.
pub fn sha256_file(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Build `SourceInfo` for a file-based conversion.
pub fn source_info(path: &Path) -> Result<SourceInfo> {
    let meta = std::fs::metadata(path)?;
    let sha = sha256_file(path)?;
    Ok(SourceInfo {
        path: path.display().to_string(),
        size_bytes: meta.len(),
        sha256: sha,
    })
}

/// Build `WorldMetadata` from the conversion context.
pub fn build_metadata(
    params: &ConvertParams,
    data: &OsmData,
    timer: &MetadataTimer,
    source: Option<SourceInfo>,
) -> WorldMetadata {
    let bounds = data.bounds.map(|(s, w, n, e)| BoundsInfo {
        south: s,
        west: w,
        north: n,
        east: e,
    });

    let features = FeatureCounts {
        nodes: data.nodes.len(),
        ways: data.ways.len(),
        relations: data.relations.len(),
        poi_nodes: data.poi_nodes.len(),
        addr_nodes: data.addr_nodes.len(),
    };

    let params_info = ParamsInfo {
        scale: params.scale,
        sea_level: params.sea_level,
        building_height: params.building_height,
        wall_straighten_threshold: params.wall_straighten_threshold,
        signs: params.signs,
        address_signs: params.address_signs,
        poi_markers: params.poi_markers,
        filter: params.filter.clone(),
        elevation: params.elevation.is_some(),
        vertical_scale: params.vertical_scale,
        elevation_smoothing: params.elevation_smoothing,
        surface_thickness: params.surface_thickness,
    };

    WorldMetadata {
        version: env!("CARGO_PKG_VERSION").to_string(),
        params: params_info,
        source,
        bounds,
        features,
        timing: timer.finish(),
    }
}

/// Write `world_info.json` to the output directory.
pub fn write_metadata(output_dir: &Path, metadata: &WorldMetadata) -> Result<()> {
    let json = serde_json::to_string_pretty(metadata)?;
    let path = output_dir.join("world_info.json");
    std::fs::write(&path, json)?;
    log::info!("Wrote metadata to {}", path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn metadata_timer_measures_duration() {
        let timer = MetadataTimer::start();
        std::thread::sleep(std::time::Duration::from_millis(50));
        let timing = timer.finish();
        assert!(timing.duration_secs >= 0.04);
        assert!(!timing.started_at.is_empty());
    }

    #[test]
    fn build_metadata_populates_fields() {
        let params = ConvertParams {
            input: None,
            output: std::path::PathBuf::from("/tmp/test"),
            scale: 2.0,
            sea_level: 62,
            building_height: 10,
            wall_straighten_threshold: 1,
            spawn_x: None,
            spawn_y: None,
            spawn_z: None,
            spawn_lat: None,
            spawn_lon: None,
            signs: true,
            address_signs: false,
            poi_markers: false,
            poi_decorations: true,
            nature_decorations: true,
            filter: FeatureFilter::default(),
            elevation: None,
            vertical_scale: 1.0,
            elevation_smoothing: 1,
            surface_thickness: 4,
        };
        let data = OsmData {
            nodes: HashMap::from([(
                1,
                crate::osm::OsmNode {
                    lat: 51.5,
                    lon: -0.1,
                },
            )]),
            ways: vec![],
            ways_by_id: HashMap::new(),
            relations: vec![],
            bounds: Some((51.5, -0.1, 51.52, -0.08)),
            poi_nodes: vec![],
            addr_nodes: vec![],
            tree_nodes: vec![],
        };
        let timer = MetadataTimer::start();
        let meta = build_metadata(&params, &data, &timer, None);

        assert_eq!(meta.params.scale, 2.0);
        assert_eq!(meta.params.sea_level, 62);
        assert!(meta.params.signs);
        assert_eq!(meta.features.nodes, 1);
        assert!(meta.source.is_none());
        let bounds = meta.bounds.unwrap();
        assert!((bounds.south - 51.5).abs() < 0.001);
    }

    #[test]
    fn sha256_file_computes_hash() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.bin");
        std::fs::write(&path, b"hello world").unwrap();
        let hash = sha256_file(&path).unwrap();
        // Known SHA-256 of "hello world"
        assert_eq!(
            hash,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }

    #[test]
    fn write_metadata_creates_json_file() {
        let dir = tempfile::tempdir().unwrap();
        let params = ConvertParams {
            input: None,
            output: dir.path().to_path_buf(),
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
            filter: FeatureFilter::default(),
            elevation: None,
            vertical_scale: 1.0,
            elevation_smoothing: 1,
            surface_thickness: 4,
        };
        let data = OsmData {
            nodes: HashMap::new(),
            ways: vec![],
            ways_by_id: HashMap::new(),
            relations: vec![],
            bounds: None,
            poi_nodes: vec![],
            addr_nodes: vec![],
            tree_nodes: vec![],
        };
        let timer = MetadataTimer::start();
        let meta = build_metadata(&params, &data, &timer, None);
        write_metadata(dir.path(), &meta).unwrap();

        let json_path = dir.path().join("world_info.json");
        assert!(json_path.exists());
        let content: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&json_path).unwrap()).unwrap();
        assert_eq!(content["params"]["sea_level"], 65);
        assert_eq!(content["version"], env!("CARGO_PKG_VERSION"));
    }
}
