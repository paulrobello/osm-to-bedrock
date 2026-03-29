//! Disk cache for raw Overpass XML responses.
//!
//! Layout: `~/.cache/osm-to-bedrock/overpass/{sha256}.xml` + `{sha256}.meta.json`
//! Key:    SHA-256 of `"{s:.4},{w:.4},{n:.4},{e:.4}|roads={},buildings={},water={},landuse={},railways={}"`

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

use crate::filter::FeatureFilter;

// ── Types ──────────────────────────────────────────────────────────────────

/// A single entry returned by [`list_areas`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheEntry {
    pub key: String,
    pub bbox: [f64; 4], // [south, west, north, east]
    pub filter: FeatureFilter,
    pub created_at: DateTime<Utc>,
    pub size_bytes: u64,
}

/// Serialised metadata stored alongside the `.xml` file.
#[derive(Debug, Serialize, Deserialize)]
struct CacheMeta {
    bbox: [f64; 4],
    filter: FeatureFilter,
    created_at: DateTime<Utc>,
    size_bytes: u64,
}

// ── Cache directory ────────────────────────────────────────────────────────

/// Return the persistent Overpass XML cache directory, creating it if needed.
///
/// Priority:
/// 1. `OVERPASS_CACHE_DIR` environment variable (override for servers / CI)
/// 2. `$HOME/.cache/osm-to-bedrock/overpass` (Linux / macOS XDG-style)
/// 3. `%LOCALAPPDATA%\osm-to-bedrock\overpass` (Windows)
/// 4. `<system-temp>/osm-to-bedrock-overpass` (fallback)
pub fn cache_dir() -> PathBuf {
    let dir = if let Ok(override_dir) = std::env::var("OVERPASS_CACHE_DIR") {
        PathBuf::from(override_dir)
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home)
            .join(".cache")
            .join("osm-to-bedrock")
            .join("overpass")
    } else if let Ok(local) = std::env::var("LOCALAPPDATA") {
        PathBuf::from(local).join("osm-to-bedrock").join("overpass")
    } else {
        std::env::temp_dir().join("osm-to-bedrock-overpass")
    };

    if let Err(e) = std::fs::create_dir_all(&dir) {
        log::warn!("Could not create Overpass cache dir {}: {e}", dir.display());
    }
    dir
}

// ── Cache key ──────────────────────────────────────────────────────────────

/// Build a deterministic SHA-256 cache key from a bounding box and feature filter.
///
/// Coords are snapped to 4 decimal places (~11 m) so small UI drags reuse the
/// same entry.
pub fn cache_key(bbox: (f64, f64, f64, f64), filter: &FeatureFilter) -> String {
    let (s, w, n, e) = bbox;
    let canonical = format!(
        "{:.4},{:.4},{:.4},{:.4}|roads={},buildings={},water={},landuse={},railways={}",
        s,
        w,
        n,
        e,
        u8::from(filter.roads),
        u8::from(filter.buildings),
        u8::from(filter.water),
        u8::from(filter.landuse),
        u8::from(filter.railways),
    );
    let hash = Sha256::digest(canonical.as_bytes());
    format!("{hash:x}")
}

// ── Public API ─────────────────────────────────────────────────────────────

/// Return cached XML for `key`, or `None` if not present or unreadable.
pub fn read(key: &str) -> Option<String> {
    read_from(&cache_dir(), key)
}

/// Atomically write `xml` + metadata for `key`.
pub fn write(
    key: &str,
    bbox: (f64, f64, f64, f64),
    filter: &FeatureFilter,
    xml: &str,
) -> Result<()> {
    write_to(&cache_dir(), key, bbox, filter, xml)
}

/// List all valid (paired) cache entries in the default cache directory.
pub fn list_areas() -> Vec<CacheEntry> {
    list_areas_in(&cache_dir())
}

/// Delete cache entries older than `min_age` (or all if `None`).
/// Returns the number of entries deleted.
pub fn clear(min_age: Option<chrono::Duration>) -> Result<usize> {
    clear_dir(&cache_dir(), min_age)
}

// ── Internal helpers (used directly by tests via explicit path arg) ────────

fn read_from(dir: &std::path::Path, key: &str) -> Option<String> {
    let xml_path = dir.join(format!("{key}.xml"));
    match std::fs::read_to_string(&xml_path) {
        Ok(s) => Some(s),
        Err(e) => {
            log::warn!("Cache read failed for {key}: {e}");
            None
        }
    }
}

fn write_to(
    dir: &std::path::Path,
    key: &str,
    bbox: (f64, f64, f64, f64),
    filter: &FeatureFilter,
    xml: &str,
) -> Result<()> {
    let (s, w, n, e) = bbox;
    let xml_path = dir.join(format!("{key}.xml"));
    let meta_path = dir.join(format!("{key}.meta.json"));
    let xml_tmp = dir.join(format!("{key}.xml.tmp"));
    let meta_tmp = dir.join(format!("{key}.meta.json.tmp"));

    // Atomic write: write to .tmp then rename
    std::fs::write(&xml_tmp, xml)?;
    std::fs::rename(&xml_tmp, &xml_path)?;

    let size_bytes = xml.len() as u64;
    let meta = CacheMeta {
        bbox: [s, w, n, e],
        filter: filter.clone(),
        created_at: Utc::now(),
        size_bytes,
    };
    std::fs::write(&meta_tmp, serde_json::to_string(&meta)?)?;
    std::fs::rename(&meta_tmp, &meta_path)?;

    Ok(())
}

fn list_areas_in(dir: &std::path::Path) -> Vec<CacheEntry> {
    let Ok(entries) = std::fs::read_dir(dir) else {
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
        let xml_path = dir.join(format!("{key}.xml"));
        if !xml_path.exists() {
            continue; // orphaned meta — skip
        }
        let Ok(raw) = std::fs::read_to_string(&path) else {
            log::warn!("Skipping unreadable cache meta: {}", path.display());
            continue;
        };
        let Ok(meta) = serde_json::from_str::<CacheMeta>(&raw) else {
            log::warn!("Skipping malformed cache meta: {}", path.display());
            continue;
        };
        result.push(CacheEntry {
            key: key.to_string(),
            bbox: meta.bbox,
            filter: meta.filter,
            created_at: meta.created_at,
            size_bytes: meta.size_bytes,
        });
    }
    result
}

// ── Containment lookup ─────────────────────────────────────────────────────

/// Return cached XML for the first entry whose bbox fully contains `bbox`
/// and whose filter exactly matches `filter`.
///
/// Containment: cached_s ≤ req_s && cached_w ≤ req_w && cached_n ≥ req_n && cached_e ≥ req_e
#[allow(dead_code)]
pub fn find_containing(bbox: (f64, f64, f64, f64), filter: &FeatureFilter) -> Option<String> {
    find_containing_in(&cache_dir(), bbox, filter)
}

fn find_containing_in(
    dir: &std::path::Path,
    bbox: (f64, f64, f64, f64),
    filter: &FeatureFilter,
) -> Option<String> {
    let (req_s, req_w, req_n, req_e) = bbox;
    for entry in list_areas_in(dir) {
        let [cs, cw, cn, ce] = entry.bbox;
        let contained = cs <= req_s && cw <= req_w && cn >= req_n && ce >= req_e;
        let filter_matches = entry.filter == *filter;
        if contained && filter_matches {
            return read_from(dir, &entry.key);
        }
    }
    None
}

fn clear_dir(dir: &std::path::Path, min_age: Option<chrono::Duration>) -> Result<usize> {
    if !dir.exists() {
        log::info!("Cache dir does not exist; nothing to clear");
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
            // Remove orphaned .xml files (no paired .meta.json)
            if let Some(stem) = name.strip_suffix(".xml") {
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
            let Ok(meta) = serde_json::from_str::<CacheMeta>(&raw) else {
                continue;
            };
            let age = now.signed_duration_since(meta.created_at);
            if age <= min_age {
                continue; // fresh — keep it
            }
        }
        let xml_path = dir.join(format!("{key}.xml"));
        let meta_path = dir.join(format!("{key}.meta.json"));
        let _ = std::fs::remove_file(&xml_path);
        let _ = std::fs::remove_file(&meta_path);
        deleted += 1;
    }
    Ok(deleted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn with_cache_dir() -> TempDir {
        tempfile::tempdir().expect("tmpdir")
    }

    fn all_on() -> FeatureFilter {
        FeatureFilter::default()
    }

    fn roads_only() -> FeatureFilter {
        FeatureFilter {
            roads: true,
            buildings: false,
            water: false,
            landuse: false,
            railways: false,
        }
    }

    /// Helper: write a meta file with a synthetic created_at timestamp.
    fn write_meta_at(dir: &std::path::Path, key: &str, created_at: DateTime<Utc>) {
        let meta = CacheMeta {
            bbox: [51.5, -0.13, 51.52, -0.10],
            filter: FeatureFilter::default(),
            created_at,
            size_bytes: 100,
        };
        // Also write a dummy .xml so clear() has both files
        std::fs::write(dir.join(format!("{key}.xml")), b"<osm/>").unwrap();
        let meta_path = dir.join(format!("{key}.meta.json"));
        std::fs::write(&meta_path, serde_json::to_string(&meta).unwrap()).unwrap();
    }

    #[test]
    fn cache_key_is_deterministic() {
        let bbox = (51.5, -0.13, 51.52, -0.10);
        let k1 = cache_key(bbox, &all_on());
        let k2 = cache_key(bbox, &all_on());
        assert_eq!(k1, k2);
        assert_eq!(k1.len(), 64, "SHA-256 hex should be 64 chars");
    }

    #[test]
    fn cache_key_snaps_coordinates() {
        // Differ by < 0.00005° (half of 0.0001°) → same key
        let bbox1 = (51.50001, -0.13000, 51.52001, -0.10000);
        let bbox2 = (51.50002, -0.13002, 51.52002, -0.10001);
        assert_eq!(cache_key(bbox1, &all_on()), cache_key(bbox2, &all_on()));
    }

    #[test]
    fn cache_key_varies_by_filter() {
        let bbox = (51.5, -0.13, 51.52, -0.10);
        let all_key = cache_key(bbox, &all_on());
        let roads_key = cache_key(bbox, &roads_only());
        assert_ne!(all_key, roads_key);
    }

    #[test]
    fn write_then_read_roundtrip() {
        let tmp = with_cache_dir();
        let key = "testkey123";
        let xml = "<osm><node id='1'/></osm>";
        let bbox = (51.5_f64, -0.13_f64, 51.52_f64, -0.10_f64);

        write_to(tmp.path(), key, bbox, &FeatureFilter::default(), xml).unwrap();
        let got = read_from(tmp.path(), key);
        assert_eq!(got.as_deref(), Some(xml));
    }

    #[test]
    fn clear_all_removes_both_files() {
        let tmp = with_cache_dir();
        let key = "aabbcc";
        write_to(
            tmp.path(),
            key,
            (51.5, -0.13, 51.52, -0.10),
            &FeatureFilter::default(),
            "<osm/>",
        )
        .unwrap();

        let deleted = clear_dir(tmp.path(), None).unwrap();
        assert_eq!(deleted, 1);
        assert!(!tmp.path().join(format!("{key}.xml")).exists());
        assert!(!tmp.path().join(format!("{key}.meta.json")).exists());
    }

    #[test]
    fn clear_by_age_keeps_fresh_entries() {
        let tmp = with_cache_dir();
        let now = Utc::now();
        let old_key = "oldentry0000000000000000000000000000000000000000000000000000000a";
        let fresh_key = "freshentry000000000000000000000000000000000000000000000000000b";

        write_meta_at(tmp.path(), old_key, now - chrono::Duration::hours(2));
        write_meta_at(tmp.path(), fresh_key, now - chrono::Duration::minutes(30));

        let deleted = clear_dir(tmp.path(), Some(chrono::Duration::hours(1))).unwrap();
        assert_eq!(deleted, 1, "only the 2h-old entry should be deleted");
        assert!(!tmp.path().join(format!("{old_key}.xml")).exists());
        assert!(tmp.path().join(format!("{fresh_key}.xml")).exists());
    }

    #[test]
    fn find_containing_returns_none_when_empty() {
        let tmp = with_cache_dir();
        let result = find_containing_in(tmp.path(), (51.51, -0.12, 51.515, -0.11), &all_on());
        assert!(result.is_none());
    }

    #[test]
    fn find_containing_returns_xml_when_bbox_contained() {
        let tmp = with_cache_dir();
        let large_bbox = (51.5_f64, -0.13_f64, 51.52_f64, -0.10_f64);
        let key = cache_key(large_bbox, &all_on());
        let xml = "<osm><node id='1'/></osm>";
        write_to(tmp.path(), &key, large_bbox, &all_on(), xml).unwrap();

        // Sub-area fully inside the large bbox
        let small_bbox = (51.505, -0.125, 51.515, -0.105);
        let result = find_containing_in(tmp.path(), small_bbox, &all_on());
        assert_eq!(result.as_deref(), Some(xml));
    }

    #[test]
    fn find_containing_returns_none_when_not_contained() {
        let tmp = with_cache_dir();
        let cached_bbox = (51.5_f64, -0.13_f64, 51.52_f64, -0.10_f64);
        let key = cache_key(cached_bbox, &all_on());
        write_to(tmp.path(), &key, cached_bbox, &all_on(), "<osm/>").unwrap();

        // Requested bbox extends outside the cached one
        let outside_bbox = (51.49, -0.13, 51.52, -0.10);
        let result = find_containing_in(tmp.path(), outside_bbox, &all_on());
        assert!(result.is_none());
    }

    #[test]
    fn find_containing_returns_none_on_filter_mismatch() {
        let tmp = with_cache_dir();
        let bbox = (51.5_f64, -0.13_f64, 51.52_f64, -0.10_f64);
        let key = cache_key(bbox, &all_on());
        write_to(tmp.path(), &key, bbox, &all_on(), "<osm/>").unwrap();

        let small_bbox = (51.505, -0.125, 51.515, -0.105);
        let result = find_containing_in(tmp.path(), small_bbox, &roads_only());
        assert!(result.is_none()); // filter mismatch → None
    }
}
