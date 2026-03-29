//! GeoJSON export for OSM data.
//!
//! Converts parsed [`OsmData`] into a GeoJSON [`FeatureCollection`].
//! Each OSM way becomes a [`Feature`] with its tags as properties plus
//! a `_type` classification property and a `_node_count` property.
//!
//! Closed ways (first node == last node, >= 4 nodes) become `Polygon`
//! geometries; open ways become `LineString` geometries. Ways with fewer
//! than 2 resolvable coordinates are skipped.

use geojson::{Feature, FeatureCollection, GeoJson, Geometry, GeometryValue};
use serde_json::{Map, Value as JsonValue};
use std::collections::HashMap;

use crate::osm::OsmData;

// ── Classification ──────────────────────────────────────────────────────────

/// Classify an OSM way by its tags into one of seven categories.
///
/// Priority (highest to lowest):
/// 1. `highway` → "road"
/// 2. `railway` → "railway"
/// 3. `building` or `building:part` → "building"
/// 4. `waterway`, or `natural=water`, or `landuse` in {reservoir,water,basin} → "water"
/// 5. `barrier` → "barrier"
/// 6. `landuse` or `natural` (any other value) → "landuse"
/// 7. Everything else → "other"
fn classify_way(tags: &HashMap<String, String>) -> &'static str {
    if tags.contains_key("highway") {
        return "road";
    }
    if tags.contains_key("railway") {
        return "railway";
    }
    if tags.contains_key("building") || tags.contains_key("building:part") {
        return "building";
    }
    if tags.contains_key("waterway")
        || tags.get("natural").is_some_and(|v| v == "water")
        || tags
            .get("landuse")
            .is_some_and(|v| matches!(v.as_str(), "reservoir" | "water" | "basin"))
    {
        return "water";
    }
    if tags.contains_key("barrier") {
        return "barrier";
    }
    if tags.contains_key("landuse") || tags.contains_key("natural") {
        return "landuse";
    }
    "other"
}

// ── Geometry helpers ────────────────────────────────────────────────────────

/// Returns `true` when the way's node-ref list forms a closed ring:
/// first ref == last ref and there are at least 4 refs (3 unique + closing).
fn is_closed_way(node_refs: &[i64]) -> bool {
    node_refs.len() >= 4 && node_refs.first() == node_refs.last()
}

// ── Public API ──────────────────────────────────────────────────────────────

/// Convert all ways in `data` to a GeoJSON [`FeatureCollection`].
///
/// Each feature carries:
/// - All OSM tag key/value pairs as string properties.
/// - `"_type"` — classification string ("road", "building", "water",
///   "landuse", or "other").
/// - `"_node_count"` — number of node references in the way (integer).
///
/// Coordinate order follows the GeoJSON spec: `[longitude, latitude]`.
pub fn to_geojson(data: &OsmData) -> FeatureCollection {
    let mut features: Vec<Feature> = data
        .ways
        .iter()
        .filter_map(|way| {
            // Resolve node refs → [lon, lat] coordinate pairs.
            let coords: Vec<Vec<f64>> = way
                .node_refs
                .iter()
                .filter_map(|id| data.nodes.get(id))
                .map(|node| vec![node.lon, node.lat])
                .collect();

            // Skip ways that don't have at least 2 resolved coordinates.
            if coords.len() < 2 {
                return None;
            }

            // Choose geometry type based on whether the way is closed.
            let geometry = if is_closed_way(&way.node_refs) && coords.len() >= 4 {
                // GeoJSON Polygon: outer ring is an array of positions; the ring
                // must be explicitly closed (first == last).
                let mut ring = coords.clone();
                // Ensure the ring is closed (it should be since node_refs is closed,
                // but guard against partial resolution dropping the closing node).
                if ring.first() != ring.last() {
                    ring.push(ring[0].clone());
                }
                Geometry::new(GeometryValue::new_polygon(std::iter::once(ring)))
            } else {
                Geometry::new(GeometryValue::new_line_string(coords))
            };

            // Build properties map: all tags + _type + _node_count.
            let way_type = classify_way(&way.tags);
            let node_count = way.node_refs.len();

            let mut props: Map<String, JsonValue> = way
                .tags
                .iter()
                .map(|(k, v)| (k.clone(), JsonValue::String(v.clone())))
                .collect();
            props.insert("_type".to_string(), JsonValue::String(way_type.to_string()));
            props.insert(
                "_node_count".to_string(),
                JsonValue::Number(node_count.into()),
            );

            Some(Feature {
                bbox: None,
                geometry: Some(geometry),
                id: None,
                properties: Some(props),
                foreign_members: None,
            })
        })
        .collect();

    // Convert multipolygon relations to GeoJSON features
    for rel in &data.relations {
        let mut polygons: Vec<Vec<Vec<Vec<f64>>>> = Vec::new();

        // Resolve outer and inner rings
        let mut outers: Vec<Vec<Vec<f64>>> = Vec::new();
        let mut inners: Vec<Vec<Vec<f64>>> = Vec::new();

        for member in &rel.members {
            if let Some(&idx) = data.ways_by_id.get(&member.way_id) {
                let way = &data.ways[idx];
                let coords: Vec<Vec<f64>> = way
                    .node_refs
                    .iter()
                    .filter_map(|id| data.nodes.get(id))
                    .map(|node| vec![node.lon, node.lat])
                    .collect();
                if coords.len() < 3 {
                    continue;
                }
                let mut ring = coords;
                if ring.first() != ring.last() {
                    ring.push(ring[0].clone());
                }
                match member.role.as_str() {
                    "outer" | "" => outers.push(ring),
                    "inner" => inners.push(ring),
                    _ => {}
                }
            }
        }

        // Build polygons: each outer ring paired with its inner rings
        // Simple approach: assign all inners to the first outer
        // (accurate containment testing would be more complex)
        for (i, outer) in outers.into_iter().enumerate() {
            let mut rings = vec![outer];
            if i == 0 {
                rings.append(&mut inners);
            }
            polygons.push(rings);
        }

        if polygons.is_empty() {
            continue;
        }

        let geometry = if polygons.len() == 1 {
            Geometry::new(GeometryValue::new_polygon(
                polygons.into_iter().next().unwrap(),
            ))
        } else {
            Geometry::new(GeometryValue::new_multi_polygon(polygons))
        };

        let way_type = classify_way(&rel.tags);
        let mut props: Map<String, JsonValue> = rel
            .tags
            .iter()
            .map(|(k, v)| (k.clone(), JsonValue::String(v.clone())))
            .collect();
        props.insert("_type".to_string(), JsonValue::String(way_type.to_string()));
        props.insert(
            "_source".to_string(),
            JsonValue::String("relation".to_string()),
        );

        features.push(Feature {
            bbox: None,
            geometry: Some(geometry),
            id: None,
            properties: Some(props),
            foreign_members: None,
        });
    }

    FeatureCollection {
        bbox: None,
        features,
        foreign_members: None,
    }
}

/// Serialise a [`FeatureCollection`] to a compact JSON string.
///
/// This is a thin convenience wrapper around [`GeoJson`]'s `Display` impl.
#[allow(dead_code)] // convenience helper — may be used by future callers
pub fn to_geojson_string(data: &OsmData) -> String {
    GeoJson::FeatureCollection(to_geojson(data)).to_string()
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::osm::{OsmData, OsmNode, OsmWay};
    use geojson::GeometryValue;
    use std::collections::HashMap;

    fn make_data(ways: Vec<OsmWay>, nodes: Vec<(i64, f64, f64)>) -> OsmData {
        let mut node_map = HashMap::new();
        for (id, lat, lon) in nodes {
            node_map.insert(id, OsmNode { lat, lon });
        }
        OsmData {
            nodes: node_map,
            ways,
            ways_by_id: HashMap::new(),
            relations: Vec::new(),
            bounds: None,
            poi_nodes: Vec::new(),
            addr_nodes: Vec::new(),
            tree_nodes: Vec::new(),
        }
    }

    // ── classify_way ─────────────────────────────────────────────────────

    #[test]
    fn classify_highway() {
        let mut tags = HashMap::new();
        tags.insert("highway".to_string(), "residential".to_string());
        assert_eq!(classify_way(&tags), "road");
    }

    #[test]
    fn classify_building() {
        let mut tags = HashMap::new();
        tags.insert("building".to_string(), "yes".to_string());
        assert_eq!(classify_way(&tags), "building");
    }

    #[test]
    fn classify_building_part() {
        let mut tags = HashMap::new();
        tags.insert("building:part".to_string(), "yes".to_string());
        assert_eq!(classify_way(&tags), "building");
    }

    #[test]
    fn classify_waterway() {
        let mut tags = HashMap::new();
        tags.insert("waterway".to_string(), "river".to_string());
        assert_eq!(classify_way(&tags), "water");
    }

    #[test]
    fn classify_natural_water() {
        let mut tags = HashMap::new();
        tags.insert("natural".to_string(), "water".to_string());
        assert_eq!(classify_way(&tags), "water");
    }

    #[test]
    fn classify_landuse_reservoir() {
        let mut tags = HashMap::new();
        tags.insert("landuse".to_string(), "reservoir".to_string());
        assert_eq!(classify_way(&tags), "water");
    }

    #[test]
    fn classify_landuse_generic() {
        let mut tags = HashMap::new();
        tags.insert("landuse".to_string(), "forest".to_string());
        assert_eq!(classify_way(&tags), "landuse");
    }

    #[test]
    fn classify_natural_generic() {
        let mut tags = HashMap::new();
        tags.insert("natural".to_string(), "wood".to_string());
        assert_eq!(classify_way(&tags), "landuse");
    }

    #[test]
    fn classify_railway() {
        let mut tags = HashMap::new();
        tags.insert("railway".to_string(), "rail".to_string());
        assert_eq!(classify_way(&tags), "railway");
    }

    #[test]
    fn classify_barrier() {
        let mut tags = HashMap::new();
        tags.insert("barrier".to_string(), "fence".to_string());
        assert_eq!(classify_way(&tags), "barrier");
    }

    #[test]
    fn classify_other() {
        let tags = HashMap::new();
        assert_eq!(classify_way(&tags), "other");
    }

    /// highway takes priority over building when both tags are present.
    #[test]
    fn classify_priority_highway_over_building() {
        let mut tags = HashMap::new();
        tags.insert("highway".to_string(), "footway".to_string());
        tags.insert("building".to_string(), "yes".to_string());
        assert_eq!(classify_way(&tags), "road");
    }

    // ── to_geojson ────────────────────────────────────────────────────────

    /// Open way (2 nodes) → LineString.
    #[test]
    fn open_way_becomes_linestring() {
        let way = OsmWay {
            tags: {
                let mut t = HashMap::new();
                t.insert("highway".to_string(), "path".to_string());
                t
            },
            node_refs: vec![1, 2],
        };
        let data = make_data(vec![way], vec![(1, 37.0, -122.0), (2, 37.1, -122.1)]);
        let fc = to_geojson(&data);
        assert_eq!(fc.features.len(), 1);
        let geom = fc.features[0].geometry.as_ref().unwrap();
        assert!(matches!(geom.value, GeometryValue::LineString { .. }));
    }

    /// Closed way (4+ node refs, first == last) → Polygon.
    #[test]
    fn closed_way_becomes_polygon() {
        // Square: nodes 1,2,3,4,1
        let way = OsmWay {
            tags: {
                let mut t = HashMap::new();
                t.insert("building".to_string(), "yes".to_string());
                t
            },
            node_refs: vec![1, 2, 3, 4, 1],
        };
        let data = make_data(
            vec![way],
            vec![
                (1, 37.0, -122.0),
                (2, 37.0, -122.1),
                (3, 37.1, -122.1),
                (4, 37.1, -122.0),
            ],
        );
        let fc = to_geojson(&data);
        assert_eq!(fc.features.len(), 1);
        let geom = fc.features[0].geometry.as_ref().unwrap();
        assert!(matches!(geom.value, GeometryValue::Polygon { .. }));
    }

    /// Ways with fewer than 2 resolvable nodes are skipped.
    #[test]
    fn way_with_missing_nodes_is_skipped() {
        let way = OsmWay {
            tags: HashMap::new(),
            node_refs: vec![99], // node 99 doesn't exist
        };
        let data = make_data(vec![way], vec![]);
        let fc = to_geojson(&data);
        assert_eq!(fc.features.len(), 0);
    }

    /// Properties include all tags, `_type`, and `_node_count`.
    #[test]
    fn properties_include_type_and_node_count() {
        let way = OsmWay {
            tags: {
                let mut t = HashMap::new();
                t.insert("highway".to_string(), "primary".to_string());
                t.insert("name".to_string(), "Main St".to_string());
                t
            },
            node_refs: vec![1, 2],
        };
        let data = make_data(vec![way], vec![(1, 0.0, 0.0), (2, 1.0, 1.0)]);
        let fc = to_geojson(&data);
        let props = fc.features[0].properties.as_ref().unwrap();
        assert_eq!(props["_type"], JsonValue::String("road".to_string()));
        assert_eq!(props["_node_count"], JsonValue::Number(2.into()));
        assert_eq!(props["highway"], JsonValue::String("primary".to_string()));
        assert_eq!(props["name"], JsonValue::String("Main St".to_string()));
    }

    /// Coordinates are [lon, lat] order per GeoJSON spec.
    #[test]
    fn coordinates_are_lon_lat_order() {
        let way = OsmWay {
            tags: HashMap::new(),
            node_refs: vec![1, 2],
        };
        // lat=10.0, lon=20.0 → GeoJSON position should be [20.0, 10.0]
        let data = make_data(vec![way], vec![(1, 10.0, 20.0), (2, 11.0, 21.0)]);
        let fc = to_geojson(&data);
        let geom = fc.features[0].geometry.as_ref().unwrap();
        if let GeometryValue::LineString { ref coordinates } = geom.value {
            // Position implements PartialEq; compare by converting the expected
            // [lon, lat] arrays into Position via the From<Vec<f64>> impl.
            assert_eq!(
                coordinates[0],
                geojson::Position::from(vec![20.0_f64, 10.0])
            );
            assert_eq!(
                coordinates[1],
                geojson::Position::from(vec![21.0_f64, 11.0])
            );
        } else {
            panic!("expected LineString");
        }
    }

    /// Polygon ring is explicitly closed (first == last position).
    #[test]
    fn polygon_ring_is_closed() {
        let way = OsmWay {
            tags: HashMap::new(),
            node_refs: vec![1, 2, 3, 4, 1],
        };
        let data = make_data(
            vec![way],
            vec![(1, 0.0, 0.0), (2, 0.0, 1.0), (3, 1.0, 1.0), (4, 1.0, 0.0)],
        );
        let fc = to_geojson(&data);
        let geom = fc.features[0].geometry.as_ref().unwrap();
        if let GeometryValue::Polygon { ref coordinates } = geom.value {
            let ring = &coordinates[0];
            assert_eq!(ring.first(), ring.last(), "polygon ring must be closed");
        } else {
            panic!("expected Polygon");
        }
    }
}
