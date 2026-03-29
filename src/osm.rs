//! OSM PBF file parser.
//!
//! Reads nodes, ways, and their tags from a `.osm.pbf` file.

use anyhow::{Context, Result};
use osmpbf::{Element, ElementReader};
use quick_xml::Reader;
use quick_xml::events::Event;
use std::collections::HashMap;
use std::path::Path;

/// A geographic point from the OSM dataset.
#[derive(Debug, Clone, Copy)]
pub struct OsmNode {
    pub lat: f64,
    pub lon: f64,
}

/// An OSM node that carries feature tags (amenity, shop, tourism, etc.).
/// Used for POI marker placement.
#[derive(Debug, Clone)]
pub struct OsmPoiNode {
    pub lat: f64,
    pub lon: f64,
    pub tags: HashMap<String, String>,
}

/// An OSM way: an ordered sequence of node references with tags.
#[derive(Debug, Clone)]
pub struct OsmWay {
    pub tags: HashMap<String, String>,
    pub node_refs: Vec<i64>,
}

/// A member of an OSM relation with its role.
#[derive(Debug, Clone)]
pub struct RelationMember {
    /// Way ID referenced by this member.
    pub way_id: i64,
    /// Role string (e.g. "outer", "inner").
    pub role: String,
}

/// An OSM relation: a collection of ways with roles and tags.
#[derive(Debug, Clone)]
pub struct OsmRelation {
    pub tags: HashMap<String, String>,
    pub members: Vec<RelationMember>,
}

/// Parsed OSM dataset.
pub struct OsmData {
    pub nodes: HashMap<i64, OsmNode>,
    pub ways: Vec<OsmWay>,
    /// Way lookup by ID for relation member resolution.
    ///
    /// Maps OSM way ID → index into .  Using an index rather than a
    /// cloned  halves peak memory for the ways collection.
    pub ways_by_id: HashMap<i64, usize>,
    /// Multipolygon relations.
    pub relations: Vec<OsmRelation>,
    /// Bounding box: (min_lat, min_lon, max_lat, max_lon)
    pub bounds: Option<(f64, f64, f64, f64)>,
    /// Standalone nodes with POI tags (amenity, shop, tourism, leisure, historic).
    pub poi_nodes: Vec<OsmPoiNode>,
    /// Standalone nodes with address tags (addr:housenumber).
    /// These are typically entrance/door nodes placed on building outlines in OSM.
    pub addr_nodes: Vec<OsmPoiNode>,
    /// Individual tree positions (from OSM `natural=tree` or Overture `land/tree`).
    pub tree_nodes: Vec<OsmNode>,
}

impl OsmData {
    /// Merge another `OsmData` into this one, combining nodes, ways, and bounds.
    pub fn merge(&mut self, other: OsmData) {
        self.nodes.extend(other.nodes);
        let offset = self.ways.len();
        self.ways.extend(other.ways);
        // Adjust indices from  to account for the ways already in .
        self.ways_by_id.extend(
            other
                .ways_by_id
                .into_iter()
                .map(|(id, idx)| (id, idx + offset)),
        );
        self.relations.extend(other.relations);
        self.poi_nodes.extend(other.poi_nodes);
        self.addr_nodes.extend(other.addr_nodes);
        self.tree_nodes.extend(other.tree_nodes);
        match (self.bounds, other.bounds) {
            (Some((a0, a1, a2, a3)), Some((b0, b1, b2, b3))) => {
                self.bounds = Some((a0.min(b0), a1.min(b1), a2.max(b2), a3.max(b3)));
            }
            (None, b) => self.bounds = b,
            _ => {}
        }
    }

    /// Clip data to a bounding box, keeping only features that touch the bbox.
    ///
    /// `bbox` is `(min_lat, min_lon, max_lat, max_lon)`.
    /// Ways are kept if at least one node falls inside the bbox.
    /// POI and address nodes are kept only if inside the bbox.
    /// Unreferenced nodes are pruned.
    pub fn clip_to_bbox(&mut self, bbox: (f64, f64, f64, f64)) {
        let (min_lat, min_lon, max_lat, max_lon) = bbox;

        let in_bbox = |lat: f64, lon: f64| -> bool {
            lat >= min_lat && lat <= max_lat && lon >= min_lon && lon <= max_lon
        };

        // Filter ways: keep if any node is inside the bbox
        let mut keep_node_ids: std::collections::HashSet<i64> = std::collections::HashSet::new();
        let mut new_ways = Vec::new();
        let mut new_ways_by_id = HashMap::new();
        for (way_id, &old_idx) in &self.ways_by_id {
            let way = &self.ways[old_idx];
            let touches_bbox = way
                .node_refs
                .iter()
                .any(|id| self.nodes.get(id).is_some_and(|n| in_bbox(n.lat, n.lon)));
            if touches_bbox {
                for id in &way.node_refs {
                    keep_node_ids.insert(*id);
                }
                let new_idx = new_ways.len();
                new_ways.push(way.clone());
                new_ways_by_id.insert(*way_id, new_idx);
            }
        }
        // Also keep ways not in ways_by_id (shouldn't happen, but be safe)
        // by scanning ways without a ways_by_id entry
        let indexed: std::collections::HashSet<usize> = self.ways_by_id.values().copied().collect();
        for (i, way) in self.ways.iter().enumerate() {
            if indexed.contains(&i) {
                continue;
            }
            let touches_bbox = way
                .node_refs
                .iter()
                .any(|id| self.nodes.get(id).is_some_and(|n| in_bbox(n.lat, n.lon)));
            if touches_bbox {
                for id in &way.node_refs {
                    keep_node_ids.insert(*id);
                }
                new_ways.push(way.clone());
            }
        }

        self.ways = new_ways;
        self.ways_by_id = new_ways_by_id;

        // Prune nodes to only those referenced by kept ways
        self.nodes.retain(|id, _| keep_node_ids.contains(id));

        // Filter POI and address nodes
        self.poi_nodes.retain(|p| in_bbox(p.lat, p.lon));
        self.addr_nodes.retain(|p| in_bbox(p.lat, p.lon));
        self.tree_nodes.retain(|n| in_bbox(n.lat, n.lon));

        // Filter relations: keep if any member way was kept
        self.relations.retain(|rel| {
            rel.members
                .iter()
                .any(|m| self.ways_by_id.contains_key(&m.way_id))
        });

        // Update bounds to the requested bbox
        self.bounds = Some(bbox);
    }
}

/// Parse a `.osm.pbf` file and return all nodes and ways.
pub fn parse_pbf(path: &Path) -> Result<OsmData> {
    let reader =
        ElementReader::from_path(path).with_context(|| format!("opening {}", path.display()))?;

    let mut nodes: HashMap<i64, OsmNode> = HashMap::new();
    let mut ways: Vec<OsmWay> = Vec::new();
    let mut ways_by_id: HashMap<i64, usize> = HashMap::new();
    let mut relations: Vec<OsmRelation> = Vec::new();
    let mut poi_nodes: Vec<OsmPoiNode> = Vec::new();
    let mut addr_nodes: Vec<OsmPoiNode> = Vec::new();
    let mut tree_nodes: Vec<OsmNode> = Vec::new();
    let mut min_lat = f64::MAX;
    let mut min_lon = f64::MAX;
    let mut max_lat = f64::MIN;
    let mut max_lon = f64::MIN;

    reader
        .for_each(|element| match element {
            Element::Node(n) => {
                let lat = n.lat();
                let lon = n.lon();
                min_lat = min_lat.min(lat);
                min_lon = min_lon.min(lon);
                max_lat = max_lat.max(lat);
                max_lon = max_lon.max(lon);
                nodes.insert(n.id(), OsmNode { lat, lon });
                let tags: HashMap<String, String> = n
                    .tags()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect();
                if tags.keys().any(|k| {
                    matches!(
                        k.as_str(),
                        "amenity" | "shop" | "tourism" | "leisure" | "historic"
                    )
                }) {
                    poi_nodes.push(OsmPoiNode {
                        lat,
                        lon,
                        tags: tags.clone(),
                    });
                }
                if tags.contains_key("addr:housenumber") {
                    addr_nodes.push(OsmPoiNode {
                        lat,
                        lon,
                        tags: tags.clone(),
                    });
                }
                if tags.get("natural").map(|s| s.as_str()) == Some("tree") {
                    tree_nodes.push(OsmNode { lat, lon });
                }
            }
            Element::DenseNode(n) => {
                let lat = n.lat();
                let lon = n.lon();
                min_lat = min_lat.min(lat);
                min_lon = min_lon.min(lon);
                max_lat = max_lat.max(lat);
                max_lon = max_lon.max(lon);
                nodes.insert(n.id(), OsmNode { lat, lon });
                let tags: HashMap<String, String> = n
                    .tags()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect();
                if tags.keys().any(|k| {
                    matches!(
                        k.as_str(),
                        "amenity" | "shop" | "tourism" | "leisure" | "historic"
                    )
                }) {
                    poi_nodes.push(OsmPoiNode {
                        lat,
                        lon,
                        tags: tags.clone(),
                    });
                }
                if tags.contains_key("addr:housenumber") {
                    addr_nodes.push(OsmPoiNode {
                        lat,
                        lon,
                        tags: tags.clone(),
                    });
                }
                if tags.get("natural").map(|s| s.as_str()) == Some("tree") {
                    tree_nodes.push(OsmNode { lat, lon });
                }
            }
            Element::Way(w) => {
                let tags: HashMap<String, String> = w
                    .tags()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect();
                let node_refs: Vec<i64> = w.refs().collect();
                let way = OsmWay {
                    tags: tags.clone(),
                    node_refs: node_refs.clone(),
                };
                let idx = ways.len();
                ways.push(way);
                ways_by_id.insert(w.id(), idx);
            }
            Element::Relation(r) => {
                let rel_type = r.tags().find(|(k, _)| *k == "type").map(|(_, v)| v);
                if rel_type == Some("multipolygon") {
                    let tags: HashMap<String, String> = r
                        .tags()
                        .map(|(k, v)| (k.to_string(), v.to_string()))
                        .collect();
                    let members: Vec<RelationMember> = r
                        .members()
                        .filter_map(|m| {
                            if matches!(m.member_type, osmpbf::elements::RelMemberType::Way) {
                                Some(RelationMember {
                                    way_id: m.member_id,
                                    role: m.role().unwrap_or_default().to_string(),
                                })
                            } else {
                                None
                            }
                        })
                        .collect();
                    if !members.is_empty() {
                        relations.push(OsmRelation { tags, members });
                    }
                }
            }
        })
        .context("reading PBF elements")?;

    let bounds = if min_lat < f64::MAX {
        Some((min_lat, min_lon, max_lat, max_lon))
    } else {
        None
    };

    log::info!(
        "Parsed {} nodes, {} ways, {} relations, {} POI nodes, {} address nodes, {} tree nodes",
        nodes.len(),
        ways.len(),
        relations.len(),
        poi_nodes.len(),
        addr_nodes.len(),
        tree_nodes.len()
    );

    Ok(OsmData {
        nodes,
        ways,
        ways_by_id,
        relations,
        bounds,
        poi_nodes,
        addr_nodes,
        tree_nodes,
    })
}

/// Parse an OSM XML string into `OsmData`.
///
/// Uses a two-pass approach: nodes are collected in the first pass so that
/// way node references resolve correctly regardless of element ordering
/// (Overpass does not guarantee nodes appear before ways).
pub fn parse_osm_xml_str(xml: &str) -> Result<OsmData> {
    // ── Pass 1: collect all nodes (and POI-tagged nodes) ─────────────────
    let mut nodes: HashMap<i64, OsmNode> = HashMap::new();
    let mut poi_nodes: Vec<OsmPoiNode> = Vec::new();
    let mut addr_nodes: Vec<OsmPoiNode> = Vec::new();
    let mut tree_nodes: Vec<OsmNode> = Vec::new();
    let mut min_lat = f64::MAX;
    let mut min_lon = f64::MAX;
    let mut max_lat = f64::MIN;
    let mut max_lon = f64::MIN;

    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    // State for nodes that have child <tag> elements
    let mut in_node = false;
    let mut cur_lat = 0.0f64;
    let mut cur_lon = 0.0f64;
    let mut cur_node_tags: HashMap<String, String> = HashMap::new();

    loop {
        match reader.read_event() {
            // Self-closing node (no child tags)
            Ok(Event::Empty(ref e)) if e.name().as_ref() == b"node" => {
                let mut id: Option<i64> = None;
                let mut lat: Option<f64> = None;
                let mut lon: Option<f64> = None;
                for attr in e.attributes().flatten() {
                    match attr.key.as_ref() {
                        b"id" => {
                            id = std::str::from_utf8(&attr.value)
                                .ok()
                                .and_then(|s| s.parse().ok())
                        }
                        b"lat" => {
                            lat = std::str::from_utf8(&attr.value)
                                .ok()
                                .and_then(|s| s.parse().ok())
                        }
                        b"lon" => {
                            lon = std::str::from_utf8(&attr.value)
                                .ok()
                                .and_then(|s| s.parse().ok())
                        }
                        _ => {}
                    }
                }
                if let (Some(id), Some(lat), Some(lon)) = (id, lat, lon) {
                    min_lat = min_lat.min(lat);
                    min_lon = min_lon.min(lon);
                    max_lat = max_lat.max(lat);
                    max_lon = max_lon.max(lon);
                    nodes.insert(id, OsmNode { lat, lon });
                }
            }
            // Opening <node> tag with child <tag> elements
            Ok(Event::Start(ref e)) if e.name().as_ref() == b"node" => {
                let mut id: Option<i64> = None;
                let mut lat: Option<f64> = None;
                let mut lon: Option<f64> = None;
                for attr in e.attributes().flatten() {
                    match attr.key.as_ref() {
                        b"id" => {
                            id = std::str::from_utf8(&attr.value)
                                .ok()
                                .and_then(|s| s.parse().ok())
                        }
                        b"lat" => {
                            lat = std::str::from_utf8(&attr.value)
                                .ok()
                                .and_then(|s| s.parse().ok())
                        }
                        b"lon" => {
                            lon = std::str::from_utf8(&attr.value)
                                .ok()
                                .and_then(|s| s.parse().ok())
                        }
                        _ => {}
                    }
                }
                if let (Some(id), Some(lat), Some(lon)) = (id, lat, lon) {
                    min_lat = min_lat.min(lat);
                    min_lon = min_lon.min(lon);
                    max_lat = max_lat.max(lat);
                    max_lon = max_lon.max(lon);
                    nodes.insert(id, OsmNode { lat, lon });
                    in_node = true;
                    cur_lat = lat;
                    cur_lon = lon;
                    cur_node_tags.clear();
                }
            }
            // <tag> child inside a node
            Ok(Event::Empty(ref e)) if in_node && e.name().as_ref() == b"tag" => {
                let mut k = String::new();
                let mut v = String::new();
                for attr in e.attributes().flatten() {
                    match attr.key.as_ref() {
                        b"k" => k = std::str::from_utf8(&attr.value).unwrap_or("").to_string(),
                        b"v" => v = std::str::from_utf8(&attr.value).unwrap_or("").to_string(),
                        _ => {}
                    }
                }
                if !k.is_empty() {
                    cur_node_tags.insert(k, v);
                }
            }
            // Closing </node>
            Ok(Event::End(ref e)) if e.name().as_ref() == b"node" && in_node => {
                in_node = false;
                if cur_node_tags.keys().any(|k| {
                    matches!(
                        k.as_str(),
                        "amenity" | "shop" | "tourism" | "leisure" | "historic"
                    )
                }) {
                    poi_nodes.push(OsmPoiNode {
                        lat: cur_lat,
                        lon: cur_lon,
                        tags: cur_node_tags.clone(),
                    });
                }
                if cur_node_tags.contains_key("addr:housenumber") {
                    addr_nodes.push(OsmPoiNode {
                        lat: cur_lat,
                        lon: cur_lon,
                        tags: cur_node_tags.clone(),
                    });
                }
                if cur_node_tags.get("natural").map(|s| s.as_str()) == Some("tree") {
                    tree_nodes.push(OsmNode {
                        lat: cur_lat,
                        lon: cur_lon,
                    });
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                return Err(anyhow::anyhow!(
                    "XML parse error at position {}: {e}",
                    reader.buffer_position()
                ));
            }
            _ => {}
        }
    }

    // ── Pass 2: collect ways and relations ───────────────────────────────
    let mut ways: Vec<OsmWay> = Vec::new();
    let mut ways_by_id: HashMap<i64, usize> = HashMap::new();
    let mut relations: Vec<OsmRelation> = Vec::new();

    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut in_way = false;
    let mut in_relation = false;
    let mut current_way_id: i64 = 0;
    let mut current_tags: HashMap<String, String> = HashMap::new();
    let mut current_node_refs: Vec<i64> = Vec::new();
    let mut current_members: Vec<RelationMember> = Vec::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => match e.name().as_ref() {
                b"way" => {
                    in_way = true;
                    current_way_id = e
                        .attributes()
                        .flatten()
                        .find(|a| a.key.as_ref() == b"id")
                        .and_then(|a| std::str::from_utf8(&a.value).ok()?.parse().ok())
                        .unwrap_or(0);
                    current_tags.clear();
                    current_node_refs.clear();
                }
                b"relation" => {
                    in_relation = true;
                    current_tags.clear();
                    current_members.clear();
                }
                _ => {}
            },
            Ok(Event::Empty(ref e)) => match e.name().as_ref() {
                b"nd" if in_way => {
                    if let Some(r) = e
                        .attributes()
                        .flatten()
                        .find(|a| a.key.as_ref() == b"ref")
                        .and_then(|a| std::str::from_utf8(&a.value).ok()?.parse::<i64>().ok())
                    {
                        current_node_refs.push(r);
                    }
                }
                b"tag" if in_way || in_relation => {
                    let mut k = String::new();
                    let mut v = String::new();
                    for attr in e.attributes().flatten() {
                        match attr.key.as_ref() {
                            b"k" => k = std::str::from_utf8(&attr.value).unwrap_or("").to_string(),
                            b"v" => v = std::str::from_utf8(&attr.value).unwrap_or("").to_string(),
                            _ => {}
                        }
                    }
                    if !k.is_empty() {
                        current_tags.insert(k, v);
                    }
                }
                b"member" if in_relation => {
                    let mut mtype = String::new();
                    let mut mref: i64 = 0;
                    let mut mrole = String::new();
                    for attr in e.attributes().flatten() {
                        match attr.key.as_ref() {
                            b"type" => {
                                mtype = std::str::from_utf8(&attr.value).unwrap_or("").to_string()
                            }
                            b"ref" => {
                                mref = std::str::from_utf8(&attr.value)
                                    .ok()
                                    .and_then(|s| s.parse().ok())
                                    .unwrap_or(0)
                            }
                            b"role" => {
                                mrole = std::str::from_utf8(&attr.value).unwrap_or("").to_string()
                            }
                            _ => {}
                        }
                    }
                    if mtype == "way" && mref != 0 {
                        current_members.push(RelationMember {
                            way_id: mref,
                            role: mrole,
                        });
                    }
                }
                _ => {}
            },
            Ok(Event::End(ref e)) => match e.name().as_ref() {
                b"way" if in_way => {
                    in_way = false;
                    let way = OsmWay {
                        tags: current_tags.clone(),
                        node_refs: current_node_refs.clone(),
                    };
                    let idx = ways.len();
                    ways.push(way);
                    ways_by_id.insert(current_way_id, idx);
                }
                b"relation" if in_relation => {
                    in_relation = false;
                    let rel_type = current_tags.get("type").map(|s| s.as_str());
                    if rel_type == Some("multipolygon") && !current_members.is_empty() {
                        relations.push(OsmRelation {
                            tags: current_tags.clone(),
                            members: current_members.clone(),
                        });
                    }
                }
                _ => {}
            },
            Ok(Event::Eof) => break,
            Err(e) => return Err(anyhow::anyhow!("XML parse error: {e}")),
            _ => {}
        }
    }

    let bounds = if min_lat < f64::MAX {
        Some((min_lat, min_lon, max_lat, max_lon))
    } else {
        None
    };

    log::info!(
        "Parsed {} nodes, {} ways, {} relations, {} POI nodes, {} address nodes, {} tree nodes (XML)",
        nodes.len(),
        ways.len(),
        relations.len(),
        poi_nodes.len(),
        addr_nodes.len(),
        tree_nodes.len()
    );

    Ok(OsmData {
        nodes,
        ways,
        ways_by_id,
        relations,
        bounds,
        poi_nodes,
        addr_nodes,
        tree_nodes,
    })
}

/// Parse a `.osm` XML file into `OsmData`.
pub fn parse_osm_xml(path: &Path) -> Result<OsmData> {
    let xml =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    parse_osm_xml_str(&xml)
}

/// Detect file format by extension and dispatch to the correct parser.
/// Supports `.osm.pbf` / `.pbf` (PBF format) and `.osm` (XML format).
pub fn parse_osm_file(path: &Path) -> Result<OsmData> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    match ext {
        "pbf" => parse_pbf(path),
        "osm" => parse_osm_xml(path),
        other => Err(anyhow::anyhow!(
            "unsupported file format '.{other}'; expected .osm.pbf or .osm"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL_OSM: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<osm version="0.6">
  <node id="1" lat="51.5" lon="-0.10"/>
  <node id="2" lat="51.5" lon="-0.09"/>
  <node id="3" lat="51.51" lon="-0.09"/>
  <way id="10">
    <nd ref="1"/>
    <nd ref="2"/>
    <nd ref="3"/>
    <tag k="highway" v="residential"/>
    <tag k="name" v="Test Street"/>
  </way>
</osm>"#;

    #[test]
    fn parse_xml_nodes() {
        let data = parse_osm_xml_str(MINIMAL_OSM).unwrap();
        assert_eq!(data.nodes.len(), 3);
        let n = data.nodes.get(&1).unwrap();
        assert!((n.lat - 51.5).abs() < 0.0001);
        assert!((n.lon - -0.10).abs() < 0.0001);
    }

    #[test]
    fn parse_xml_ways() {
        let data = parse_osm_xml_str(MINIMAL_OSM).unwrap();
        assert_eq!(data.ways.len(), 1);
        assert_eq!(data.ways[0].tags["highway"], "residential");
        assert_eq!(data.ways[0].tags["name"], "Test Street");
        assert_eq!(data.ways[0].node_refs, vec![1, 2, 3]);
    }

    #[test]
    fn parse_xml_bounds_computed_from_nodes() {
        let data = parse_osm_xml_str(MINIMAL_OSM).unwrap();
        let (min_lat, min_lon, max_lat, max_lon) = data.bounds.unwrap();
        assert!((min_lat - 51.5).abs() < 0.0001);
        assert!((max_lat - 51.51).abs() < 0.0001);
        assert!((min_lon - -0.10).abs() < 0.0001);
        assert!((max_lon - -0.09).abs() < 0.0001);
    }

    #[test]
    fn parse_xml_nodes_after_ways() {
        // Overpass does not guarantee node-before-way ordering
        let xml = r#"<?xml version="1.0"?>
<osm version="0.6">
  <way id="1">
    <nd ref="10"/>
    <nd ref="11"/>
    <tag k="highway" v="primary"/>
  </way>
  <node id="10" lat="1.0" lon="1.0"/>
  <node id="11" lat="1.1" lon="1.1"/>
</osm>"#;
        let data = parse_osm_xml_str(xml).unwrap();
        assert_eq!(data.nodes.len(), 2);
        assert_eq!(data.ways.len(), 1);
        assert_eq!(data.ways[0].node_refs, vec![10, 11]);
    }

    #[test]
    fn parse_xml_multipolygon_relation() {
        let xml = r#"<?xml version="1.0"?>
<osm version="0.6">
  <node id="1" lat="0.0" lon="0.0"/>
  <way id="100">
    <nd ref="1"/>
    <tag k="landuse" v="park"/>
  </way>
  <relation id="200">
    <member type="way" ref="100" role="outer"/>
    <tag k="type" v="multipolygon"/>
    <tag k="landuse" v="park"/>
  </relation>
</osm>"#;
        let data = parse_osm_xml_str(xml).unwrap();
        assert_eq!(data.relations.len(), 1);
        assert_eq!(data.relations[0].members[0].way_id, 100);
        assert_eq!(data.relations[0].members[0].role, "outer");
        assert_eq!(data.relations[0].tags["landuse"], "park");
    }

    #[test]
    fn parse_xml_poi_nodes_collected() {
        let xml = r#"<?xml version="1.0"?>
<osm version="0.6">
  <node id="1" lat="51.5" lon="-0.10"/>
  <node id="2" lat="51.51" lon="-0.11">
    <tag k="amenity" v="restaurant"/>
    <tag k="name" v="The Pub"/>
  </node>
  <node id="3" lat="51.52" lon="-0.12">
    <tag k="shop" v="supermarket"/>
  </node>
</osm>"#;
        let data = parse_osm_xml_str(xml).unwrap();
        assert_eq!(data.nodes.len(), 3);
        assert_eq!(data.poi_nodes.len(), 2);
        assert_eq!(data.poi_nodes[0].tags["amenity"], "restaurant");
        assert_eq!(data.poi_nodes[0].tags["name"], "The Pub");
        assert_eq!(data.poi_nodes[1].tags["shop"], "supermarket");
    }

    #[test]
    fn parse_xml_non_poi_nodes_not_collected() {
        let xml = r#"<?xml version="1.0"?>
<osm version="0.6">
  <node id="1" lat="51.5" lon="-0.10"/>
  <node id="2" lat="51.51" lon="-0.11">
    <tag k="natural" v="tree"/>
  </node>
</osm>"#;
        let data = parse_osm_xml_str(xml).unwrap();
        assert_eq!(data.poi_nodes.len(), 0);
    }

    #[test]
    fn parse_osm_file_detects_format() {
        use std::io::Write;
        let mut tmp = tempfile::Builder::new().suffix(".osm").tempfile().unwrap();
        tmp.write_all(MINIMAL_OSM.as_bytes()).unwrap();
        let (_, path) = tmp.into_parts();
        let data = parse_osm_file(&path).unwrap();
        assert_eq!(data.nodes.len(), 3);
    }
}
