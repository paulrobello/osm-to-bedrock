'use client';

import { useCallback, useEffect, useRef, useState } from 'react';
import type Map from 'ol/Map';
import type VectorLayer from 'ol/layer/Vector';
import type VectorSource from 'ol/source/Vector';
import type { FeatureLike } from 'ol/Feature';
import type { FeatureCollection } from 'geojson';
import type * as GeoJSON from 'geojson';

// Default map centre — Woodland, CA, USA.
const DEFAULT_LON = -121.7733;
const DEFAULT_LAT = 38.6785;
const DEFAULT_ZOOM = 13;

export interface CursorPosition {
  lon: number;
  lat: number;
}

export interface SelectedFeatureData {
  properties: Record<string, string>;
  geometry_type: string;
}

export interface CacheEntry {
  key: string;
  bbox: [number, number, number, number]; // [south, west, north, east]
  filter: {
    roads: boolean;
    buildings: boolean;
    water: boolean;
    landuse: boolean;
    railways: boolean;
  };
  created_at: string; // ISO 8601
  size_bytes: number;
}

export interface UseMapReturn {
  mapRef: React.RefObject<HTMLDivElement | null>;
  map: Map | null;
  cursorPosition: CursorPosition | null;
  zoom: number;
  featureCount: number;
  flyTo: (bbox: [number, number, number, number], center: [number, number]) => Promise<void>;
  loadGeoJSON: (geojson: FeatureCollection) => Record<string, number>;
  setLayerVisible: (layerId: string, visible: boolean) => void;
  enableBboxDraw: (onDone: (bbox: [number, number, number, number]) => void) => void;
  disableBboxDraw: () => void;
  enableSpawnMode: (onDone: (pos: { lat: number; lon: number }) => void) => void;
  disableSpawnMode: () => void;
  setSelectedFeatureById: (id: string | number | null) => void;
  onFeatureClick: React.MutableRefObject<((feat: SelectedFeatureData) => void) | null>;
  loadCacheAreas: (entries: CacheEntry[]) => void;
}

// Layer style definitions
const LAYER_STYLES: Record<string, { stroke: string; fill: string; fillOpacity: number; strokeWidth: number }> = {
  roads: { stroke: '#888888', fill: 'transparent', fillOpacity: 0, strokeWidth: 2 },
  buildings: { stroke: '#ff6b6b', fill: '#ff6b6b', fillOpacity: 0.6, strokeWidth: 1 },
  water: { stroke: '#45b7d1', fill: '#45b7d1', fillOpacity: 0.4, strokeWidth: 1 },
  landuse: { stroke: '#96c93d', fill: '#96c93d', fillOpacity: 0.3, strokeWidth: 1 },
  railway: { stroke: '#8D6E63', fill: 'transparent', fillOpacity: 0, strokeWidth: 3 },
  barrier: { stroke: '#A1887F', fill: 'transparent', fillOpacity: 0, strokeWidth: 1 },
};

// Mapping from _type property to layer id
const TYPE_TO_LAYER: Record<string, string> = {
  road: 'roads',
  building: 'buildings',
  water: 'water',
  landuse: 'landuse',
  railway: 'railway',
  barrier: 'barrier',
};

export function useMap(): UseMapReturn {
  const mapRef = useRef<HTMLDivElement | null>(null);
  const [map, setMap] = useState<Map | null>(null);
  const [cursorPosition, setCursorPosition] = useState<CursorPosition | null>(null);
  const [zoom, setZoom] = useState<number>(DEFAULT_ZOOM);
  const [featureCount, setFeatureCount] = useState(0);

  // Refs for vector layers, draw interaction, spawn mode
  const vectorLayersRef = useRef<Record<string, VectorLayer> | undefined>(undefined);
  const vectorSourcesRef = useRef<Record<string, VectorSource> | undefined>(undefined);
  const drawInteractionRef = useRef<unknown>(null);
  const drawSourceRef = useRef<VectorSource | null>(null);
  const dragPanRef = useRef<unknown>(null);
  const spawnLayerRef = useRef<VectorLayer | null>(null);
  const spawnListenerKeyRef = useRef<unknown>(null);
  const bboxCallbackRef = useRef<((bbox: [number, number, number, number]) => void) | null>(null);
  const spawnModeActiveRef = useRef(false);
  const spawnCallbackRef = useRef<((pos: { lat: number; lon: number }) => void) | null>(null);
  const highlightLayerRef = useRef<VectorLayer | null>(null);
  const onFeatureClick = useRef<((feat: SelectedFeatureData) => void) | null>(null);
  const cacheLayerRef = useRef<VectorLayer | null>(null);
  const cacheSourceRef = useRef<VectorSource | null>(null);

  useEffect(() => {
    if (!mapRef.current) return;

    let olMap: Map | null = null;
    let cancelled = false;

    async function initMap() {
      if (!mapRef.current) return;

      // Clean up any leftover OL instances (React strict mode double-mount)
      while (mapRef.current.firstChild) {
        mapRef.current.removeChild(mapRef.current.firstChild);
      }

      const { default: OlMap } = await import('ol/Map');
      const { default: View } = await import('ol/View');
      const { default: TileLayer } = await import('ol/layer/Tile');
      const { default: OSM } = await import('ol/source/OSM');
      const { default: OlVectorLayer } = await import('ol/layer/Vector');
      const { default: OlVectorSource } = await import('ol/source/Vector');
      const { fromLonLat, toLonLat } = await import('ol/proj');
      const { defaults: defaultControls } = await import('ol/control');
      const { Style, Fill, Stroke } = await import('ol/style');
      const { default: DragPan } = await import('ol/interaction/DragPan');

      // Abort if cleanup ran while we were importing (React strict mode)
      if (cancelled || !mapRef.current) return;

      // Remove any stale viewports from a previous mount
      while (mapRef.current.firstChild) {
        mapRef.current.removeChild(mapRef.current.firstChild);
      }

      olMap = new OlMap({
        target: mapRef.current,
        layers: [
          new TileLayer({
            source: new OSM(),
            zIndex: 0,
          }),
        ],
        view: new View({
          center: fromLonLat([DEFAULT_LON, DEFAULT_LAT]),
          zoom: DEFAULT_ZOOM,
        }),
        controls: defaultControls(),
      });

      // Create vector layers for each type
      const layers: Record<string, VectorLayer> = {};
      const sources: Record<string, VectorSource> = {};

      let zIdx = 10; // above tile layer (default zIndex 0)
      for (const [layerId, styleDef] of Object.entries(LAYER_STYLES)) {
        const source = new OlVectorSource();
        const fillColor = styleDef.fill === 'transparent'
          ? 'rgba(0,0,0,0)'
          : hexToRgba(styleDef.fill, styleDef.fillOpacity);
        const layerStyle = new Style({
          stroke: new Stroke({ color: styleDef.stroke, width: styleDef.strokeWidth }),
          fill: new Fill({ color: fillColor }),
        });
        const layer = new OlVectorLayer({
          source,
          zIndex: zIdx++,
          style: layerStyle,
          properties: { layerId },
        });
        olMap.addLayer(layer);
        layers[layerId] = layer;
        sources[layerId] = source;
      }

      // Cache areas layer — bbox rectangles for cached Overpass data
      const cacheSource = new OlVectorSource();
      const cacheLayer = new OlVectorLayer({
        source: cacheSource,
        style: new Style({
          stroke: new Stroke({ color: 'rgba(0, 120, 255, 0.4)', width: 1.5 }),
          fill: new Fill({ color: 'rgba(0, 120, 255, 0.08)' }),
        }),
        zIndex: 5, // below OSM feature layers (zIdx starts at 10)
        properties: { layerId: 'cache' },
      });
      olMap.addLayer(cacheLayer);
      cacheLayerRef.current = cacheLayer;
      cacheSourceRef.current = cacheSource;
      // Register in vectorLayersRef so setLayerVisible('cache') works
      layers['cache'] = cacheLayer;
      sources['cache'] = cacheSource;

      vectorLayersRef.current = layers;
      vectorSourcesRef.current = sources;

      // Store constructors for use in loadGeoJSON
      const { default: OlFeatureCtor } = await import('ol/Feature');
      const { default: OlLineStringCtor } = await import('ol/geom/LineString');
      const { default: OlPolygonCtor } = await import('ol/geom/Polygon');
      featureCtorsRef.current = { Feature: OlFeatureCtor, LineString: OlLineStringCtor, Polygon: OlPolygonCtor, fromLonLat };
      styleCtorsRef.current = { Style, Fill, Stroke };

      // Promote vector-layer divs above the tile layer via inline style.
      //
      // OpenLayers renders each layer into its own absolutely-positioned div inside
      // `.ol-layers`.  We need the vector layers (index > 0) to sit above the OSM
      // tile layer (index 0).  Because OL appends these divs asynchronously after
      // `setTarget`, we apply the style once the browser has had a chance to paint
      // (two animation frames are enough for OL's synchronous initial render).
      const applyLayerZIndex = () => {
        const layerDivs = mapRef.current?.querySelectorAll('.ol-layers > .ol-layer');
        layerDivs?.forEach((el, i) => {
          if (i > 0) (el as HTMLElement).style.zIndex = '10';
        });
      };
      requestAnimationFrame(() => requestAnimationFrame(applyLayerZIndex));

      // Highlight layer for selected features
      const highlightSource = new OlVectorSource();
      const highlightLayer = new OlVectorLayer({
        source: highlightSource,
        style: new Style({
          stroke: new Stroke({ color: '#ffd93d', width: 4 }),
          fill: new Fill({ color: 'rgba(255,217,61,0.2)' }),
        }),
        properties: { layerId: '_highlight' },
      });
      olMap.addLayer(highlightLayer);
      highlightLayerRef.current = highlightLayer;

      // Spawn marker layer
      const spawnSource = new OlVectorSource();
      const spawnLayer = new OlVectorLayer({
        source: spawnSource,
        properties: { layerId: '_spawn' },
      });
      olMap.addLayer(spawnLayer);
      spawnLayerRef.current = spawnLayer;

      // Draw box layer — shows the drawn rectangle
      const drawSource = new OlVectorSource();
      drawSourceRef.current = drawSource;
      const drawLayer = new OlVectorLayer({
        source: drawSource,
        style: new Style({
          stroke: new Stroke({ color: 'rgba(126,200,227,0.8)', width: 2, lineDash: [6, 4] }),
          fill: new Fill({ color: 'rgba(126,200,227,0.1)' }),
        }),
        properties: { layerId: '_draw' },
      });
      olMap.addLayer(drawLayer);

      // DragBox interaction — purpose-built for rectangular selection
      const { default: DragBox } = await import('ol/interaction/DragBox');
      const { always } = await import('ol/events/condition');
      const { default: Feature } = await import('ol/Feature');
      const { fromExtent } = await import('ol/geom/Polygon');

      const dragBox = new DragBox({
        condition: always, // no modifier key needed
        className: 'ol-dragbox',
      });
      dragBox.setActive(false);
      dragBox.on('boxend', () => {
        const extent = dragBox.getGeometry().getExtent();
        const bl = toLonLat([extent[0], extent[1]]);
        const tr = toLonLat([extent[2], extent[3]]);
        bboxCallbackRef.current?.([bl[1], bl[0], tr[1], tr[0]]);

        // Show the drawn box on the map
        drawSource.clear();
        drawSource.addFeature(new Feature(fromExtent(extent)));
      });
      olMap.addInteraction(dragBox);
      drawInteractionRef.current = dragBox;

      // Pointer move for cursor position
      olMap.on('pointermove', (evt) => {
        const [lon, lat] = toLonLat(evt.coordinate);
        setCursorPosition({ lon, lat });
      });

      olMap.on('moveend', () => {
        if (!olMap) return;
        const view = olMap.getView();
        const z = view.getZoom();
        if (z !== undefined) {
          setZoom(z);
        }
      });

      // Unified click handler — spawn mode or feature selection
      olMap.on('singleclick', async (evt) => {
        if (!olMap) return;

        // Spawn mode: place marker and call callback
        if (spawnModeActiveRef.current && spawnCallbackRef.current) {
          const { toLonLat: tll, fromLonLat: fll } = await import('ol/proj');
          const { default: Feat } = await import('ol/Feature');
          const { default: Pt } = await import('ol/geom/Point');
          const { Style: S, Fill: F, Stroke: St } = await import('ol/style');
          const CM = await import('ol/style/Circle');

          const [lon, lat] = tll(evt.coordinate);
          const spawnSrc = spawnLayerRef.current?.getSource();
          if (spawnSrc) {
            spawnSrc.clear();
            const m = new Feat({ geometry: new Pt(fll([lon, lat])), _internal: true });
            m.setStyle(new S({
              image: new CM.default({ radius: 8, fill: new F({ color: '#ffd93d' }), stroke: new St({ color: '#000', width: 2 }) }),
            }));
            spawnSrc.addFeature(m);
          }
          spawnCallbackRef.current({ lat, lon });
          return;
        }

        // Feature selection
        let found = false;
        olMap.forEachFeatureAtPixel(evt.pixel, (feature: FeatureLike) => {
          if (found) return;
          const props = feature.getProperties();
          if (props['_internal']) return;
          const geomType = feature.getGeometry()?.getType() ?? 'Unknown';
          const cleanProps: Record<string, string> = {};
          for (const [k, v] of Object.entries(props)) {
            if (k !== 'geometry' && v !== undefined && v !== null) {
              cleanProps[k] = String(v);
            }
          }
          onFeatureClick.current?.({
            properties: cleanProps,
            geometry_type: geomType,
          });
          found = true;
        });
      });

      setMap(olMap);
      setZoom(DEFAULT_ZOOM);
    }

    initMap();

    return () => {
      cancelled = true;
      if (olMap) {
        olMap.setTarget(undefined);
        olMap.dispose();
        olMap = null;
      }
    };
  }, []);

  const flyTo = useCallback(
    async (bbox: [number, number, number, number], center: [number, number]) => {
      if (!map) return;

      const { fromLonLat, transformExtent } = await import('ol/proj');

      const view = map.getView();
      const [south, west, north, east] = bbox;
      const extent = transformExtent([west, south, east, north], 'EPSG:4326', 'EPSG:3857');

      const bboxWidthDeg = east - west;
      const bboxHeightDeg = north - south;
      if (bboxWidthDeg < 0.001 && bboxHeightDeg < 0.001) {
        view.animate({
          center: fromLonLat(center),
          zoom: 16,
          duration: 600,
        });
      } else {
        view.fit(extent, {
          duration: 600,
          padding: [40, 40, 40, 40],
          maxZoom: 17,
        });
      }
    },
    [map]
  );

  // Set during initMap — guaranteed ready when map is ready
  const featureCtorsRef = useRef<{
    Feature: typeof import('ol/Feature').default;
    LineString: typeof import('ol/geom/LineString').default;
    Polygon: typeof import('ol/geom/Polygon').default;
    fromLonLat: typeof import('ol/proj').fromLonLat;
  } | null>(null);
  const geojsonFormatRef = useRef<import('ol/format/GeoJSON').default | null>(null);
  const styleCtorsRef = useRef<{
    Style: typeof import('ol/style').Style;
    Fill: typeof import('ol/style').Fill;
    Stroke: typeof import('ol/style').Stroke;
  } | null>(null);

  const loadGeoJSON = useCallback(
    (geojson: FeatureCollection): Record<string, number> => {
      if (!map || !vectorSourcesRef.current || !featureCtorsRef.current) return {};

      const format = geojsonFormatRef.current;

      // Clear existing features
      for (const source of Object.values(vectorSourcesRef.current)) {
        source.clear();
      }

      const counts: Record<string, number> = {
        roads: 0,
        buildings: 0,
        water: 0,
        landuse: 0,
        railway: 0,
        barrier: 0,
      };

      // Group features by layer type, then batch-parse per layer
      const byLayer: Record<string, typeof geojson.features> = {};
      for (const feature of geojson.features) {
        const featureType = (feature.properties?.['_type'] as string) ?? 'other';
        const layerId = TYPE_TO_LAYER[featureType];
        if (!layerId) continue;
        if (!byLayer[layerId]) byLayer[layerId] = [];
        byLayer[layerId].push(feature);
      }

      if (!featureCtorsRef.current || !styleCtorsRef.current) return counts;
      const { Feature: FC, LineString: LS, Polygon: PG, fromLonLat: fll } = featureCtorsRef.current;
      const { Style: S, Fill: F, Stroke: St } = styleCtorsRef.current;

      for (const [layerId, features] of Object.entries(byLayer)) {
        const source = vectorSourcesRef.current[layerId];
        if (!source) continue;

        const sd = LAYER_STYLES[layerId];
        const featureStyle = sd ? new S({
          stroke: new St({ color: sd.stroke, width: sd.strokeWidth }),
          fill: new F({ color: sd.fill === 'transparent' ? 'rgba(0,0,0,0)' : hexToRgba(sd.fill, sd.fillOpacity) }),
        }) : undefined;

        try {
          const olFeatures: import('ol/Feature').default[] = [];
          for (const gj of features) {
            if (!gj.geometry) continue;
            let geom: import('ol/geom/Geometry').default | null = null;
            const coords = (gj.geometry as GeoJSON.LineString | GeoJSON.Polygon).coordinates;

            if (gj.geometry.type === 'LineString') {
              const projected = (coords as number[][]).map(c => fll(c as [number, number]));
              geom = new LS(projected);
            } else if (gj.geometry.type === 'Polygon') {
              const rings = (coords as number[][][]).map(ring =>
                ring.map(c => fll(c as [number, number]))
              );
              geom = new PG(rings);
            }

            if (geom) {
              const f = new FC({ geometry: geom });
              if (featureStyle) f.setStyle(featureStyle);
              if (gj.properties) {
                for (const [k, v] of Object.entries(gj.properties)) {
                  f.set(k, v);
                }
              }
              olFeatures.push(f);
            }
          }

          source.addFeatures(olFeatures);
          counts[layerId] = olFeatures.length;
        } catch (err) {
          console.error(`Failed to create ${layerId} features:`, err);
        }
      }

      // Force re-render of all sources
      for (const source of Object.values(vectorSourcesRef.current)) {
        source.changed();
      }
      // Force vector layers to re-render
      if (vectorLayersRef.current) {
        for (const layer of Object.values(vectorLayersRef.current)) {
          layer.changed();
        }
      }
      map.render();
      map.renderSync();

      let total = 0;
      for (const c of Object.values(counts)) total += c;
      setFeatureCount(total);

      // Fit map to show all loaded features
      if (total > 0) {
        const allSources = Object.values(vectorSourcesRef.current);
        let extent: import('ol/extent').Extent | null = null;
        for (const source of allSources) {
          const srcExtent = source.getExtent();
          if (srcExtent && isFinite(srcExtent[0])) {
            if (!extent) {
              extent = [...srcExtent];
            } else {
              extent[0] = Math.min(extent[0], srcExtent[0]);
              extent[1] = Math.min(extent[1], srcExtent[1]);
              extent[2] = Math.max(extent[2], srcExtent[2]);
              extent[3] = Math.max(extent[3], srcExtent[3]);
            }
          }
        }
        if (extent) {
          map.getView().fit(extent, { padding: [40, 40, 40, 40], maxZoom: 17, duration: 600 });
        }
      }

      return counts;
    },
    [map]
  );

  const setLayerVisible = useCallback(
    (layerId: string, visible: boolean) => {
      if (!vectorLayersRef.current?.[layerId]) return;
      vectorLayersRef.current[layerId].setVisible(visible);
    },
    []
  );

  const enableBboxDraw = useCallback(
    (onDone: (bbox: [number, number, number, number]) => void) => {
      if (!map) return;
      const dragBox = drawInteractionRef.current as import('ol/interaction/DragBox').default | null;
      if (!dragBox) return;

      bboxCallbackRef.current = onDone;
      dragBox.setActive(true);

      // Disable all other interactions so drag goes to DragBox
      for (const interaction of map.getInteractions().getArray()) {
        if (interaction !== dragBox) {
          interaction.setActive(false);
        }
      }
    },
    [map]
  );

  const disableBboxDraw = useCallback(() => {
    if (!map) return;
    const dragBox = drawInteractionRef.current as import('ol/interaction/DragBox').default | null;
    if (dragBox) dragBox.setActive(false);
    bboxCallbackRef.current = null;

    // Re-enable all other interactions
    for (const interaction of map.getInteractions().getArray()) {
      if (interaction !== dragBox) {
        interaction.setActive(true);
      }
    }
  }, [map]);

  const enableSpawnMode = useCallback(
    (onDone: (pos: { lat: number; lon: number }) => void) => {
      spawnModeActiveRef.current = true;
      spawnCallbackRef.current = onDone;
    },
    []
  );

  const disableSpawnMode = useCallback(() => {
    spawnModeActiveRef.current = false;
    spawnCallbackRef.current = null;
  }, []);

  const setSelectedFeatureById = useCallback(
    (_id: string | number | null) => {
      // Clear highlight
      const highlightSource = highlightLayerRef.current?.getSource();
      if (highlightSource) {
        highlightSource.clear();
      }
      // For now, highlight is handled via the feature click flow
    },
    []
  );

  const loadCacheAreas = useCallback((entries: CacheEntry[]) => {
    const source = cacheSourceRef.current;
    if (!source) return;
    void (async () => {
      const { default: Feature } = await import('ol/Feature');
      const { fromExtent } = await import('ol/geom/Polygon');
      const { fromLonLat } = await import('ol/proj');
      source.clear();
      for (const entry of entries) {
        const [s, w, n, e] = entry.bbox;
        const sw = fromLonLat([w, s]);
        const ne = fromLonLat([e, n]);
        const extent: [number, number, number, number] = [sw[0], sw[1], ne[0], ne[1]];
        const feat = new Feature({
          geometry: fromExtent(extent),
          _cacheEntry: true,
          _cacheBbox: entry.bbox,
          created_at: entry.created_at,
        });
        source.addFeature(feat);
      }
    })();
  }, []);

  return {
    mapRef,
    map,
    cursorPosition,
    zoom,
    featureCount,
    flyTo,
    loadGeoJSON,
    setLayerVisible,
    enableBboxDraw,
    disableBboxDraw,
    enableSpawnMode,
    disableSpawnMode,
    setSelectedFeatureById,
    onFeatureClick,
    loadCacheAreas,
  };
}

function hexToRgba(hex: string, alpha: number): string {
  const r = parseInt(hex.slice(1, 3), 16);
  const g = parseInt(hex.slice(3, 5), 16);
  const b = parseInt(hex.slice(5, 7), 16);
  return `rgba(${r},${g},${b},${alpha})`;
}
