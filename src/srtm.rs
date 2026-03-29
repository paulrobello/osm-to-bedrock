//! Auto-download of SRTM elevation tiles.
//!
//! Fetches 1 arc-second HGT tiles from the AWS Terrain Tiles bucket
//! (Mapzen/Tilezen open data, no authentication required):
//!
//! ```text
//! https://s3.amazonaws.com/elevation-tiles-prod/skadi/{dir}/{name}.hgt.gz
//! ```
//!
//! Each tile is a gzip-compressed SRTM1 HGT file (3601 × 3601 × 2 bytes)
//! covering one 1°×1° cell, named after its SW corner: `N48W123.hgt.gz`.

use anyhow::Result;
use std::io::Read;
use std::path::{Path, PathBuf};

const BASE_URL: &str = "https://s3.amazonaws.com/elevation-tiles-prod/skadi";

// ── Cache directory ─────────────────────────────────────────────────────────

/// Return the persistent SRTM tile cache directory, creating it if needed.
///
/// Priority:
/// 1. `SRTM_CACHE_DIR` environment variable (override for servers / CI)
/// 2. `$HOME/.cache/osm-to-bedrock/srtm` (Linux / macOS XDG-style)
/// 3. `%LOCALAPPDATA%\osm-to-bedrock\srtm` (Windows)
/// 4. `<system-temp>/osm-to-bedrock-srtm` (fallback)
pub fn cache_dir() -> PathBuf {
    let dir = if let Ok(v) = std::env::var("SRTM_CACHE_DIR") {
        PathBuf::from(v)
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home)
            .join(".cache")
            .join("osm-to-bedrock")
            .join("srtm")
    } else if let Ok(local) = std::env::var("LOCALAPPDATA") {
        PathBuf::from(local).join("osm-to-bedrock").join("srtm")
    } else {
        std::env::temp_dir().join("osm-to-bedrock-srtm")
    };

    if let Err(e) = std::fs::create_dir_all(&dir) {
        log::warn!("Could not create SRTM cache dir {}: {e}", dir.display());
    }
    dir
}

// ── Tile utilities ─────────────────────────────────────────────────────────

/// Return all 1°×1° tile SW corners needed to cover (`min_lat`, `min_lon`) –
/// (`max_lat`, `max_lon`).
///
/// Each entry is `(lat_sw, lon_sw)` as signed integer degrees.
pub fn tiles_for_bbox(min_lat: f64, min_lon: f64, max_lat: f64, max_lon: f64) -> Vec<(i32, i32)> {
    let lat0 = min_lat.floor() as i32;
    let lat1 = max_lat.ceil() as i32;
    let lon0 = min_lon.floor() as i32;
    let lon1 = max_lon.ceil() as i32;

    let mut tiles = Vec::new();
    for lat in lat0..lat1 {
        for lon in lon0..lon1 {
            tiles.push((lat, lon));
        }
    }
    tiles
}

/// Format a tile SW corner `(lat_sw, lon_sw)` as the standard HGT filename
/// stem (without extension): e.g. `(48, -123)` → `"N48W123"`.
pub fn tile_name(lat: i32, lon: i32) -> String {
    let ns = if lat >= 0 { 'N' } else { 'S' };
    let ew = if lon >= 0 { 'E' } else { 'W' };
    format!("{ns}{:02}{ew}{:03}", lat.unsigned_abs(), lon.unsigned_abs())
}

// ── Download ───────────────────────────────────────────────────────────────

/// Download, decompress, and save a single SRTM tile to `dest_dir`.
///
/// Skips the download if the `.hgt` file already exists.
/// Returns `Ok(true)` if the tile was downloaded, `Ok(false)` if it already existed.
pub fn download_tile(lat: i32, lon: i32, dest_dir: &Path) -> Result<bool> {
    let name = tile_name(lat, lon);
    let hgt_path = dest_dir.join(format!("{name}.hgt"));

    if hgt_path.exists() {
        log::debug!("Elevation tile {name} already exists — skipping");
        return Ok(false);
    }

    // Build the directory component: e.g. "N48" or "S05"
    let ns = if lat >= 0 { 'N' } else { 'S' };
    let dir_part = format!("{ns}{:02}", lat.unsigned_abs());
    let url = format!("{BASE_URL}/{dir_part}/{name}.hgt.gz");

    log::info!("Downloading elevation tile {name}…");

    // SEC-007: use an explicit timeout to prevent indefinitely blocking the
    // Tokio thread pool on stalled S3 connections.
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| anyhow::anyhow!("Failed to build HTTP client: {e}"))?;

    let response = client
        .get(&url)
        .send()
        .map_err(|e| anyhow::anyhow!("Request failed for {url}: {e}"))?;

    if !response.status().is_success() {
        anyhow::bail!("HTTP {} downloading {}", response.status(), url);
    }

    let gz_bytes = response
        .bytes()
        .map_err(|e| anyhow::anyhow!("Failed to read response body for {name}: {e}"))?;

    let mut decoder = flate2::read::GzDecoder::new(gz_bytes.as_ref());
    let mut hgt_data = Vec::new();
    decoder
        .read_to_end(&mut hgt_data)
        .map_err(|e| anyhow::anyhow!("Gzip decompression failed for {name}: {e}"))?;

    // SEC-008: write to a temporary file first, then atomically rename into
    // place.  This prevents a concurrent request from memory-mapping a
    // partially-written file, and eliminates the TOCTOU window between the
    // existence check above and the final write.
    let tmp_path = hgt_path.with_extension("hgt.tmp");
    std::fs::write(&tmp_path, &hgt_data)
        .map_err(|e| anyhow::anyhow!("Failed to write tmp file {}: {e}", tmp_path.display()))?;
    std::fs::rename(&tmp_path, &hgt_path).map_err(|e| {
        anyhow::anyhow!(
            "Failed to rename {} → {}: {e}",
            tmp_path.display(),
            hgt_path.display()
        )
    })?;

    log::info!(
        "Saved elevation tile {} ({:.1} MB)",
        name,
        hgt_data.len() as f64 / 1_048_576.0
    );
    Ok(true)
}

/// Download a single tile, retrying up to `max_retries` times on failure.
///
/// Returns `Ok(true)` if downloaded, `Ok(false)` if already cached, or an
/// error describing all attempts if every try failed.
fn download_tile_with_retry(lat: i32, lon: i32, dest_dir: &Path, max_retries: u32) -> Result<bool> {
    let name = tile_name(lat, lon);
    let mut last_err = anyhow::anyhow!("no attempts made");
    for attempt in 1..=max_retries {
        match download_tile(lat, lon, dest_dir) {
            Ok(v) => return Ok(v),
            Err(e) => {
                last_err = e;
                if attempt < max_retries {
                    let delay = std::time::Duration::from_secs(2u64.pow(attempt - 1));
                    log::warn!(
                        "Elevation tile {name} attempt {attempt}/{max_retries} failed: {last_err} — retrying in {}s",
                        delay.as_secs()
                    );
                    std::thread::sleep(delay);
                }
            }
        }
    }
    Err(last_err.context(format!(
        "elevation tile {name} failed after {max_retries} attempts"
    )))
}

/// Download all SRTM tiles needed to cover the given bounding box into
/// `dest_dir`.
///
/// `progress_cb` is called before each tile with `(tile_index, total_tiles,
/// tile_name)` so callers can report progress.
///
/// Each tile is retried up to 3 times with exponential backoff (1 s, 2 s).
/// If any tile fails all retries the function returns an error — the caller
/// should abort the conversion rather than silently produce flat terrain.
///
/// Returns the number of tiles actually downloaded (excludes pre-existing ones).
pub fn download_tiles_for_bbox(
    min_lat: f64,
    min_lon: f64,
    max_lat: f64,
    max_lon: f64,
    dest_dir: &Path,
    progress_cb: &dyn Fn(usize, usize, &str),
) -> Result<usize> {
    let tiles = tiles_for_bbox(min_lat, min_lon, max_lat, max_lon);
    let total = tiles.len();

    if total == 0 {
        log::warn!("No SRTM tiles computed for bbox — bbox may be empty");
        return Ok(0);
    }

    log::info!("Downloading {total} SRTM tile(s) for bounding box");

    let mut downloaded = 0usize;
    let mut failed: Vec<String> = Vec::new();

    for (i, (lat, lon)) in tiles.iter().enumerate() {
        let name = tile_name(*lat, *lon);
        progress_cb(i, total, &name);
        match download_tile_with_retry(*lat, *lon, dest_dir, 3) {
            Ok(true) => downloaded += 1,
            Ok(false) => {}
            Err(e) => {
                log::error!("Elevation tile {name} could not be downloaded: {e:#}");
                failed.push(name);
            }
        }
    }

    if !failed.is_empty() {
        anyhow::bail!(
            "Failed to download {} elevation tile(s): {}. \
             Cannot generate terrain without complete elevation data.",
            failed.len(),
            failed.join(", ")
        );
    }

    log::info!("Elevation tiles ready ({downloaded} new, {total} total)");
    Ok(downloaded)
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_name_north_west() {
        assert_eq!(tile_name(48, -123), "N48W123");
    }

    #[test]
    fn tile_name_south_east() {
        assert_eq!(tile_name(-33, 151), "S33E151");
    }

    #[test]
    fn tile_name_equator_prime_meridian() {
        assert_eq!(tile_name(0, 0), "N00E000");
    }

    #[test]
    fn tiles_for_bbox_single_tile() {
        // A small bbox well within one degree cell
        let tiles = tiles_for_bbox(48.1, -122.9, 48.8, -122.1);
        assert_eq!(tiles.len(), 1);
        assert!(tiles.contains(&(48, -123)));
    }

    #[test]
    fn tiles_for_bbox_two_columns() {
        // Spans the lon=-123 boundary
        let tiles = tiles_for_bbox(48.1, -123.5, 48.8, -122.5);
        assert_eq!(tiles.len(), 2);
        assert!(tiles.contains(&(48, -124)));
        assert!(tiles.contains(&(48, -123)));
    }

    #[test]
    fn tiles_for_bbox_four_tiles() {
        // Spans both a lat and a lon boundary
        let tiles = tiles_for_bbox(47.5, -123.5, 48.5, -122.5);
        assert_eq!(tiles.len(), 4);
    }

    #[test]
    fn tiles_for_empty_bbox() {
        let tiles = tiles_for_bbox(0.0, 0.0, 0.0, 0.0);
        assert!(tiles.is_empty());
    }
}
