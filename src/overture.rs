//! Overture Maps integration via the `overturemaps` Python CLI.
//!
//! This module provides helpers for checking whether the Overture CLI is
//! installed on the system PATH, invoking it to download GeoJSON data for a
//! given theme and bounding box, and converting the resulting GeoJSON into
//! the `OsmData` structure used by the rest of the pipeline.
//!
//! The `overturemaps` CLI (PyPI: `overturemaps`) is an optional runtime
//! dependency — callers should check [`is_cli_available`] before attempting
//! any download.  If the CLI is absent, the integration is silently skipped.

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use crate::osm::{OsmData, OsmNode, OsmPoiNode, OsmWay};
use crate::params::{OvertureParams, OvertureTheme};

// ── Synthetic node-ID counter ─────────────────────────────────────────────

/// Atomic counter for synthetic negative node IDs.
///
/// Overture geometry nodes do not carry OSM IDs.  We assign synthetic
/// negative IDs starting at −1 000 000 000 to avoid any collision with
/// real OSM IDs (which are always positive).
static SYNTHETIC_ID_COUNTER: AtomicI64 = AtomicI64::new(-1_000_000_000);

/// Return the next unique synthetic (negative) node ID.
fn next_synthetic_id() -> i64 {
    SYNTHETIC_ID_COUNTER.fetch_sub(1, Ordering::Relaxed)
}

// ── CLI availability check ────────────────────────────────────────────────

/// Check whether the `overturemaps` CLI is available on the system PATH.
///
/// Runs `overturemaps --version` with a short timeout.  Returns `true` if
/// the command succeeds (exit code 0), `false` otherwise.
pub fn is_cli_available() -> bool {
    std::process::Command::new("overturemaps")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

// ── GeoJSON download via CLI ──────────────────────────────────────────────

/// Download Overture GeoJSON for a single CLI type and bounding box.
///
/// Invokes:
/// ```text
/// overturemaps download --bbox W,S,E,N -t <cli_type> -o <tmpfile>
/// ```
///
/// # Arguments
///
/// * `cli_type` – The Overture type string (e.g. `"building"`, `"segment"`).
/// * `bbox` – `(min_lat, min_lon, max_lat, max_lon)` bounding box.
/// * `timeout_secs` – Maximum wall-clock seconds to wait for the CLI.
///
/// # Returns
///
/// The GeoJSON string written by the CLI, or an error if the CLI fails or
/// times out.
pub fn fetch_geojson_for_type(
    cli_type: &str,
    bbox: (f64, f64, f64, f64),
    timeout_secs: u64,
) -> Result<String> {
    let (min_lat, min_lon, max_lat, max_lon) = bbox;
    // Overture CLI expects W,S,E,N order (min_lon, min_lat, max_lon, max_lat).
    let bbox_str = format!("{min_lon},{min_lat},{max_lon},{max_lat}");

    // Write output to a named temp file so the CLI can stream to disk.
    let tmp = tempfile::Builder::new()
        .suffix(".geojson")
        .tempfile()
        .context("creating temp file for overturemaps output")?;
    let tmp_path = tmp.path().to_path_buf();

    let mut child = std::process::Command::new("overturemaps")
        .args([
            "download",
            "-f",
            "geojson",
            "--bbox",
            &bbox_str,
            "-t",
            cli_type,
            "-o",
            tmp_path.to_str().unwrap_or_default(),
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .context("spawning overturemaps CLI")?;

    // Poll for completion up to the timeout using a background thread.
    let timeout = Duration::from_secs(timeout_secs);
    let start = std::time::Instant::now();
    let status = loop {
        match child.try_wait().context("polling overturemaps CLI")? {
            Some(s) => break s,
            None => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    bail!("overturemaps CLI timed out after {timeout_secs}s for type '{cli_type}'");
                }
                std::thread::sleep(Duration::from_millis(250));
            }
        }
    };

    if !status.success() {
        let stderr = child
            .stderr
            .take()
            .map(|mut s| {
                let mut buf = String::new();
                use std::io::Read;
                let _ = s.read_to_string(&mut buf);
                buf
            })
            .unwrap_or_default();
        let stderr_msg = if stderr.trim().is_empty() {
            String::new()
        } else {
            format!(": {}", stderr.trim())
        };
        bail!(
            "overturemaps CLI exited with status {} for type '{cli_type}'{stderr_msg}",
            status.code().unwrap_or(-1)
        );
    }

    let content = std::fs::read_to_string(&tmp_path)
        .with_context(|| format!("reading overturemaps output for type '{cli_type}'"))?;

    Ok(content)
}

// ── GeoJSON → OsmData conversion ─────────────────────────────────────────

/// Update a running bounding-box accumulator with a new coordinate.
fn update_bounds(
    min_lat: &mut f64,
    min_lon: &mut f64,
    max_lat: &mut f64,
    max_lon: &mut f64,
    lat: f64,
    lon: f64,
) {
    *min_lat = min_lat.min(lat);
    *min_lon = min_lon.min(lon);
    *max_lat = max_lat.max(lat);
    *max_lon = max_lon.max(lon);
}

/// Convert a GeoJSON coordinate array `[lon, lat]` or `[lon, lat, ele]` to an
/// `(OsmNode, i64)` pair and update the bounding-box accumulator.
///
/// Returns the synthetic node ID and the node, or `None` if the array is
/// malformed.
fn coord_to_node(
    coord: &Value,
    min_lat: &mut f64,
    min_lon: &mut f64,
    max_lat: &mut f64,
    max_lon: &mut f64,
) -> Option<(i64, OsmNode)> {
    let arr = coord.as_array()?;
    let lon = arr.first()?.as_f64()?;
    let lat = arr.get(1)?.as_f64()?;
    update_bounds(min_lat, min_lon, max_lat, max_lon, lat, lon);
    Some((next_synthetic_id(), OsmNode { lat, lon }))
}

/// Convert a GeoJSON coordinate array (ring or line) into a list of node IDs
/// and the corresponding node map entries.
///
/// Each element of `coords` is expected to be a `[lon, lat]` array.
fn coords_to_nodes(
    coords: &[Value],
    min_lat: &mut f64,
    min_lon: &mut f64,
    max_lat: &mut f64,
    max_lon: &mut f64,
) -> (Vec<i64>, HashMap<i64, OsmNode>) {
    let mut node_refs = Vec::with_capacity(coords.len());
    let mut nodes = HashMap::with_capacity(coords.len());
    for coord in coords {
        if let Some((id, node)) = coord_to_node(coord, min_lat, min_lon, max_lat, max_lon) {
            node_refs.push(id);
            nodes.insert(id, node);
        }
    }
    (node_refs, nodes)
}

/// Map an Overture place category string to the appropriate OSM primary key.
fn map_place_category_to_osm_key(category: &str) -> &'static str {
    match category {
        "restaurant" | "cafe" | "bar" | "fast_food" | "food_and_drink" => "amenity",
        "supermarket" | "grocery" | "clothing" | "electronics" | "retail" => "shop",
        "hotel" | "motel" | "hostel" | "accommodation" => "tourism",
        "park" | "playground" | "sports_centre" | "stadium" | "recreation" => "leisure",
        _ => "amenity",
    }
}

/// Map Overture feature properties to OSM-style tags for the given theme.
fn map_tags_for_theme(props: &Value, theme: OvertureTheme) -> HashMap<String, String> {
    let mut tags: HashMap<String, String> = HashMap::new();

    match theme {
        OvertureTheme::Building => {
            // class → building (default "yes")
            let class = props.get("class").and_then(|v| v.as_str()).unwrap_or("yes");
            tags.insert("building".into(), class.to_string());

            // height → building:height
            if let Some(h) = props.get("height").and_then(|v| v.as_f64()) {
                tags.insert("building:height".into(), h.to_string());
            }
            // num_floors → building:levels
            if let Some(f) = props.get("num_floors").and_then(|v| v.as_u64()) {
                tags.insert("building:levels".into(), f.to_string());
            }
        }

        OvertureTheme::Transportation => {
            // class → highway (default "unclassified")
            let class = props
                .get("class")
                .and_then(|v| v.as_str())
                .unwrap_or("unclassified");
            tags.insert("highway".into(), class.to_string());

            // names.primary → name
            if let Some(name) = props
                .get("names")
                .and_then(|n| n.get("primary"))
                .and_then(|v| v.as_str())
            {
                tags.insert("name".into(), name.to_string());
            }
            // road_surface → surface
            if let Some(surface) = props.get("road_surface").and_then(|v| v.as_str()) {
                tags.insert("surface".into(), surface.to_string());
            }
            // is_bridge → bridge=yes
            if props
                .get("is_bridge")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                tags.insert("bridge".into(), "yes".into());
            }
            // is_tunnel → tunnel=yes
            if props
                .get("is_tunnel")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                tags.insert("tunnel".into(), "yes".into());
            }
        }

        OvertureTheme::Place => {
            // categories.primary → amenity / shop / tourism / leisure
            if let Some(category) = props
                .get("categories")
                .and_then(|c| c.get("primary"))
                .and_then(|v| v.as_str())
            {
                let osm_key = map_place_category_to_osm_key(category);
                tags.insert(osm_key.into(), category.to_string());
            }
            // names.primary → name
            if let Some(name) = props
                .get("names")
                .and_then(|n| n.get("primary"))
                .and_then(|v| v.as_str())
            {
                tags.insert("name".into(), name.to_string());
            }
        }

        OvertureTheme::Base => {
            // Overture Base uses "subtype" and "class" to distinguish features.
            // We map them to the appropriate OSM keys.
            let subtype = props.get("subtype").and_then(|v| v.as_str()).unwrap_or("");
            let class = props.get("class").and_then(|v| v.as_str()).unwrap_or("");

            // Water bodies
            if matches!(
                subtype,
                "water" | "lake" | "pond" | "reservoir" | "ocean" | "sea"
            ) {
                tags.insert("natural".into(), "water".into());
                if !subtype.is_empty() && subtype != "water" {
                    tags.insert("water".into(), subtype.to_string());
                }
            }
            // Waterways
            else if matches!(subtype, "river" | "stream" | "canal" | "drain" | "ditch") {
                tags.insert("waterway".into(), subtype.to_string());
            }
            // Land use — from class when subtype indicates land_use
            else if matches!(
                subtype,
                "forest"
                    | "farmland"
                    | "residential"
                    | "commercial"
                    | "industrial"
                    | "cemetery"
                    | "grass"
                    | "scrub"
                    | "farmyard"
            ) {
                tags.insert("landuse".into(), subtype.to_string());
            }
            // Natural land cover from class
            else if matches!(subtype, "land" | "")
                && matches!(
                    class,
                    "grass" | "scrub" | "heath" | "bare_rock" | "sand" | "beach"
                )
            {
                tags.insert("natural".into(), class.to_string());
            }
            // Leisure areas
            else if matches!(subtype, "park" | "garden" | "pitch" | "playground") {
                tags.insert("leisure".into(), subtype.to_string());
            }
            // Individual tree points
            else if subtype == "tree" {
                tags.insert("natural".into(), "tree".into());
            }
            // Fallback: try the class field
            else if !class.is_empty() {
                tags.insert("landuse".into(), class.to_string());
            }
        }

        OvertureTheme::Address => {
            // number → addr:housenumber
            if let Some(number) = props.get("number").and_then(|v| v.as_str()) {
                tags.insert("addr:housenumber".into(), number.to_string());
            }
            // street → addr:street
            if let Some(street) = props.get("street").and_then(|v| v.as_str()) {
                tags.insert("addr:street".into(), street.to_string());
            }
        }
    }

    tags
}

/// Parse an Overture GeoJSON `FeatureCollection` string into an [`OsmData`].
///
/// Each GeoJSON feature is converted according to `theme`:
///
/// - `Point` geometries become POI nodes (Place theme) or address nodes (Address theme).
/// - `LineString` geometries become ways.
/// - `Polygon` geometries become ways using the outer ring.
/// - `MultiPolygon` geometries produce one way per polygon outer ring.
///
/// Synthetic negative node IDs are assigned to avoid collision with OSM IDs.
pub fn parse_overture_geojson(geojson_str: &str, theme: OvertureTheme) -> Result<OsmData> {
    let root: Value = serde_json::from_str(geojson_str).context("parsing Overture GeoJSON")?;

    let features = root
        .get("features")
        .and_then(|f| f.as_array())
        .map(|a| a.as_slice())
        .unwrap_or(&[]);

    let mut nodes: HashMap<i64, OsmNode> = HashMap::new();
    let mut ways: Vec<OsmWay> = Vec::new();
    let mut ways_by_id: HashMap<i64, usize> = HashMap::new();
    let mut poi_nodes: Vec<OsmPoiNode> = Vec::new();
    let mut addr_nodes: Vec<OsmPoiNode> = Vec::new();
    let mut tree_nodes: Vec<OsmNode> = Vec::new();

    let mut min_lat = f64::MAX;
    let mut min_lon = f64::MAX;
    let mut max_lat = f64::MIN;
    let mut max_lon = f64::MIN;

    for feature in features {
        let props = feature.get("properties").unwrap_or(&Value::Null);
        let tags = map_tags_for_theme(props, theme);

        let geometry = match feature.get("geometry") {
            Some(g) => g,
            None => continue,
        };
        let geom_type = geometry.get("type").and_then(|t| t.as_str()).unwrap_or("");
        let coordinates = geometry.get("coordinates");

        match geom_type {
            "Point" => {
                if let Some(coord) = coordinates
                    && let Some((id, node)) = coord_to_node(
                        coord,
                        &mut min_lat,
                        &mut min_lon,
                        &mut max_lat,
                        &mut max_lon,
                    )
                {
                    nodes.insert(id, node);
                    let poi = OsmPoiNode {
                        lat: node.lat,
                        lon: node.lon,
                        tags: tags.clone(),
                    };
                    match theme {
                        OvertureTheme::Address => addr_nodes.push(poi),
                        OvertureTheme::Place => poi_nodes.push(poi),
                        _ => {
                            // Decorative tree nodes from land theme
                            if tags.get("natural").map(|s| s.as_str()) == Some("tree") {
                                tree_nodes.push(OsmNode {
                                    lat: node.lat,
                                    lon: node.lon,
                                });
                            }
                        }
                    }
                }
            }

            "LineString" => {
                if let Some(coords) = coordinates.and_then(|c| c.as_array()) {
                    let (node_refs, new_nodes) = coords_to_nodes(
                        coords,
                        &mut min_lat,
                        &mut min_lon,
                        &mut max_lat,
                        &mut max_lon,
                    );
                    if !node_refs.is_empty() {
                        let way_id = next_synthetic_id();
                        let idx = ways.len();
                        ways.push(OsmWay { tags, node_refs });
                        ways_by_id.insert(way_id, idx);
                        nodes.extend(new_nodes);
                    }
                }
            }

            "Polygon" => {
                // Use the outer ring (first element).
                if let Some(outer_ring) = coordinates
                    .and_then(|c| c.as_array())
                    .and_then(|rings| rings.first())
                    .and_then(|r| r.as_array())
                {
                    let (node_refs, new_nodes) = coords_to_nodes(
                        outer_ring,
                        &mut min_lat,
                        &mut min_lon,
                        &mut max_lat,
                        &mut max_lon,
                    );
                    if !node_refs.is_empty() {
                        let way_id = next_synthetic_id();
                        let idx = ways.len();
                        ways.push(OsmWay { tags, node_refs });
                        ways_by_id.insert(way_id, idx);
                        nodes.extend(new_nodes);
                    }
                }
            }

            "MultiPolygon" => {
                // Each polygon produces one way from its outer ring.
                if let Some(polygons) = coordinates.and_then(|c| c.as_array()) {
                    for polygon in polygons {
                        if let Some(outer_ring) = polygon
                            .as_array()
                            .and_then(|rings| rings.first())
                            .and_then(|r| r.as_array())
                        {
                            let (node_refs, new_nodes) = coords_to_nodes(
                                outer_ring,
                                &mut min_lat,
                                &mut min_lon,
                                &mut max_lat,
                                &mut max_lon,
                            );
                            if !node_refs.is_empty() {
                                let way_id = next_synthetic_id();
                                let idx = ways.len();
                                ways.push(OsmWay {
                                    tags: tags.clone(),
                                    node_refs,
                                });
                                ways_by_id.insert(way_id, idx);
                                nodes.extend(new_nodes);
                            }
                        }
                    }
                }
            }

            _ => {
                // Unknown geometry type — skip.
            }
        }
    }

    let bounds = if min_lat < f64::MAX {
        Some((min_lat, min_lon, max_lat, max_lon))
    } else {
        None
    };

    Ok(OsmData {
        nodes,
        ways,
        ways_by_id,
        relations: Vec::new(),
        bounds,
        poi_nodes,
        addr_nodes,
        tree_nodes,
    })
}

// ── Overture cache ────────────────────────────────────────────────────────

/// Serialised metadata stored alongside the `.geojson` cache file.
#[derive(Debug, Serialize, Deserialize)]
pub struct OvertureCacheMeta {
    pub bbox: [f64; 4],
    pub cli_type: String,
    pub created_at: DateTime<Utc>,
    pub size_bytes: u64,
}

/// Return the Overture GeoJSON cache directory, creating it if needed.
///
/// Priority:
/// 1. `OVERTURE_CACHE_DIR` environment variable
/// 2. `$HOME/.cache/osm-to-bedrock/overture/` (Linux / macOS)
/// 3. `%LOCALAPPDATA%\osm-to-bedrock\overture` (Windows)
/// 4. `<system-temp>/osm-to-bedrock-overture` (fallback)
pub fn overture_cache_dir() -> PathBuf {
    let dir = if let Ok(override_dir) = std::env::var("OVERTURE_CACHE_DIR") {
        PathBuf::from(override_dir)
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home)
            .join(".cache")
            .join("osm-to-bedrock")
            .join("overture")
    } else if let Ok(local) = std::env::var("LOCALAPPDATA") {
        PathBuf::from(local).join("osm-to-bedrock").join("overture")
    } else {
        std::env::temp_dir().join("osm-to-bedrock-overture")
    };

    if let Err(e) = std::fs::create_dir_all(&dir) {
        log::warn!("Could not create Overture cache dir {}: {e}", dir.display());
    }
    dir
}

/// Build a deterministic SHA-256 cache key from a bounding box and CLI type.
///
/// Coordinates are snapped to 4 decimal places (~11 m) so small UI drags
/// reuse the same entry.
pub fn overture_cache_key(bbox: (f64, f64, f64, f64), cli_type: &str) -> String {
    let (s, w, n, e) = bbox;
    let canonical = format!("overture|{s:.4},{w:.4},{n:.4},{e:.4}|{cli_type}");
    let hash = Sha256::digest(canonical.as_bytes());
    format!("{hash:x}")
}

/// Return cached GeoJSON for `key`, or `None` if absent or unreadable.
pub fn overture_cache_read(dir: &Path, key: &str) -> Option<String> {
    let path = dir.join(format!("{key}.geojson"));
    match std::fs::read_to_string(&path) {
        Ok(s) => Some(s),
        Err(e) => {
            log::debug!("Overture cache miss for {key}: {e}");
            None
        }
    }
}

/// Atomically write `geojson` + metadata for `key`.
pub fn overture_cache_write(
    dir: &Path,
    key: &str,
    bbox: (f64, f64, f64, f64),
    cli_type: &str,
    geojson: &str,
) -> Result<()> {
    let (s, w, n, e) = bbox;
    let geojson_path = dir.join(format!("{key}.geojson"));
    let meta_path = dir.join(format!("{key}.meta.json"));
    let geojson_tmp = dir.join(format!("{key}.geojson.tmp"));
    let meta_tmp = dir.join(format!("{key}.meta.json.tmp"));

    // Atomic write: write to .tmp then rename
    std::fs::write(&geojson_tmp, geojson)?;
    std::fs::rename(&geojson_tmp, &geojson_path)?;

    let size_bytes = geojson.len() as u64;
    let meta = OvertureCacheMeta {
        bbox: [s, w, n, e],
        cli_type: cli_type.to_string(),
        created_at: Utc::now(),
        size_bytes,
    };
    std::fs::write(&meta_tmp, serde_json::to_string(&meta)?)?;
    std::fs::rename(&meta_tmp, &meta_path)?;

    Ok(())
}

/// A single Overture cache entry for listing purposes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OvertureCacheEntry {
    pub key: String,
    pub bbox: [f64; 4],
    pub cli_type: String,
    pub created_at: DateTime<Utc>,
    pub size_bytes: u64,
}

/// List all valid Overture cache entries.
pub fn list_overture_areas() -> Vec<OvertureCacheEntry> {
    let dir = overture_cache_dir();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut result = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(key) = name.strip_suffix(".meta.json") else {
            continue;
        };
        let geojson_path = dir.join(format!("{key}.geojson"));
        if !geojson_path.exists() {
            continue;
        }
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(meta) = serde_json::from_str::<OvertureCacheMeta>(&raw) else {
            continue;
        };
        result.push(OvertureCacheEntry {
            key: key.to_string(),
            bbox: meta.bbox,
            cli_type: meta.cli_type,
            created_at: meta.created_at,
            size_bytes: meta.size_bytes,
        });
    }
    result
}

/// Clear Overture cache entries, optionally only those older than `min_age`.
///
/// Returns the number of entries deleted.
pub fn clear_overture_cache(min_age: Option<chrono::Duration>) -> Result<usize> {
    clear_overture_cache_dir(&overture_cache_dir(), min_age)
}

fn clear_overture_cache_dir(dir: &Path, min_age: Option<chrono::Duration>) -> Result<usize> {
    if !dir.exists() {
        log::info!("Overture cache dir does not exist; nothing to clear");
        return Ok(0);
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(0);
    };
    let now = Utc::now();
    let mut deleted = 0usize;

    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(key) = name.strip_suffix(".meta.json") else {
            // Remove orphaned .geojson files (no paired .meta.json)
            if let Some(stem) = name.strip_suffix(".geojson") {
                let meta_name = format!("{stem}.meta.json");
                if !dir.join(&meta_name).exists() {
                    let _ = std::fs::remove_file(&path);
                }
            }
            continue;
        };
        if let Some(min_age) = min_age {
            let Ok(raw) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Ok(meta) = serde_json::from_str::<OvertureCacheMeta>(&raw) else {
                continue;
            };
            let age = now.signed_duration_since(meta.created_at);
            if age <= min_age {
                continue; // fresh — keep it
            }
        }
        let geojson_path = dir.join(format!("{key}.geojson"));
        let meta_path = dir.join(format!("{key}.meta.json"));
        let _ = std::fs::remove_file(&geojson_path);
        let _ = std::fs::remove_file(&meta_path);
        deleted += 1;
    }
    Ok(deleted)
}

// ── High-level fetch API ──────────────────────────────────────────────────

/// Create an empty [`OsmData`] to accumulate merged results into.
fn empty_osm_data() -> OsmData {
    OsmData {
        nodes: HashMap::new(),
        ways: vec![],
        ways_by_id: HashMap::new(),
        relations: vec![],
        bounds: None,
        poi_nodes: vec![],
        addr_nodes: vec![],
        tree_nodes: vec![],
    }
}

/// Fetch and parse Overture Maps data for all enabled themes, merging into a
/// single [`OsmData`].
///
/// For each CLI type belonging to each requested theme:
/// 1. Check the disk cache.
/// 2. On cache miss, invoke the `overturemaps` CLI to download GeoJSON.
/// 3. Write the result to cache.
/// 4. Parse the GeoJSON into `OsmData` and merge.
///
/// # Errors
///
/// Returns an error if `params.enabled` is false, the CLI is not installed,
/// or any theme fetch or parse fails.
pub fn fetch_overture_data(
    bbox: (f64, f64, f64, f64),
    params: &OvertureParams,
    progress_cb: &mut dyn FnMut(f32, &str),
) -> Result<OsmData> {
    if !params.enabled {
        bail!("Overture Maps integration is not enabled");
    }
    if !is_cli_available() {
        bail!(
            "The `overturemaps` CLI is not installed.\n\
             Install it with: pip install overturemaps\n\
             Then retry."
        );
    }

    let theme_names: Vec<String> = params.themes.iter().map(|t| t.to_string()).collect();
    log::info!(
        "Starting Overture Maps fetch (bbox: {:.4},{:.4},{:.4},{:.4}, themes: {})",
        bbox.0,
        bbox.1,
        bbox.2,
        bbox.3,
        theme_names.join(", ")
    );

    let cache_dir = overture_cache_dir();

    // Flatten all (theme, cli_type) pairs so we can report progress as a
    // fraction of total work.
    let pairs: Vec<(OvertureTheme, &'static str)> = params
        .themes
        .iter()
        .flat_map(|&theme| theme.cli_types().into_iter().map(move |t| (theme, t)))
        .collect();

    let total = pairs.len() as f32;
    let mut accumulated = empty_osm_data();

    for (i, (theme, cli_type)) in pairs.iter().enumerate() {
        let pct = i as f32 / total;
        progress_cb(pct, &format!("Fetching Overture {cli_type}…"));

        let key = overture_cache_key(bbox, cli_type);
        let geojson = if let Some(cached) = overture_cache_read(&cache_dir, &key) {
            log::debug!("Overture cache hit for {cli_type} (key {key})");
            cached
        } else {
            log::debug!("Overture cache miss for {cli_type} — downloading");
            let fetched = fetch_geojson_for_type(cli_type, bbox, params.timeout_secs)
                .with_context(|| format!("fetching Overture data for type '{cli_type}'"))?;
            overture_cache_write(&cache_dir, &key, bbox, cli_type, &fetched)
                .with_context(|| format!("caching Overture data for type '{cli_type}'"))?;
            fetched
        };

        let data = parse_overture_geojson(&geojson, *theme)
            .with_context(|| format!("parsing Overture GeoJSON for type '{cli_type}'"))?;
        accumulated.merge(data);
    }

    log::info!(
        "Overture Maps fetch complete ({} ways, {} POI nodes, {} address nodes)",
        accumulated.ways.len(),
        accumulated.poi_nodes.len(),
        accumulated.addr_nodes.len(),
    );
    progress_cb(1.0, "Overture data ready");
    Ok(accumulated)
}

/// Like [`fetch_overture_data`] but never fails.
///
/// - If the CLI is unavailable, returns empty [`OsmData`] after logging a warning.
/// - If a theme fetch fails, logs a warning and skips it.
/// - If parsing a GeoJSON result fails, logs a warning and skips it.
pub fn fetch_overture_data_best_effort(
    bbox: (f64, f64, f64, f64),
    params: &OvertureParams,
    progress_cb: &mut dyn FnMut(f32, &str),
) -> OsmData {
    if !params.enabled {
        return empty_osm_data();
    }
    if !is_cli_available() {
        log::warn!(
            "Overture Maps CLI not available — skipping Overture data.\n\
             Install with: pip install overturemaps"
        );
        return empty_osm_data();
    }

    let cache_dir = overture_cache_dir();

    let pairs: Vec<(OvertureTheme, &'static str)> = params
        .themes
        .iter()
        .flat_map(|&theme| theme.cli_types().into_iter().map(move |t| (theme, t)))
        .collect();

    let total = pairs.len() as f32;
    let mut accumulated = empty_osm_data();

    for (i, (theme, cli_type)) in pairs.iter().enumerate() {
        let pct = i as f32 / total;
        progress_cb(pct, &format!("Fetching Overture {cli_type}…"));

        let key = overture_cache_key(bbox, cli_type);
        let geojson = if let Some(cached) = overture_cache_read(&cache_dir, &key) {
            cached
        } else {
            match fetch_geojson_for_type(cli_type, bbox, params.timeout_secs) {
                Ok(fetched) => {
                    if let Err(e) = overture_cache_write(&cache_dir, &key, bbox, cli_type, &fetched)
                    {
                        log::warn!("Failed to write Overture cache for {cli_type}: {e}");
                    }
                    fetched
                }
                Err(e) => {
                    log::warn!("Skipping Overture type '{cli_type}': {e}");
                    continue;
                }
            }
        };

        match parse_overture_geojson(&geojson, *theme) {
            Ok(data) => accumulated.merge(data),
            Err(e) => {
                log::warn!("Failed to parse Overture GeoJSON for '{cli_type}': {e}");
            }
        }
    }

    progress_cb(1.0, "Overture data ready");
    accumulated
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── helpers ──────────────────────────────────────────────────────────

    fn point_feature(lon: f64, lat: f64, props: serde_json::Value) -> String {
        serde_json::json!({
            "type": "FeatureCollection",
            "features": [{
                "type": "Feature",
                "geometry": {
                    "type": "Point",
                    "coordinates": [lon, lat]
                },
                "properties": props
            }]
        })
        .to_string()
    }

    fn polygon_feature(props: serde_json::Value) -> String {
        // A simple 4-corner square polygon.
        serde_json::json!({
            "type": "FeatureCollection",
            "features": [{
                "type": "Feature",
                "geometry": {
                    "type": "Polygon",
                    "coordinates": [[
                        [0.0, 0.0],
                        [0.0, 1.0],
                        [1.0, 1.0],
                        [1.0, 0.0],
                        [0.0, 0.0]
                    ]]
                },
                "properties": props
            }]
        })
        .to_string()
    }

    fn line_feature(props: serde_json::Value) -> String {
        serde_json::json!({
            "type": "FeatureCollection",
            "features": [{
                "type": "Feature",
                "geometry": {
                    "type": "LineString",
                    "coordinates": [
                        [0.0, 0.0],
                        [0.0, 1.0],
                        [1.0, 1.0]
                    ]
                },
                "properties": props
            }]
        })
        .to_string()
    }

    // ── Building tests ───────────────────────────────────────────────────

    #[test]
    fn building_with_class_height_floors() {
        let geojson = polygon_feature(serde_json::json!({
            "class": "residential",
            "height": 12.5,
            "num_floors": 4
        }));
        let data = parse_overture_geojson(&geojson, OvertureTheme::Building).unwrap();
        assert_eq!(data.ways.len(), 1);
        let tags = &data.ways[0].tags;
        assert_eq!(tags["building"], "residential");
        assert_eq!(tags["building:height"], "12.5");
        assert_eq!(tags["building:levels"], "4");
    }

    #[test]
    fn building_no_class_defaults_yes() {
        let geojson = polygon_feature(serde_json::json!({}));
        let data = parse_overture_geojson(&geojson, OvertureTheme::Building).unwrap();
        assert_eq!(data.ways.len(), 1);
        assert_eq!(data.ways[0].tags["building"], "yes");
    }

    // ── Transportation tests ─────────────────────────────────────────────

    #[test]
    fn transportation_all_fields() {
        let geojson = line_feature(serde_json::json!({
            "class": "primary",
            "names": { "primary": "Main Street" },
            "road_surface": "paved",
            "is_bridge": true,
            "is_tunnel": false
        }));
        let data = parse_overture_geojson(&geojson, OvertureTheme::Transportation).unwrap();
        assert_eq!(data.ways.len(), 1);
        let tags = &data.ways[0].tags;
        assert_eq!(tags["highway"], "primary");
        assert_eq!(tags["name"], "Main Street");
        assert_eq!(tags["surface"], "paved");
        assert_eq!(tags["bridge"], "yes");
        assert!(!tags.contains_key("tunnel"));
    }

    #[test]
    fn transportation_no_class_defaults_unclassified() {
        let geojson = line_feature(serde_json::json!({}));
        let data = parse_overture_geojson(&geojson, OvertureTheme::Transportation).unwrap();
        assert_eq!(data.ways[0].tags["highway"], "unclassified");
    }

    // ── Place tests ──────────────────────────────────────────────────────

    #[test]
    fn place_becomes_poi_node() {
        let geojson = point_feature(
            -0.1,
            51.5,
            serde_json::json!({
                "categories": { "primary": "restaurant" },
                "names": { "primary": "The Bistro" }
            }),
        );
        let data = parse_overture_geojson(&geojson, OvertureTheme::Place).unwrap();
        assert_eq!(data.poi_nodes.len(), 1);
        assert_eq!(data.poi_nodes[0].tags["amenity"], "restaurant");
        assert_eq!(data.poi_nodes[0].tags["name"], "The Bistro");
        assert!((data.poi_nodes[0].lat - 51.5).abs() < 1e-9);
        assert!((data.poi_nodes[0].lon - -0.1).abs() < 1e-9);
    }

    // ── Base theme tests ─────────────────────────────────────────────────

    #[test]
    fn base_water_subtype_maps_to_natural_water() {
        let geojson = polygon_feature(serde_json::json!({
            "subtype": "lake",
            "class": "lake"
        }));
        let data = parse_overture_geojson(&geojson, OvertureTheme::Base).unwrap();
        assert_eq!(data.ways.len(), 1);
        assert_eq!(data.ways[0].tags["natural"], "water");
        assert_eq!(data.ways[0].tags["water"], "lake");
    }

    #[test]
    fn base_landuse_forest_subtype() {
        let geojson = polygon_feature(serde_json::json!({
            "subtype": "forest",
            "class": "forest"
        }));
        let data = parse_overture_geojson(&geojson, OvertureTheme::Base).unwrap();
        assert_eq!(data.ways.len(), 1);
        assert_eq!(data.ways[0].tags["landuse"], "forest");
    }

    // ── Address tests ────────────────────────────────────────────────────

    #[test]
    fn address_becomes_addr_node() {
        let geojson = point_feature(
            -0.2,
            51.6,
            serde_json::json!({
                "number": "42",
                "street": "Baker Street"
            }),
        );
        let data = parse_overture_geojson(&geojson, OvertureTheme::Address).unwrap();
        assert_eq!(data.addr_nodes.len(), 1);
        assert_eq!(data.addr_nodes[0].tags["addr:housenumber"], "42");
        assert_eq!(data.addr_nodes[0].tags["addr:street"], "Baker Street");
        // Should NOT appear in poi_nodes.
        assert_eq!(data.poi_nodes.len(), 0);
    }

    // ── Edge cases ───────────────────────────────────────────────────────

    #[test]
    fn empty_feature_collection_returns_empty_osm_data() {
        let geojson = r#"{"type":"FeatureCollection","features":[]}"#;
        let data = parse_overture_geojson(geojson, OvertureTheme::Building).unwrap();
        assert!(data.nodes.is_empty());
        assert!(data.ways.is_empty());
        assert!(data.poi_nodes.is_empty());
        assert!(data.addr_nodes.is_empty());
        assert!(data.bounds.is_none());
    }

    #[test]
    fn multipolygon_produces_multiple_ways() {
        let geojson = serde_json::json!({
            "type": "FeatureCollection",
            "features": [{
                "type": "Feature",
                "geometry": {
                    "type": "MultiPolygon",
                    "coordinates": [
                        [[[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [0.0, 0.0]]],
                        [[[2.0, 2.0], [2.0, 3.0], [3.0, 3.0], [2.0, 2.0]]]
                    ]
                },
                "properties": { "class": "office" }
            }]
        })
        .to_string();
        let data = parse_overture_geojson(&geojson, OvertureTheme::Building).unwrap();
        assert_eq!(data.ways.len(), 2);
    }

    #[test]
    fn bounds_computed_correctly() {
        let geojson = polygon_feature(serde_json::json!({}));
        let data = parse_overture_geojson(&geojson, OvertureTheme::Building).unwrap();
        let (min_lat, min_lon, max_lat, max_lon) = data.bounds.unwrap();
        assert!((min_lat - 0.0).abs() < 1e-9);
        assert!((min_lon - 0.0).abs() < 1e-9);
        assert!((max_lat - 1.0).abs() < 1e-9);
        assert!((max_lon - 1.0).abs() < 1e-9);
    }

    // ── Cache tests ──────────────────────────────────────────────────────

    #[test]
    fn overture_cache_key_is_deterministic() {
        let bbox = (51.5, -0.13, 51.52, -0.10);
        let k1 = overture_cache_key(bbox, "building");
        let k2 = overture_cache_key(bbox, "building");
        assert_eq!(k1, k2);
        assert_eq!(k1.len(), 64, "SHA-256 hex should be 64 chars");
    }

    #[test]
    fn overture_cache_key_varies_by_theme() {
        let bbox = (51.5, -0.13, 51.52, -0.10);
        let k1 = overture_cache_key(bbox, "building");
        let k2 = overture_cache_key(bbox, "segment");
        assert_ne!(k1, k2);
    }

    #[test]
    fn overture_cache_write_read_roundtrip() {
        let tmp = tempfile::tempdir().expect("tmpdir");
        let bbox = (51.5_f64, -0.13_f64, 51.52_f64, -0.10_f64);
        let key = overture_cache_key(bbox, "building");
        let geojson = r#"{"type":"FeatureCollection","features":[]}"#;

        overture_cache_write(tmp.path(), &key, bbox, "building", geojson).unwrap();
        let result = overture_cache_read(tmp.path(), &key);
        assert_eq!(result.as_deref(), Some(geojson));
    }
}
