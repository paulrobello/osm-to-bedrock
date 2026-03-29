//! SRTM HGT elevation data loading and bilinear interpolation.
//!
//! Supports SRTM1 (3601×3601, ~30 m resolution) and SRTM3 (1201×1201, ~90 m
//! resolution) tiles.  Each tile is a 1°×1° cell named after its SW corner:
//! `N48W123.hgt` → lat 48–49 °N, lon 122–123 °W.
//!
//! ## Memory-mapped I/O
//!
//! HGT files are memory-mapped rather than read into a `Vec<i16>`.  An
//! SRTM1 tile is ~26 MB; mapping it means the OS pages in only the rows that
//! are actually queried rather than loading the whole file up front.

use std::{collections::HashMap, fs, io, path::Path};

use memmap2::Mmap;

// ── HGT tile ──────────────────────────────────────────────────────────────

/// A single SRTM HGT tile covering a 1°×1° cell.
///
/// Values are big-endian i16 metres; −32 768 is the void sentinel.
/// Row 0 = northernmost row, column 0 = westernmost column.
///
/// The raw file bytes are memory-mapped: the OS loads pages on demand, so
/// only the elevation rows that are actually queried consume physical RAM.
struct HgtTile {
    /// SW-corner latitude (integer degrees, −90 ..= 89).
    lat_sw: i32,
    /// SW-corner longitude (integer degrees, −180 ..= 179).
    lon_sw: i32,
    /// Grid dimension: 1201 (SRTM3) or 3601 (SRTM1).
    size: usize,
    /// Memory-mapped bytes of the HGT file (big-endian i16 pairs).
    mmap: Mmap,
}

impl HgtTile {
    /// Memory-map an HGT file from `path`.
    fn load(path: &Path) -> io::Result<Self> {
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid HGT filename"))?
            .to_ascii_uppercase();

        let (lat_sw, lon_sw) = parse_hgt_filename(&stem)?;

        let file = fs::File::open(path)?;
        // SAFETY: The `Mmap` is live for the lifetime of `HgtTile`, which is
        // owned by `ElevationData`.  The underlying `.hgt` file is:
        //   1. Immutable after it is written — downloads use an atomic
        //      write-then-rename pattern (see `srtm::download_tile`) so the
        //      file is fully written before this mmap can ever open it.
        //   2. Never truncated or deleted while the process holds this mapping;
        //      the SRTM cache is only a write-target, never pruned at runtime.
        // Therefore no other thread can modify the mapped region while it is
        // live, satisfying the safety contract of `Mmap::map`.
        let mmap = unsafe { Mmap::map(&file)? };

        let n_samples = mmap.len() / 2;
        let size = match n_samples {
            1_442_401 => 1201,  // SRTM3 (1201 × 1201)
            12_967_201 => 3601, // SRTM1 (3601 × 3601)
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "unexpected HGT file size: {} bytes ({} samples); expected SRTM1 or SRTM3",
                        mmap.len(),
                        n_samples
                    ),
                ));
            }
        };

        Ok(HgtTile {
            lat_sw,
            lon_sw,
            size,
            mmap,
        })
    }

    /// Return the elevation in metres at (`lat`, `lon`) using bilinear
    /// interpolation.  Returns `None` if the point lies outside this tile or
    /// if all four surrounding samples are void.
    fn elevation_at(&self, lat: f64, lon: f64) -> Option<f64> {
        let lat_sw = self.lat_sw as f64;
        let lon_sw = self.lon_sw as f64;

        // Strict bounds check (allow the tile edges themselves).
        if lat < lat_sw || lat > lat_sw + 1.0 || lon < lon_sw || lon > lon_sw + 1.0 {
            return None;
        }

        let nf = (self.size - 1) as f64;

        // Row 0 = north edge → row increases southward.
        let rf = (lat_sw + 1.0 - lat) * nf;
        // Col 0 = west edge → col increases eastward.
        let cf = (lon - lon_sw) * nf;

        let r0 = rf.floor() as usize;
        let c0 = cf.floor() as usize;
        let r1 = (r0 + 1).min(self.size - 1);
        let c1 = (c0 + 1).min(self.size - 1);
        let dr = rf - r0 as f64;
        let dc = cf - c0 as f64;

        // Gather four surrounding samples; skip void values.
        let h00 = self.sample(r0, c0)?;
        let h01 = self.sample(r0, c1)?;
        let h10 = self.sample(r1, c0)?;
        let h11 = self.sample(r1, c1)?;

        Some(
            (1.0 - dr) * (1.0 - dc) * h00
                + (1.0 - dr) * dc * h01
                + dr * (1.0 - dc) * h10
                + dr * dc * h11,
        )
    }

    /// Return a single elevation sample, or `None` for void (−32 768).
    ///
    /// Reads directly from the memory-mapped bytes with `i16::from_be_bytes`.
    #[inline]
    fn sample(&self, row: usize, col: usize) -> Option<f64> {
        let offset = (row * self.size + col) * 2;
        let b0 = self.mmap[offset];
        let b1 = self.mmap[offset + 1];
        let v = i16::from_be_bytes([b0, b1]);
        if v == -32768 { None } else { Some(v as f64) }
    }
}

// ── Filename parsing ───────────────────────────────────────────────────────

/// Parse an uppercase HGT filename stem (e.g. `"N48W123"`) into
/// `(lat_sw, lon_sw)`.
fn parse_hgt_filename(stem: &str) -> io::Result<(i32, i32)> {
    let bytes = stem.as_bytes();
    if bytes.len() < 7 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("HGT filename stem too short: '{stem}'"),
        ));
    }

    let lat_sign: i32 = match bytes[0] {
        b'N' => 1,
        b'S' => -1,
        c => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("expected N/S as first character, got '{}'", c as char),
            ));
        }
    };

    // Find the E/W separator after position 1.
    let ew_pos = bytes[1..]
        .iter()
        .position(|&b| b == b'E' || b == b'W')
        .map(|p| p + 1)
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "missing E/W in HGT filename")
        })?;

    let lat: i32 = std::str::from_utf8(&bytes[1..ew_pos])
        .ok()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid latitude digits in HGT filename",
            )
        })?;

    let lon_sign: i32 = match bytes[ew_pos] {
        b'E' => 1,
        b'W' => -1,
        c => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("expected E/W, got '{}'", c as char),
            ));
        }
    };

    let lon: i32 = std::str::from_utf8(&bytes[ew_pos + 1..])
        .ok()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid longitude digits in HGT filename",
            )
        })?;

    Ok((lat_sign * lat, lon_sign * lon))
}

// ── ElevationData ──────────────────────────────────────────────────────────

/// A collection of HGT tiles that together cover a geographic region.
///
/// Load with [`ElevationData::from_path`] from a single `.hgt` file or a
/// directory containing multiple `.hgt` files (non-recursive scan).
pub struct ElevationData {
    /// (lat_sw, lon_sw) → tile
    tiles: HashMap<(i32, i32), HgtTile>,
}

impl ElevationData {
    /// Load from a `.hgt` file or a directory of `.hgt` files.
    pub fn from_path(path: &Path) -> anyhow::Result<Self> {
        let mut tiles = HashMap::new();

        if path.is_dir() {
            for entry in fs::read_dir(path)? {
                let entry = entry?;
                let p = entry.path();
                let is_hgt = p
                    .extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| e.eq_ignore_ascii_case("hgt"));
                if !is_hgt {
                    continue;
                }
                match HgtTile::load(&p) {
                    Ok(tile) => {
                        log::info!("Memory-mapped elevation tile: {}", p.display());
                        tiles.insert((tile.lat_sw, tile.lon_sw), tile);
                    }
                    Err(e) => {
                        log::warn!("Skipping {}: {e}", p.display());
                    }
                }
            }
        } else {
            let tile = HgtTile::load(path)?;
            log::info!("Memory-mapped elevation tile: {}", path.display());
            tiles.insert((tile.lat_sw, tile.lon_sw), tile);
        }

        if tiles.is_empty() {
            anyhow::bail!("No valid HGT tiles found at {}", path.display());
        }

        log::info!("Loaded {} elevation tile(s)", tiles.len());
        Ok(Self { tiles })
    }

    /// Return the elevation in metres at (`lat`, `lon`), or `None` if no tile
    /// covers this location or the samples are void.
    pub fn elevation_at(&self, lat: f64, lon: f64) -> Option<f64> {
        let lat_sw = lat.floor() as i32;
        let lon_sw = lon.floor() as i32;
        self.tiles.get(&(lat_sw, lon_sw))?.elevation_at(lat, lon)
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
impl HgtTile {
    /// Create a synthetic tile of arbitrary `size × size` from a flat slice
    /// of i16 elevation values (big-endian, row-major).
    ///
    /// Bypasses the SRTM1/SRTM3 size validation so unit tests can use small
    /// (e.g. 2×2) grids without allocating multi-megabyte fixtures.
    fn synthetic(lat_sw: i32, lon_sw: i32, size: usize, data: &[i16]) -> io::Result<Self> {
        use std::io::Write as _;
        assert_eq!(data.len(), size * size, "data.len() must equal size*size");

        // Write big-endian i16 pairs to a temp file so we can memory-map it.
        let dir = tempfile::tempdir()?;
        let ns = if lat_sw >= 0 { 'N' } else { 'S' };
        let ew = if lon_sw >= 0 { 'E' } else { 'W' };
        let name = format!(
            "{}{:02}{}{:03}.hgt",
            ns,
            lat_sw.unsigned_abs(),
            ew,
            lon_sw.unsigned_abs()
        );
        let path = dir.path().join(&name);
        let mut f = fs::File::create(&path)?;
        for &v in data {
            f.write_all(&v.to_be_bytes())?;
        }
        f.flush()?;
        drop(f);

        let file = fs::File::open(&path)?;
        let mmap = unsafe { Mmap::map(&file)? };

        // Leak the tempdir so the file outlives the mmap.
        std::mem::forget(dir);

        Ok(HgtTile {
            lat_sw,
            lon_sw,
            size,
            mmap,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_north_west() {
        assert_eq!(parse_hgt_filename("N48W123").unwrap(), (48, -123));
    }

    #[test]
    fn parse_south_east() {
        assert_eq!(parse_hgt_filename("S33E151").unwrap(), (-33, 151));
    }

    #[test]
    fn parse_north_east() {
        assert_eq!(parse_hgt_filename("N00E000").unwrap(), (0, 0));
    }

    #[test]
    fn parse_south_west_two_digit_lon() {
        assert_eq!(parse_hgt_filename("S05W072").unwrap(), (-5, -72));
    }

    #[test]
    fn parse_rejects_short_stem() {
        assert!(parse_hgt_filename("N1W1").is_err());
    }

    #[test]
    fn tile_elevation_at_corners() {
        // 2×2 tile: NW=100, NE=200, SW=300, SE=400
        // row 0 = north edge, row 1 = south edge
        let tile = HgtTile::synthetic(0, 0, 2, &[100, 200, 300, 400]).unwrap();

        // Exact NW corner (lat=1, lon=0 → row=0, col=0)
        let nw = tile.elevation_at(1.0, 0.0).unwrap();
        assert!((nw - 100.0).abs() < 0.01, "NW corner: {nw}");

        // Exact NE corner (lat=1, lon=1 → row=0, col=1)
        let ne = tile.elevation_at(1.0, 1.0).unwrap();
        assert!((ne - 200.0).abs() < 0.01, "NE corner: {ne}");

        // Exact SW corner (lat=0, lon=0 → row=1, col=0)
        let sw = tile.elevation_at(0.0, 0.0).unwrap();
        assert!((sw - 300.0).abs() < 0.01, "SW corner: {sw}");

        // Exact SE corner (lat=0, lon=1 → row=1, col=1)
        let se = tile.elevation_at(0.0, 1.0).unwrap();
        assert!((se - 400.0).abs() < 0.01, "SE corner: {se}");
    }

    #[test]
    fn tile_elevation_centre_bilinear() {
        // Centre of a 2×2 tile → average of all four = (100+200+300+400)/4 = 250
        let tile = HgtTile::synthetic(0, 0, 2, &[100, 200, 300, 400]).unwrap();
        let mid = tile.elevation_at(0.5, 0.5).unwrap();
        assert!((mid - 250.0).abs() < 0.01, "centre bilinear: {mid}");
    }

    #[test]
    fn tile_elevation_out_of_bounds() {
        let tile = HgtTile::synthetic(0, 0, 2, &[100, 100, 100, 100]).unwrap();
        assert!(tile.elevation_at(-0.1, 0.5).is_none());
        assert!(tile.elevation_at(1.1, 0.5).is_none());
        assert!(tile.elevation_at(0.5, -0.1).is_none());
        assert!(tile.elevation_at(0.5, 1.1).is_none());
    }

    #[test]
    fn tile_void_returns_none() {
        let tile = HgtTile::synthetic(0, 0, 2, &[-32768, -32768, -32768, -32768]).unwrap();
        assert!(tile.elevation_at(0.5, 0.5).is_none());
    }
}
