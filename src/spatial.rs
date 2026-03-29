//! Spatial data structures used by the conversion pipeline.
//!
//! Provides:
//! - [`SpatialIndex`] — type-bucketed + grid-indexed way lookup
//! - [`HeightMap`] — per-column surface-Y cache
//! - [`ResolvedRelation`] — multipolygon relation with block-coordinate rings
//! - [`compute_surface_y`] — elevation-aware surface Y for a single column
//! - [`TILE_CHUNKS`] — streaming tile size constant

use crate::{bedrock, convert, elevation, osm};
use std::collections::{HashMap, HashSet};

/// Number of chunks per dimension in one streaming tile.
///
/// 64 chunks × 16 blocks = 1024 blocks per dimension.  Each tile is processed
/// independently during streaming conversion: terrain + features are written
/// and cleared before moving to the next tile, keeping peak chunk memory
/// proportional to `TILE_CHUNKS²` rather than the whole map.
pub const TILE_CHUNKS: i32 = 64;

/// Type-bucketed and grid-indexed lookup structure built once from `resolved_ways`.
///
/// Type buckets give O(way-type) iteration instead of O(all-ways) per render pass.
/// The 64-block grid enables per-chunk queries for future parallel rendering.
pub struct SpatialIndex {
    /// 64-block grid cells → way indices for bounding-box queries.
    pub grid: HashMap<(i32, i32), Vec<usize>>,
    // ── Type buckets ─────────────────────────────────────────────────────
    pub highways: Vec<usize>,
    pub buildings: Vec<usize>,
    pub landuse: Vec<usize>,
    pub waterways: Vec<usize>,
    pub railways: Vec<usize>,
    pub barriers: Vec<usize>,
    /// Ways with amenity / shop / tourism / leisure tags.
    pub pois: Vec<usize>,
    /// Ways with addr:housenumber (building address signs).
    pub address: Vec<usize>,
}

impl SpatialIndex {
    const CELL: i32 = 64;

    pub fn build(resolved_ways: &[(&osm::OsmWay, Vec<(i32, i32)>)]) -> Self {
        let mut idx = SpatialIndex {
            grid: HashMap::new(),
            highways: Vec::new(),
            buildings: Vec::new(),
            landuse: Vec::new(),
            waterways: Vec::new(),
            railways: Vec::new(),
            barriers: Vec::new(),
            pois: Vec::new(),
            address: Vec::new(),
        };

        for (i, (way, pts)) in resolved_ways.iter().enumerate() {
            let t = &way.tags;
            if t.contains_key("highway") {
                idx.highways.push(i);
            }
            if t.contains_key("building") || t.contains_key("building:part") {
                idx.buildings.push(i);
            }
            if t.contains_key("landuse") || t.contains_key("natural") {
                idx.landuse.push(i);
            }
            if t.contains_key("waterway")
                || t.get("natural").is_some_and(|v| v == "water")
                || t.get("landuse")
                    .is_some_and(|v| matches!(v.as_str(), "reservoir" | "water" | "basin"))
            {
                idx.waterways.push(i);
            }
            if t.get("railway").is_some_and(|v| v == "rail") {
                idx.railways.push(i);
            }
            if t.contains_key("barrier") {
                idx.barriers.push(i);
            }
            if t.contains_key("amenity")
                || t.contains_key("shop")
                || t.contains_key("tourism")
                || t.contains_key("leisure")
            {
                idx.pois.push(i);
            }
            if t.contains_key("addr:housenumber") {
                idx.address.push(i);
            }

            // Spatial grid
            if pts.is_empty() {
                continue;
            }
            let min_x = pts.iter().map(|p| p.0).min().unwrap();
            let max_x = pts.iter().map(|p| p.0).max().unwrap();
            let min_z = pts.iter().map(|p| p.1).min().unwrap();
            let max_z = pts.iter().map(|p| p.1).max().unwrap();
            let c0x = min_x.div_euclid(Self::CELL);
            let c1x = max_x.div_euclid(Self::CELL);
            let c0z = min_z.div_euclid(Self::CELL);
            let c1z = max_z.div_euclid(Self::CELL);
            for cx in c0x..=c1x {
                for cz in c0z..=c1z {
                    idx.grid.entry((cx, cz)).or_default().push(i);
                }
            }
        }
        idx
    }

    /// Return way indices whose bounding box overlaps the given block rectangle.
    pub fn query_rect(&self, min_x: i32, min_z: i32, max_x: i32, max_z: i32) -> Vec<usize> {
        let c0x = min_x.div_euclid(Self::CELL);
        let c1x = max_x.div_euclid(Self::CELL);
        let c0z = min_z.div_euclid(Self::CELL);
        let c1z = max_z.div_euclid(Self::CELL);
        let mut seen = HashSet::new();
        let mut result = Vec::new();
        for cx in c0x..=c1x {
            for cz in c0z..=c1z {
                if let Some(indices) = self.grid.get(&(cx, cz)) {
                    for &wi in indices {
                        if seen.insert(wi) {
                            result.push(wi);
                        }
                    }
                }
            }
        }
        result
    }
}

/// A multipolygon relation with its rings resolved to block coordinates.
pub struct ResolvedRelation<'a> {
    pub tags: &'a HashMap<String, String>,
    pub outers: Vec<Vec<(i32, i32)>>,
    pub inners: Vec<Vec<(i32, i32)>>,
}

/// Per-column surface-Y lookup built during terrain generation.
///
/// Stores one `i32` per (block_x, block_z) column.  When the world bounds are
/// known at construction time, a flat `Vec<i32>` with offset arithmetic is used
/// (4× lower memory and better cache locality than a `HashMap`).  When bounds
/// are unknown (preview path), it falls back to a `HashMap`.
///
/// Falls back to `default` (sea level) for positions outside the bounds.
pub struct HeightMap {
    /// Flat storage used when bounds are known (`Some`).
    data_vec: Option<HeightMapVec>,
    /// Sparse fallback used when bounds are unknown.
    data_map: HashMap<(i32, i32), i32>,
    pub default: i32,
}

struct HeightMapVec {
    data: Vec<i32>,
    origin_x: i32,
    origin_z: i32,
    width: usize, // number of columns in X
    depth: usize, // number of columns in Z
}

impl HeightMapVec {
    fn new(min_bx: i32, min_bz: i32, max_bx: i32, max_bz: i32, default: i32) -> Self {
        let width = ((max_bx - min_bx) as usize).saturating_add(1);
        let depth = ((max_bz - min_bz) as usize).saturating_add(1);
        Self {
            data: vec![default; width * depth],
            origin_x: min_bx,
            origin_z: min_bz,
            width,
            depth,
        }
    }

    #[inline]
    fn index(&self, bx: i32, bz: i32) -> Option<usize> {
        let ix = (bx - self.origin_x) as usize;
        let iz = (bz - self.origin_z) as usize;
        if ix < self.width && iz < self.depth {
            Some(ix * self.depth + iz)
        } else {
            None
        }
    }
}

impl HeightMap {
    /// Create an unbounded `HeightMap` backed by a `HashMap` (preview path).
    pub fn new(default: i32) -> Self {
        Self {
            data_vec: None,
            data_map: HashMap::new(),
            default,
        }
    }

    /// Create a bounded `HeightMap` backed by a flat `Vec` (streaming path).
    ///
    /// All block columns in `[min_bx..=max_bx] × [min_bz..=max_bz]` are
    /// pre-allocated and initialised to `default`.
    pub fn with_bounds(min_bx: i32, min_bz: i32, max_bx: i32, max_bz: i32, default: i32) -> Self {
        Self {
            data_vec: Some(HeightMapVec::new(min_bx, min_bz, max_bx, max_bz, default)),
            data_map: HashMap::new(),
            default,
        }
    }

    #[inline]
    pub fn get(&self, bx: i32, bz: i32) -> i32 {
        if let Some(ref v) = self.data_vec {
            if let Some(idx) = v.index(bx, bz) {
                return v.data[idx];
            }
            return self.default;
        }
        *self.data_map.get(&(bx, bz)).unwrap_or(&self.default)
    }

    #[inline]
    pub fn insert(&mut self, bx: i32, bz: i32, y: i32) {
        if let Some(ref mut v) = self.data_vec
            && let Some(idx) = v.index(bx, bz)
        {
            v.data[idx] = y;
            return;
            // Out-of-bounds insert on vec path; fall through to map (shouldn't happen in normal use).
        }
        self.data_map.insert((bx, bz), y);
    }

    /// Apply a median filter with the given `radius` to smooth elevation jitter.
    ///
    /// Radius 0 is a no-op.  Radius 1 uses a 3×3 kernel, radius 2 uses 5×5, etc.
    /// The filter is double-buffered: reads from the current data, writes to a
    /// fresh buffer, then swaps — so no read-after-write artifacts.
    pub fn smooth(&mut self, radius: i32) {
        if radius <= 0 {
            return;
        }
        let r = radius;

        if let Some(ref v) = self.data_vec {
            // Vec-backed path: iterate every cell in the flat buffer.
            let width = v.width as i32;
            let depth = v.depth as i32;
            let mut smoothed = v.data.clone();

            for ix in 0..width {
                for iz in 0..depth {
                    let mut neighbours = Vec::with_capacity(((2 * r + 1) * (2 * r + 1)) as usize);
                    for dx in -r..=r {
                        for dz in -r..=r {
                            let nx = ix + dx;
                            let nz = iz + dz;
                            if nx >= 0 && nx < width && nz >= 0 && nz < depth {
                                neighbours.push(v.data[(nx as usize) * v.depth + (nz as usize)]);
                            }
                        }
                    }
                    neighbours.sort_unstable();
                    smoothed[(ix as usize) * v.depth + (iz as usize)] =
                        neighbours[neighbours.len() / 2];
                }
            }

            self.data_vec.as_mut().unwrap().data = smoothed;
        } else {
            // HashMap-backed path: collect all keys, compute median for each.
            let keys: Vec<(i32, i32)> = self.data_map.keys().copied().collect();
            let mut smoothed = HashMap::with_capacity(keys.len());

            for &(bx, bz) in &keys {
                let mut neighbours = Vec::with_capacity(((2 * r + 1) * (2 * r + 1)) as usize);
                for dx in -r..=r {
                    for dz in -r..=r {
                        if let Some(&h) = self.data_map.get(&(bx + dx, bz + dz)) {
                            neighbours.push(h);
                        }
                    }
                }
                neighbours.sort_unstable();
                smoothed.insert((bx, bz), neighbours[neighbours.len() / 2]);
            }

            self.data_map = smoothed;
        }
    }
}

/// Compute the surface Y (ground level) for a single block column.
///
/// If elevation data is provided and covers the location, returns
/// `sea_level + round(elevation_metres × vertical_scale)`, clamped to the
/// valid Y range.  Otherwise falls back to `sea_level`.
#[inline]
pub fn compute_surface_y(
    bx: i32,
    bz: i32,
    elevation: &Option<elevation::ElevationData>,
    conv: &convert::CoordConverter,
    sea_level: i32,
    vertical_scale: f64,
) -> i32 {
    if let Some(elev) = elevation {
        let (lat, lon) = conv.to_lat_lon(bx, bz);
        if let Some(elev_m) = elev.elevation_at(lat, lon) {
            return ((sea_level as f64 + elev_m * vertical_scale).round() as i32)
                .clamp(bedrock::MIN_Y + 2, bedrock::MAX_Y - 2);
        }
    }
    sea_level
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smooth_radius_zero_is_noop() {
        let mut hm = HeightMap::with_bounds(0, 0, 4, 4, 65);
        hm.insert(2, 2, 70);
        hm.smooth(0);
        assert_eq!(hm.get(2, 2), 70);
    }

    #[test]
    fn smooth_radius_one_removes_isolated_bump() {
        // 5x5 grid all at 65, with one block at 66
        let mut hm = HeightMap::with_bounds(0, 0, 4, 4, 65);
        for x in 0..=4 {
            for z in 0..=4 {
                hm.insert(x, z, 65);
            }
        }
        hm.insert(2, 2, 66); // isolated 1-block bump

        hm.smooth(1);

        // The bump should be smoothed away — median of 9 values where 8 are 65
        // and 1 is 66 → median is 65
        assert_eq!(hm.get(2, 2), 65);
    }

    #[test]
    fn smooth_preserves_plateau() {
        // 5x5 grid: left half at 65, right half at 70
        let mut hm = HeightMap::with_bounds(0, 0, 4, 4, 65);
        for x in 0..=4 {
            for z in 0..=4 {
                let val = if x <= 2 { 65 } else { 70 };
                hm.insert(x, z, val);
            }
        }

        hm.smooth(1);

        // Interior points should keep their plateau value
        assert_eq!(hm.get(0, 2), 65);
        assert_eq!(hm.get(4, 2), 70);
    }

    #[test]
    fn smooth_hashmap_path() {
        // Unbounded HeightMap (HashMap path)
        let mut hm = HeightMap::new(65);
        for x in 0..5 {
            for z in 0..5 {
                hm.insert(x, z, 65);
            }
        }
        hm.insert(2, 2, 66);

        hm.smooth(1);

        assert_eq!(hm.get(2, 2), 65);
    }
}
