// src/overpass.rs
//! Overpass API integration: QL query builder and HTTP fetch.

use anyhow::{Result, bail};

use crate::filter::FeatureFilter;
use crate::osm::OsmData;

const DEFAULT_OVERPASS_URL: &str = "https://overpass-api.de/api/interpreter";
const OVERPASS_TIMEOUT_SECS: u64 = 60;

/// Approved Overpass API hostnames. Only HTTPS URLs whose host appears in this
/// list are accepted. All other values are rejected to prevent SSRF attacks.
const ALLOWED_OVERPASS_HOSTS: &[&str] = &[
    "overpass-api.de",
    "overpass.kumi.systems",
    "overpass.openstreetmap.ru",
    "maps.mail.ru",
    "overpass.osm.ch",
];

/// Validate that `url` is a safe Overpass endpoint.
///
/// Rejects any URL that:
/// - does not start with `https://`, or
/// - whose host is not in `ALLOWED_OVERPASS_HOSTS`.
///
/// The host is extracted as the portion between `https://` and the first `/`,
/// `?`, or `#` (or end of string). Port suffixes (`:8080`) are stripped before
/// the allowlist check so that non-standard ports on approved hosts are also
/// permitted.
///
/// Returns `Ok(())` if the URL is acceptable, or an error with a descriptive
/// message otherwise.
pub fn validate_overpass_url(url: &str) -> Result<()> {
    // Scheme check — must be https.
    let rest = url
        .strip_prefix("https://")
        .ok_or_else(|| anyhow::anyhow!("Overpass URL must use HTTPS (got: '{url}')"))?;

    // Extract host (up to the first '/', '?', '#', or end of string).
    let host_and_port = rest.split(&['/', '?', '#']).next().unwrap_or(rest);

    // Strip optional port (":8080").
    let host = host_and_port.split(':').next().unwrap_or(host_and_port);

    if host.is_empty() {
        bail!("Overpass URL has no host");
    }

    if !ALLOWED_OVERPASS_HOSTS.contains(&host) {
        bail!(
            "Overpass host '{}' is not in the approved list. \
             Allowed hosts: {}",
            host,
            ALLOWED_OVERPASS_HOSTS.join(", ")
        );
    }

    Ok(())
}

/// Resolve the Overpass API URL.
/// Priority: `OVERPASS_URL` env var → hardcoded default.
pub fn default_overpass_url() -> &'static str {
    use std::sync::OnceLock;
    static RESOLVED: OnceLock<String> = OnceLock::new();
    RESOLVED
        .get_or_init(|| {
            std::env::var("OVERPASS_URL").unwrap_or_else(|_| DEFAULT_OVERPASS_URL.to_string())
        })
        .as_str()
}

/// Build an Overpass QL query (XML output) for the given bounding box,
/// including only the feature types enabled in `filter`.
///
/// `bbox` is `(south, west, north, east)` in decimal degrees.
pub fn build_overpass_query(bbox: (f64, f64, f64, f64), filter: &FeatureFilter) -> Result<String> {
    let (south, west, north, east) = bbox;
    if south >= north {
        bail!("invalid bbox: south ({south}) must be less than north ({north})");
    }
    if west >= east {
        bail!("invalid bbox: west ({west}) must be less than east ({east})");
    }

    let b = format!("{south},{west},{north},{east}");
    let mut parts: Vec<String> = Vec::new();

    if filter.roads {
        parts.push(format!(r#"way["highway"]({b});"#));
    }
    if filter.buildings {
        parts.push(format!(r#"way["building"]({b});"#));
        // Named addresses on standalone nodes (entrance/door nodes in OSM)
        parts.push(format!(r#"node["addr:housenumber"]({b});"#));
    }
    if filter.water {
        parts.push(format!(r#"way["waterway"]({b});"#));
        parts.push(format!(r#"way["natural"="water"]({b});"#));
    }
    if filter.landuse {
        parts.push(format!(r#"way["landuse"]({b});"#));
        parts.push(format!(r#"way["natural"]({b});"#));
    }
    if filter.railways {
        parts.push(format!(r#"way["railway"="rail"]({b});"#));
    }
    // POI nodes: shops, restaurants, amenities, and tourism spots are
    // predominantly node features in OSM — ways alone would miss most of them.
    parts.push(format!(r#"node["amenity"]({b});"#));
    parts.push(format!(r#"node["shop"]({b});"#));
    parts.push(format!(r#"node["tourism"]({b});"#));
    parts.push(format!(r#"node["leisure"]({b});"#));
    parts.push(format!(r#"node["historic"]({b});"#));

    if parts.is_empty() {
        bail!("all feature types are disabled — nothing to query");
    }

    Ok(format!(
        "[out:xml][timeout:{OVERPASS_TIMEOUT_SECS}];\n({});\nout body;>;out skel qt;",
        parts.join("")
    ))
}

/// Fetch raw OSM XML from the Overpass API for the given bounding box.
///
/// - Validates `overpass_url` against an approved host allowlist (SSRF guard).
/// - Validates `bbox` before making any network request.
/// - Returns a user-readable error for HTTP 429 (server busy).
/// - Uses a blocking `reqwest` client (call from `spawn_blocking`).
pub fn fetch_osm_xml(
    bbox: (f64, f64, f64, f64),
    filter: &FeatureFilter,
    overpass_url: &str,
) -> Result<String> {
    validate_overpass_url(overpass_url)?;
    let query = build_overpass_query(bbox, filter)?;

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(OVERPASS_TIMEOUT_SECS))
        .build()?;

    let res = client
        .post(overpass_url)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(format!("data={}", urlencoding::encode(&query)))
        .send()?;

    if res.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
        bail!("Overpass is busy — try again in a few minutes");
    }
    if !res.status().is_success() {
        let status = res.status();
        let body = res.text().unwrap_or_default();
        bail!("Overpass API error ({status}): {body}");
    }

    Ok(res.text()?)
}

/// Fetch OSM data from Overpass (or cache) and parse it into `OsmData`.
///
/// - `use_cache = true`:  check cache first; write to cache on miss.
/// - `use_cache = false`: always fetch from Overpass; write result to cache.
pub fn fetch_osm_data(
    bbox: (f64, f64, f64, f64),
    filter: &FeatureFilter,
    use_cache: bool,
    overpass_url: &str,
) -> Result<OsmData> {
    let key = crate::osm_cache::cache_key(bbox, filter);

    if use_cache {
        if let Some(xml) = crate::osm_cache::read(&key) {
            log::info!("Cache hit for key {}", &key[..8]);
            return crate::osm::parse_osm_xml_str(&xml);
        }
        // Second-chance: containment lookup
        if let Some(xml) = crate::osm_cache::find_containing(bbox, filter) {
            log::info!("Cache containment hit — reusing larger cached area");
            return crate::osm::parse_osm_xml_str(&xml);
        }
        log::info!("Cache miss — fetching from Overpass (bbox {bbox:?})");
    } else {
        log::info!("Force-fetching from Overpass (bbox {bbox:?})");
    }

    let xml = fetch_osm_xml(bbox, filter, overpass_url)?;

    if let Err(e) = crate::osm_cache::write(&key, bbox, filter, &xml) {
        log::warn!("Cache write failed: {e}");
    }

    crate::osm::parse_osm_xml_str(&xml)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::FeatureFilter;

    #[test]
    fn query_includes_all_types_by_default() {
        let filter = FeatureFilter::default();
        let q = build_overpass_query((51.5, -0.13, 51.52, -0.10), &filter).unwrap();
        assert!(q.contains(r#"way["highway"]"#), "missing highway");
        assert!(q.contains(r#"way["building"]"#), "missing building");
        assert!(q.contains(r#"way["waterway"]"#), "missing waterway");
        assert!(
            q.contains(r#"way["natural"="water"]"#),
            "missing natural water"
        );
        assert!(q.contains(r#"way["landuse"]"#), "missing landuse");
        assert!(q.contains(r#"way["railway"="rail"]"#), "missing railway");
    }

    #[test]
    fn query_excludes_disabled_roads() {
        let filter = FeatureFilter {
            roads: false,
            ..FeatureFilter::default()
        };
        let q = build_overpass_query((51.5, -0.13, 51.52, -0.10), &filter).unwrap();
        assert!(!q.contains(r#"way["highway"]"#));
        assert!(q.contains(r#"way["building"]"#)); // others still present
    }

    #[test]
    fn query_excludes_disabled_water() {
        let filter = FeatureFilter {
            water: false,
            ..FeatureFilter::default()
        };
        let q = build_overpass_query((51.5, -0.13, 51.52, -0.10), &filter).unwrap();
        assert!(!q.contains(r#"way["waterway"]"#));
        assert!(!q.contains(r#"way["natural"="water"]"#));
    }

    #[test]
    fn query_contains_bbox_coords() {
        let filter = FeatureFilter::default();
        let q = build_overpass_query((51.5, -0.13, 51.52, -0.10), &filter).unwrap();
        assert!(q.contains("51.5"), "missing south");
        assert!(q.contains("-0.13"), "missing west");
        assert!(q.contains("51.52"), "missing north");
        assert!(q.contains("-0.1"), "missing east");
    }

    #[test]
    fn invalid_bbox_south_gt_north() {
        let filter = FeatureFilter::default();
        let result = build_overpass_query((51.52, -0.13, 51.5, -0.10), &filter);
        assert!(result.is_err(), "should fail when south >= north");
    }

    #[test]
    fn invalid_bbox_west_gt_east() {
        let filter = FeatureFilter::default();
        let result = build_overpass_query((51.5, -0.10, 51.52, -0.13), &filter);
        assert!(result.is_err(), "should fail when west >= east");
    }

    #[test]
    fn all_disabled_still_queries_poi_nodes() {
        // Even when all feature categories are disabled, POI node queries
        // (amenity, shop, tourism, leisure, historic) are always included.
        let filter = FeatureFilter {
            roads: false,
            buildings: false,
            water: false,
            landuse: false,
            railways: false,
        };
        let q = build_overpass_query((51.5, -0.13, 51.52, -0.10), &filter).unwrap();
        assert!(
            q.contains(r#"node["amenity"]"#),
            "POI node queries should always be present"
        );
        assert!(!q.contains(r#"way["highway"]"#), "roads should be absent");
        assert!(
            !q.contains(r#"way["building"]"#),
            "buildings should be absent"
        );
    }

    // ── validate_overpass_url ──────────────────────────────────────────────

    #[test]
    fn valid_default_overpass_url_is_accepted() {
        assert!(validate_overpass_url("https://overpass-api.de/api/interpreter").is_ok());
    }

    #[test]
    fn valid_mirror_url_is_accepted() {
        assert!(validate_overpass_url("https://overpass.kumi.systems/api/interpreter").is_ok());
    }

    #[test]
    fn http_scheme_is_rejected() {
        let err = validate_overpass_url("http://overpass-api.de/api/interpreter");
        assert!(err.is_err(), "HTTP should be rejected");
        let msg = err.unwrap_err().to_string();
        assert!(msg.contains("HTTPS"), "error should mention HTTPS: {msg}");
    }

    #[test]
    fn unknown_host_is_rejected() {
        let err = validate_overpass_url("https://evil.example.com/api/interpreter");
        assert!(err.is_err(), "unknown host should be rejected");
        let msg = err.unwrap_err().to_string();
        assert!(
            msg.contains("approved list"),
            "error should mention approved list: {msg}"
        );
    }

    #[test]
    fn ssrf_metadata_url_is_rejected() {
        assert!(
            validate_overpass_url("https://169.254.169.254/latest/meta-data/").is_err(),
            "AWS metadata URL must be rejected"
        );
    }

    #[test]
    fn internal_ip_http_is_rejected() {
        assert!(
            validate_overpass_url("http://192.168.1.1/overpass").is_err(),
            "RFC-1918 HTTP URL must be rejected"
        );
    }

    #[test]
    fn url_with_port_on_approved_host_is_accepted() {
        assert!(
            validate_overpass_url("https://overpass-api.de:443/api/interpreter").is_ok(),
            "explicit port on approved host should be allowed"
        );
    }
}
