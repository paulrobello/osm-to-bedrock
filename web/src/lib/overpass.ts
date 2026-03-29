import type * as GeoJSON from 'geojson';

export interface OverpassNode {
  type: 'node';
  id: number;
  lat: number;
  lon: number;
  tags?: Record<string, string>;
}

export interface OverpassWay {
  type: 'way';
  id: number;
  nodes: number[];
  tags?: Record<string, string>;
}

export type OverpassElement = OverpassNode | OverpassWay;

export interface OverpassResponse {
  version: number;
  generator: string;
  elements: OverpassElement[];
}

/** Which OSM feature types to include. All fields default to true. */
export interface FeatureFilter {
  roads: boolean;
  buildings: boolean;
  water: boolean;
  landuse: boolean;
  railways: boolean;
}

export const defaultFilter: FeatureFilter = {
  roads: true,
  buildings: true,
  water: true,
  landuse: true,
  railways: true,
};

/**
 * Builds an Overpass QL query (JSON output) for a bounding box.
 * Used for map preview overlays — returns JSON format.
 * The Rust server uses a separate XML-format query for conversion.
 *
 * @param bbox - [south, west, north, east] in decimal degrees
 * @param filter - which feature types to include (defaults to all)
 */
export function buildQuery(
  bbox: [number, number, number, number],
  filter: FeatureFilter = defaultFilter
): string {
  const [s, w, n, e] = bbox;
  const b = `${s},${w},${n},${e}`;
  const parts: string[] = [];

  if (filter.roads)     parts.push(`way["highway"](${b});`);
  if (filter.buildings) parts.push(`way["building"](${b});`);
  if (filter.water) {
    parts.push(`way["waterway"](${b});`);
    parts.push(`way["natural"="water"](${b});`);
  }
  if (filter.landuse)   parts.push(`way["landuse"](${b});`);
  if (filter.railways)  parts.push(`way["railway"="rail"](${b});`);

  return (
    `[out:json][timeout:30];\n(${parts.join('')});\nout body;>;out skel qt;`
  );
}

type FeatureType = 'road' | 'building' | 'water' | 'landuse' | 'other';

function classifyTags(tags: Record<string, string>): FeatureType {
  if (tags['highway']) return 'road';
  if (tags['building']) return 'building';
  if (tags['waterway'] || tags['natural'] === 'water') return 'water';
  if (tags['landuse']) return 'landuse';
  return 'other';
}

/**
 * Converts an Overpass JSON response to a GeoJSON FeatureCollection.
 * Each feature gets a `_type` property classifying it as road/building/water/landuse/other.
 * @param data - Raw Overpass API response
 * @returns GeoJSON FeatureCollection
 */
export function overpassToGeoJSON(data: OverpassResponse): GeoJSON.FeatureCollection {
  // Build node lookup
  const nodeMap = new Map<number, OverpassNode>();
  for (const el of data.elements) {
    if (el.type === 'node') {
      nodeMap.set(el.id, el);
    }
  }

  const features: GeoJSON.Feature[] = [];

  for (const el of data.elements) {
    if (el.type !== 'way') continue;
    const way = el as OverpassWay;
    if (!way.tags) continue;

    const coords: GeoJSON.Position[] = [];
    for (const nodeId of way.nodes) {
      const node = nodeMap.get(nodeId);
      if (node) {
        coords.push([node.lon, node.lat]);
      }
    }

    if (coords.length < 2) continue;

    const tags = way.tags;
    const featureType = classifyTags(tags);

    // Closed rings with enough coords become polygons (buildings, landuse, water areas)
    const isClosed =
      coords.length >= 4 &&
      coords[0][0] === coords[coords.length - 1][0] &&
      coords[0][1] === coords[coords.length - 1][1];

    const isPolygonType =
      featureType === 'building' || featureType === 'landuse' || featureType === 'water';

    let geometry: GeoJSON.Geometry;
    if (isClosed && isPolygonType) {
      geometry = {
        type: 'Polygon',
        coordinates: [coords],
      };
    } else {
      geometry = {
        type: 'LineString',
        coordinates: coords,
      };
    }

    features.push({
      type: 'Feature',
      id: way.id,
      geometry,
      properties: {
        ...tags,
        _type: featureType,
        _osm_id: way.id,
      },
    });
  }

  return {
    type: 'FeatureCollection',
    features,
  };
}
