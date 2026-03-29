//! Geographic coordinate conversion and rasterization utilities.

/// Approximate metres per degree of latitude (constant worldwide).
const METRES_PER_DEG_LAT: f64 = 111_320.0;

/// Converts OSM geographic coordinates to Minecraft block coordinates.
///
/// * East  → +X
/// * North → −Z  (Minecraft's north is −Z)
/// * Scale: `metres_per_block` controls map zoom
pub struct CoordConverter {
    pub origin_lat: f64,
    pub origin_lon: f64,
    pub metres_per_block: f64,
}

impl CoordConverter {
    pub fn new(origin_lat: f64, origin_lon: f64, metres_per_block: f64) -> Self {
        Self {
            origin_lat,
            origin_lon,
            metres_per_block,
        }
    }

    /// Convert (lat, lon) to (block_x, block_z).
    pub fn to_block_xz(&self, lat: f64, lon: f64) -> (i32, i32) {
        let metres_per_deg_lon = METRES_PER_DEG_LAT * self.origin_lat.to_radians().cos();
        let dx = (lon - self.origin_lon) * metres_per_deg_lon / self.metres_per_block;
        let dz = -(lat - self.origin_lat) * METRES_PER_DEG_LAT / self.metres_per_block;
        (dx.round() as i32, dz.round() as i32)
    }

    /// Convert (block_x, block_z) back to (lat, lon).
    ///
    /// Inverse of [`to_block_xz`].  Used by elevation sampling to map each
    /// block column back to a geographic coordinate.
    pub fn to_lat_lon(&self, bx: i32, bz: i32) -> (f64, f64) {
        let metres_per_deg_lon = METRES_PER_DEG_LAT * self.origin_lat.to_radians().cos();
        let lon = self.origin_lon + (bx as f64 * self.metres_per_block) / metres_per_deg_lon;
        let lat = self.origin_lat - (bz as f64 * self.metres_per_block) / METRES_PER_DEG_LAT;
        (lat, lon)
    }

    /// Return the (chunk_x, chunk_z) that contains a block coordinate.
    #[allow(dead_code)]
    pub fn block_to_chunk(x: i32, z: i32) -> (i32, i32) {
        (x.div_euclid(16), z.div_euclid(16))
    }

    /// Return local (0..16) coordinates within a chunk.
    #[allow(dead_code)]
    pub fn local_in_chunk(x: i32, z: i32) -> (i32, i32) {
        (x.rem_euclid(16), z.rem_euclid(16))
    }
}

// ── Rasterization ─────────────────────────────────────────────────────────

/// Rasterise a line segment using Bresenham's algorithm.
/// Returns every (x, z) block on the line.
pub fn rasterize_line(x0: i32, z0: i32, x1: i32, z1: i32) -> Vec<(i32, i32)> {
    let mut points = Vec::new();
    let dx = (x1 - x0).abs();
    let dz = (z1 - z0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let sz = if z0 < z1 { 1 } else { -1 };
    let mut err = dx - dz;
    let mut x = x0;
    let mut z = z0;

    loop {
        points.push((x, z));
        if x == x1 && z == z1 {
            break;
        }
        let e2 = 2 * err;
        if e2 > -dz {
            err -= dz;
            x += sx;
        }
        if e2 < dx {
            err += dx;
            z += sz;
        }
    }
    points
}

/// Rasterise a closed polygon using a scanline fill.
/// `pts` is a slice of (x, z) block coordinates forming the polygon boundary.
/// Returns all filled (x, z) positions inside (and including the boundary).
pub fn rasterize_polygon(pts: &[(i32, i32)]) -> Vec<(i32, i32)> {
    if pts.len() < 3 {
        return pts.to_vec();
    }

    let min_z = pts.iter().map(|&(_, z)| z).min().unwrap();
    let max_z = pts.iter().map(|&(_, z)| z).max().unwrap();

    let mut filled = Vec::new();
    let n = pts.len();

    for scan_z in min_z..=max_z {
        let mut xs: Vec<i32> = Vec::new();

        for i in 0..n {
            let j = (i + 1) % n;
            let (x0, z0) = pts[i];
            let (x1, z1) = pts[j];

            // Edge must cross the scanline
            if (z0 <= scan_z && z1 > scan_z) || (z1 <= scan_z && z0 > scan_z) {
                let dz = z1 - z0;
                let x_intersect = x0 + (scan_z - z0) * (x1 - x0) / dz;
                xs.push(x_intersect);
            }
        }

        xs.sort_unstable();

        let mut i = 0;
        while i + 1 < xs.len() {
            for x in xs[i]..=xs[i + 1] {
                filled.push((x, scan_z));
            }
            i += 2;
        }
    }

    filled
}

/// Rasterise a polygon with holes using scanline fill.
/// `outer` is the outer ring, `holes` is a slice of inner rings to subtract.
/// Returns all filled (x, z) positions inside the outer ring but outside any hole.
pub fn rasterize_polygon_with_holes(
    outer: &[(i32, i32)],
    holes: &[Vec<(i32, i32)>],
) -> Vec<(i32, i32)> {
    if holes.is_empty() {
        return rasterize_polygon(outer);
    }

    // Rasterize the outer ring
    let filled = rasterize_polygon(outer);

    // Collect all hole interior points into a HashSet for fast lookup
    let mut hole_set = std::collections::HashSet::new();
    for hole in holes {
        if hole.len() >= 3 {
            for pt in rasterize_polygon(hole) {
                hole_set.insert(pt);
            }
        }
    }

    // Subtract holes
    filled
        .into_iter()
        .filter(|pt| !hole_set.contains(pt))
        .collect()
}

// ── Polygon Processing ────────────────────────────────────────────────────────

/// Snap nearly-axis-aligned building wall edges to exact cardinal directions.
///
/// For each polygon edge `(v[i], v[(i+1)%n])`:
/// - If `|dx| <= threshold` **and** `|dz| > threshold`: snap both endpoints' x
///   to `(x0 + x1) / 2` (nearly-vertical wall).
/// - If `|dz| <= threshold` **and** `|dx| > threshold`: snap both endpoints' z
///   to `(z0 + z1) / 2` (nearly-horizontal wall).
///
/// Consecutive identical vertices produced by snapping are removed.
/// A `threshold` of `0` returns the polygon unchanged.
pub fn straighten_polygon(pts: &[(i32, i32)], threshold: i32) -> Vec<(i32, i32)> {
    if threshold == 0 || pts.len() < 3 {
        return pts.to_vec();
    }
    let n = pts.len();
    let mut out = pts.to_vec();
    for i in 0..n {
        let j = (i + 1) % n;
        let dx = (out[j].0 - out[i].0).abs();
        let dz = (out[j].1 - out[i].1).abs();
        if dx <= threshold && dz > threshold {
            let avg_x = (out[i].0 + out[j].0) / 2;
            out[i].0 = avg_x;
            out[j].0 = avg_x;
        } else if dz <= threshold && dx > threshold {
            let avg_z = (out[i].1 + out[j].1) / 2;
            out[i].1 = avg_z;
            out[j].1 = avg_z;
        }
    }
    out.dedup();
    if out.len() > 1 && out.first() == out.last() {
        out.pop();
    }
    out
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_maps_to_zero() {
        let c = CoordConverter::new(51.5, -0.12, 1.0);
        assert_eq!(c.to_block_xz(51.5, -0.12), (0, 0));
    }

    #[test]
    fn east_is_positive_x() {
        let c = CoordConverter::new(0.0, 0.0, 1.0);
        let (x, _) = c.to_block_xz(0.0, 0.001); // slightly east
        assert!(x > 0, "east should be +x, got {x}");
    }

    #[test]
    fn north_is_negative_z() {
        let c = CoordConverter::new(0.0, 0.0, 1.0);
        let (_, z) = c.to_block_xz(0.001, 0.0); // slightly north
        assert!(z < 0, "north should be -z, got {z}");
    }

    #[test]
    fn block_to_chunk_positive() {
        assert_eq!(CoordConverter::block_to_chunk(0, 0), (0, 0));
        assert_eq!(CoordConverter::block_to_chunk(15, 15), (0, 0));
        assert_eq!(CoordConverter::block_to_chunk(16, 16), (1, 1));
        assert_eq!(CoordConverter::block_to_chunk(31, 31), (1, 1));
    }

    #[test]
    fn block_to_chunk_negative() {
        assert_eq!(CoordConverter::block_to_chunk(-1, -1), (-1, -1));
        assert_eq!(CoordConverter::block_to_chunk(-16, -16), (-1, -1));
        assert_eq!(CoordConverter::block_to_chunk(-17, -17), (-2, -2));
    }

    #[test]
    fn line_endpoints_included() {
        let pts = rasterize_line(0, 0, 5, 5);
        assert!(pts.contains(&(0, 0)));
        assert!(pts.contains(&(5, 5)));
    }

    #[test]
    fn to_lat_lon_round_trips() {
        let c = CoordConverter::new(51.5, -0.12, 1.0);
        let (lat, lon) = c.to_lat_lon(0, 0);
        assert!((lat - 51.5).abs() < 1e-9, "lat round-trip: {lat}");
        assert!((lon - -0.12).abs() < 1e-9, "lon round-trip: {lon}");

        // A block offset should invert correctly.
        let (bx, bz) = c.to_block_xz(51.501, -0.115);
        let (lat2, lon2) = c.to_lat_lon(bx, bz);
        // Block rounding limits precision to ~1 metre, so 0.0001° ≈ 11 m is safe.
        assert!((lat2 - 51.501).abs() < 0.0001, "lat invert: {lat2}");
        assert!((lon2 - -0.115).abs() < 0.0001, "lon invert: {lon2}");
    }

    #[test]
    fn polygon_fills_square() {
        let pts = vec![(0, 0), (4, 0), (4, 4), (0, 4)];
        let filled = rasterize_polygon(&pts);
        assert!(filled.contains(&(2, 2)));
        assert!(!filled.is_empty());
    }

    #[test]
    fn straighten_noop_when_threshold_zero() {
        let pts = vec![(0, 0), (1, 0), (1, 4), (0, 4)];
        assert_eq!(straighten_polygon(&pts, 0), pts);
    }

    #[test]
    fn straighten_x_stagger_snapped() {
        // Wall from (0,0)→(1,5): |dx|=1 ≤ threshold=1 and |dz|=5 > 1 → snap x to avg=0
        let pts = vec![(0, 0), (1, 5), (1, 10), (0, 10)];
        let result = straighten_polygon(&pts, 1);
        // After snap the first edge has equal x on both ends
        let x0 = result[0].0;
        let x1 = result[1].0;
        assert_eq!(x0, x1, "x stagger should be snapped: got {x0} vs {x1}");
    }

    #[test]
    fn straighten_z_stagger_snapped() {
        // Wall from (0,0)→(5,1): |dz|=1 ≤ threshold=1 and |dx|=5 > 1 → snap z to avg=0
        let pts = vec![(0, 0), (5, 1), (5, 5), (0, 5)];
        let result = straighten_polygon(&pts, 1);
        let z0 = result[0].1;
        let z1 = result[1].1;
        assert_eq!(z0, z1, "z stagger should be snapped: got {z0} vs {z1}");
    }

    #[test]
    fn straighten_large_stagger_not_snapped_at_threshold_1() {
        // |dx|=2 > threshold=1, |dz|=5 > threshold=1 → not snapped
        let pts = vec![(0, 0), (2, 5), (2, 10), (0, 10)];
        assert_eq!(straighten_polygon(&pts, 1), pts);
    }

    #[test]
    fn straighten_large_stagger_snapped_at_threshold_2() {
        let pts = vec![(0, 0), (2, 5), (2, 10), (0, 10)];
        let result = straighten_polygon(&pts, 2);
        // Single-pass snapping: edge 0→1 snaps x to 1, edge 3→0 snaps x to 0
        // No vertex-state tracking, so both snaps occur independently
        assert_ne!(result[0].0, result[1].0, "edges snap independently");
        assert_eq!(result[0].0, 0, "edge 3→0 snaps x to 0");
        assert_eq!(result[1].0, 1, "edge 0→1 snaps x to 1");
    }

    #[test]
    fn straighten_degenerate_polygon_passthrough() {
        let pts: Vec<(i32, i32)> = vec![(0, 0), (1, 1)];
        assert_eq!(straighten_polygon(&pts, 1), pts);
    }
}
