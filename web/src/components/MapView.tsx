'use client';

import 'ol/ol.css';
import { useEffect, useState, useCallback, useRef, useMemo } from 'react';
import { useMap, type SelectedFeatureData, type CacheEntry } from '@/hooks/useMap';
import type { FeatureCollection } from 'geojson';

interface MapViewProps {
  onFlyTo?: (flyTo: (bbox: [number, number, number, number], center: [number, number]) => Promise<void>) => void;
  geojsonData?: FeatureCollection | null;
  layerVisibility?: Record<string, boolean>;
  onBboxDrawn?: (bbox: [number, number, number, number]) => void;
  onFeatureSelect?: (feature: SelectedFeatureData) => void;
  onSpawnSet?: (pos: { lat: number; lon: number }) => void;
  onLayerCounts?: (counts: Record<string, number>) => void;
  spawnMode?: boolean;
  /** Increment this to trigger a cache-areas refresh */
  refreshCacheTrigger?: number;
  /** MC world origin (center of bbox) for coordinate conversion */
  mcOrigin?: { lat: number; lon: number } | null;
  /** MC conversion params for coordinate conversion */
  mcParams?: { scale: number; seaLevel: number };
}

export function MapView({
  onFlyTo,
  geojsonData,
  layerVisibility,
  onBboxDrawn,
  onFeatureSelect,
  onSpawnSet,
  onLayerCounts,
  spawnMode,
  refreshCacheTrigger,
  mcOrigin,
  mcParams,
}: MapViewProps) {
  const {
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
    onFeatureClick,
    loadCacheAreas,
  } = useMap();

  const [drawActive, setDrawActive] = useState(false);
  const [mcCopied, setMcCopied] = useState(false);

  // Refs for context menu access (avoids stale closures in useEffect)
  const mcOriginRef = useRef(mcOrigin);
  mcOriginRef.current = mcOrigin;
  const mcParamsRef = useRef(mcParams);
  mcParamsRef.current = mcParams;

  // Compute MC block coordinates from cursor position
  const mcCoords = useMemo(() => {
    if (!cursorPosition || !mcOrigin || !mcParams) return null;
    const METRES_PER_DEG_LAT = 111_320;
    const metresPerDegLon = METRES_PER_DEG_LAT * Math.cos(mcOrigin.lat * Math.PI / 180);
    const x = Math.round((cursorPosition.lon - mcOrigin.lon) * metresPerDegLon / mcParams.scale);
    const z = Math.round(-(cursorPosition.lat - mcOrigin.lat) * METRES_PER_DEG_LAT / mcParams.scale);
    const y = mcParams.seaLevel + 1;
    return { x, y, z };
  }, [cursorPosition, mcOrigin, mcParams]);

  const handleMcCopy = useCallback(() => {
    if (!mcCoords) return;
    const cmd = `/tp @s ${mcCoords.x} ${mcCoords.y} ${mcCoords.z}`;
    navigator.clipboard.writeText(cmd).then(() => {
      setMcCopied(true);
      setTimeout(() => setMcCopied(false), 1200);
    });
  }, [mcCoords]);

  const [cacheTooltip, setCacheTooltip] = useState<{
    x: number;
    y: number;
    text: string;
  } | null>(null);

  // Expose flyTo to parent via callback ref
  useEffect(() => {
    if (onFlyTo) {
      onFlyTo(flyTo);
    }
  }, [flyTo, onFlyTo]);

  // Wire feature click callback
  useEffect(() => {
    onFeatureClick.current = onFeatureSelect ?? null;
  }, [onFeatureSelect, onFeatureClick]);

  // Load GeoJSON when data changes
  useEffect(() => {
    if (!map || !geojsonData) return;
    const counts = loadGeoJSON(geojsonData);
    onLayerCounts?.(counts);
  }, [map, geojsonData, loadGeoJSON, onLayerCounts]);

  // Sync layer visibility
  useEffect(() => {
    if (!layerVisibility) return;
    for (const [layerId, visible] of Object.entries(layerVisibility)) {
      setLayerVisible(layerId, visible);
    }
  }, [layerVisibility, setLayerVisible]);

  // Spawn mode
  useEffect(() => {
    if (!map) return;
    if (spawnMode && onSpawnSet) {
      enableSpawnMode(onSpawnSet);
      return () => { disableSpawnMode(); };
    } else {
      disableSpawnMode();
    }
  }, [map, spawnMode, onSpawnSet, enableSpawnMode, disableSpawnMode]);

  // Fetch cache areas on mount and whenever refreshCacheTrigger changes
  useEffect(() => {
    if (!map) return;
    void (async () => {
      try {
        const res = await fetch('/api/cache');
        if (!res.ok) return;
        const entries = (await res.json()) as CacheEntry[];
        loadCacheAreas(entries);
        onLayerCounts?.({ cache: entries.length });
      } catch {
        // silent fail — cache overlay is non-essential
      }
    })();
  }, [map, refreshCacheTrigger, loadCacheAreas, onLayerCounts]);

  // Click on a cached area to adopt its bbox
  useEffect(() => {
    if (!map) return;

    const handleClick = (evt: import('ol/MapBrowserEvent').default) => {
      const pixel = evt.pixel as [number, number];
      let matched = false;
      map.forEachFeatureAtPixel(
        pixel,
        (feature) => {
          if (matched) return;
          const props = feature.getProperties() as Record<string, unknown>;
          if (props['_cacheEntry'] && props['_cacheBbox']) {
            matched = true;
            const bbox = props['_cacheBbox'] as [number, number, number, number];
            onBboxDrawn?.(bbox);
          }
        },
        {
          hitTolerance: 2,
          layerFilter: (layer) =>
            (layer.getProperties() as Record<string, unknown>)['layerId'] === 'cache',
        }
      );
    };

    map.on('singleclick', handleClick);
    return () => {
      map.un('singleclick', handleClick);
    };
  }, [map, onBboxDrawn]);

  // Hover tooltip for cache area features
  useEffect(() => {
    if (!map) return;

    // OL's pointermove event provides `pixel` typed as Pixel (number[]).
    // Use the non-generic base MapBrowserEvent to match OL's on() overload signature.
    const handlePointerMove = (evt: import('ol/MapBrowserEvent').default) => {
      const pixel = evt.pixel as [number, number];
      let found = false;
      map.forEachFeatureAtPixel(
        pixel,
        (feature) => {
          if (found) return;
          const props = feature.getProperties() as Record<string, unknown>;
          if (props['_cacheEntry']) {
            found = true;
            const createdAt = props['created_at'] as string | undefined;
            if (createdAt) {
              const diffMs = Date.now() - new Date(createdAt).getTime();
              const diffH = Math.floor(diffMs / 3_600_000);
              const diffM = Math.floor((diffMs % 3_600_000) / 60_000);
              const age = diffH > 0 ? `${diffH}h ago` : diffM < 1 ? 'just now' : `${diffM}m ago`;
              const text = `Cached ${age} — click to use bbox`;
              setCacheTooltip({ x: pixel[0], y: pixel[1], text });
            }
          }
        },
        {
          hitTolerance: 0,
          layerFilter: (layer) => (layer.getProperties() as Record<string, unknown>)['layerId'] === 'cache',
        }
      );
      if (!found) setCacheTooltip(null);
      // Change cursor when hovering over cache areas
      const target = map.getTargetElement();
      if (target) {
        (target as HTMLElement).style.cursor = found ? 'pointer' : '';
      }
    };

    map.on('pointermove', handlePointerMove);
    return () => {
      map.un('pointermove', handlePointerMove);
      const target = map.getTargetElement();
      if (target) {
        (target as HTMLElement).style.cursor = '';
      }
    };
  }, [map]);

  // Draw box via raw mouse events on an overlay div — bypasses OL interactions entirely
  const drawOverlayRef = useRef<HTMLDivElement | null>(null);
  const drawStartRef = useRef<{ x: number; y: number } | null>(null);
  const drawRectRef = useRef<HTMLDivElement | null>(null);

  // Store start pixel relative to the overlay (same position as map)
  const drawStartPixelRef = useRef<[number, number] | null>(null);

  const handleDrawMouseDown = useCallback((e: React.MouseEvent) => {
    if (!drawActive) return;
    e.preventDefault();
    e.stopPropagation();
    // nativeEvent.offsetX/Y is relative to the event target (the overlay div)
    drawStartPixelRef.current = [e.nativeEvent.offsetX, e.nativeEvent.offsetY];
    drawStartRef.current = { x: e.clientX, y: e.clientY };

    const rect = document.createElement('div');
    rect.style.cssText = `
      position: fixed; border: 2px dashed rgba(126,200,227,0.8);
      background: rgba(126,200,227,0.1); pointer-events: none; z-index: 100;
    `;
    document.body.appendChild(rect);
    drawRectRef.current = rect;
  }, [drawActive]);

  const handleDrawMouseMove = useCallback((e: React.MouseEvent) => {
    if (!drawStartRef.current || !drawRectRef.current) return;
    e.preventDefault();
    const { x: sx, y: sy } = drawStartRef.current;
    const rect = drawRectRef.current;
    rect.style.left = `${Math.min(sx, e.clientX)}px`;
    rect.style.top = `${Math.min(sy, e.clientY)}px`;
    rect.style.width = `${Math.abs(e.clientX - sx)}px`;
    rect.style.height = `${Math.abs(e.clientY - sy)}px`;
  }, []);

  const handleDrawMouseUp = useCallback((e: React.MouseEvent) => {
    if (!drawStartPixelRef.current || !map) return;
    const startPixel = drawStartPixelRef.current;
    const endPixel: [number, number] = [e.nativeEvent.offsetX, e.nativeEvent.offsetY];
    drawStartPixelRef.current = null;
    drawStartRef.current = null;
    if (drawRectRef.current) {
      drawRectRef.current.remove();
      drawRectRef.current = null;
    }

    // Skip if too small (accidental click)
    if (Math.abs(endPixel[0] - startPixel[0]) < 5 && Math.abs(endPixel[1] - startPixel[1]) < 5) {
      setDrawActive(false);
      return;
    }

    const coord1 = map.getCoordinateFromPixel(startPixel);
    const coord2 = map.getCoordinateFromPixel(endPixel);
    if (!coord1 || !coord2) return;

    void (async () => {
      const { toLonLat } = await import('ol/proj');
      const [lon1, lat1] = toLonLat(coord1);
      const [lon2, lat2] = toLonLat(coord2);
      const south = Math.min(lat1, lat2);
      const north = Math.max(lat1, lat2);
      const west = Math.min(lon1, lon2);
      const east = Math.max(lon1, lon2);
      onBboxDrawn?.([south, west, north, east]);
    })();

    setDrawActive(false);
  }, [map, onBboxDrawn]);

  const handleDrawToggle = useCallback(() => {
    setDrawActive((prev) => !prev);
  }, []);

  // Context menu for spawn
  useEffect(() => {
    const el = mapRef.current;
    if (!el) return;

    const handleContextMenu = (e: MouseEvent) => {
      e.preventDefault();

      // Remove existing menu if any
      const existing = document.getElementById('osm-ctx-menu');
      if (existing) existing.remove();

      const menu = document.createElement('div');
      menu.id = 'osm-ctx-menu';
      menu.style.cssText = `
        position: fixed;
        left: ${e.clientX}px;
        top: ${e.clientY}px;
        background: var(--bg-elevated, #151822);
        border: 1px solid rgba(255,255,255,0.1);
        border-radius: 8px;
        padding: 4px;
        z-index: 1000;
        box-shadow: 0 8px 32px rgba(0,0,0,0.7), 0 2px 8px rgba(0,0,0,0.5);
        min-width: 164px;
      `;

      const item = document.createElement('button');
      item.textContent = 'Set spawn here';
      item.style.cssText = `
        display: block;
        width: 100%;
        text-align: left;
        padding: 7px 12px;
        background: transparent;
        color: var(--accent-gold, #e8b84d);
        border: none;
        font-size: 11.5px;
        cursor: pointer;
        border-radius: 5px;
        font-family: "DM Sans", sans-serif;
        font-weight: 500;
        letter-spacing: 0.01em;
      `;
      item.onmouseenter = () => { item.style.background = 'rgba(232,184,77,0.1)'; };
      item.onmouseleave = () => { item.style.background = 'transparent'; };
      item.onclick = () => {
        menu.remove();
        if (!map || !onSpawnSet) return;
        void (async () => {
          const { toLonLat, fromLonLat } = await import('ol/proj');
          const pixel = map.getEventPixel(e);
          const coord = map.getCoordinateFromPixel(pixel);
          if (!coord) return;
          const [lon, lat] = toLonLat(coord);

          // Place marker
          const { default: Feature } = await import('ol/Feature');
          const { default: Point } = await import('ol/geom/Point');
          const { Style, Fill, Stroke } = await import('ol/style');
          const CircleStyleMod = await import('ol/style/Circle');
          const CircleStyle = CircleStyleMod.default;

          const spawnSource = map.getLayers().getArray()
            .find((l) => l.get('layerId') === '_spawn');
          if (spawnSource && 'getSource' in spawnSource) {
            const src = (spawnSource as import('ol/layer/Vector').default).getSource();
            if (src) {
              src.clear();
              const marker = new Feature({
                geometry: new Point(fromLonLat([lon, lat])),
                _internal: true,
              });
              marker.setStyle(
                new Style({
                  image: new CircleStyle({
                    radius: 8,
                    fill: new Fill({ color: '#ffd93d' }),
                    stroke: new Stroke({ color: '#000', width: 2 }),
                  }),
                })
              );
              src.addFeature(marker);
            }
          }
          onSpawnSet({ lat, lon });
        })();
      };

      menu.appendChild(item);

      // "Copy teleport command" — only when MC origin is available
      const origin = mcOriginRef.current;
      const mcp = mcParamsRef.current;
      if (origin && mcp && map) {
        const pixel = map.getEventPixel(e);
        const coord = map.getCoordinateFromPixel(pixel);
        if (coord) {
          void (async () => {
            const { toLonLat } = await import('ol/proj');
            const [lon, lat] = toLonLat(coord);
            const METRES_PER_DEG_LAT = 111_320;
            const metresPerDegLon = METRES_PER_DEG_LAT * Math.cos(origin.lat * Math.PI / 180);
            const x = Math.round((lon - origin.lon) * metresPerDegLon / mcp.scale);
            const z = Math.round(-(lat - origin.lat) * METRES_PER_DEG_LAT / mcp.scale);
            const y = mcp.seaLevel + 1;
            const cmd = `/tp @s ${x} ${y} ${z}`;

            const tpItem = document.createElement('button');
            tpItem.textContent = `Copy  ${cmd}`;
            tpItem.style.cssText = `
              display: block;
              width: 100%;
              text-align: left;
              padding: 7px 12px;
              background: transparent;
              color: var(--accent-cyan, #56c8d8);
              border: none;
              font-size: 11.5px;
              cursor: pointer;
              border-radius: 5px;
              font-family: "DM Sans", sans-serif;
              font-weight: 500;
              letter-spacing: 0.01em;
            `;
            tpItem.onmouseenter = () => { tpItem.style.background = 'rgba(86,200,216,0.1)'; };
            tpItem.onmouseleave = () => { tpItem.style.background = 'transparent'; };
            tpItem.onclick = () => {
              navigator.clipboard.writeText(cmd).then(() => {
                tpItem.textContent = 'Copied!';
                tpItem.style.color = 'var(--accent-green, #4CAF50)';
                setTimeout(() => menu.remove(), 600);
              });
            };
            menu.appendChild(tpItem);
          })();
        }
      }

      document.body.appendChild(menu);

      // Remove on click outside
      const removeMenu = (ev: MouseEvent) => {
        if (!menu.contains(ev.target as Node)) {
          menu.remove();
          document.removeEventListener('mousedown', removeMenu);
        }
      };
      setTimeout(() => document.addEventListener('mousedown', removeMenu), 0);
    };

    el.addEventListener('contextmenu', handleContextMenu);
    return () => {
      el.removeEventListener('contextmenu', handleContextMenu);
      const existing = document.getElementById('osm-ctx-menu');
      if (existing) existing.remove();
    };
  }, [map, mapRef, onSpawnSet]);

  return (
    <div className="relative flex-1 h-full w-full">
      {/* Map container */}
      <div
        ref={mapRef}
        className="absolute inset-0"
        style={{ width: '100%', height: '100%' }}
      />

      {/* Transparent overlay that captures mouse events during draw mode */}
      {drawActive && (
        <div
          ref={drawOverlayRef}
          className="absolute inset-0 z-10"
          style={{ cursor: 'crosshair' }}
          onMouseDown={handleDrawMouseDown}
          onMouseMove={handleDrawMouseMove}
          onMouseUp={handleDrawMouseUp}
        />
      )}

      {/* Draw Box button — bottom-left */}
      <button
        onClick={handleDrawToggle}
        className="absolute bottom-10 left-3 z-10 flex items-center gap-1.5 rounded-full px-3.5 py-1.5 text-[11px] font-medium tracking-wide transition-all duration-150"
        style={{
          background: drawActive
            ? 'rgba(86,200,216,0.18)'
            : 'rgba(8,9,13,0.88)',
          color: drawActive ? 'var(--accent-cyan)' : 'var(--text-secondary)',
          border: drawActive
            ? '1px solid rgba(86,200,216,0.5)'
            : '1px solid var(--border-medium)',
          backdropFilter: 'blur(6px)',
          cursor: 'pointer',
          boxShadow: drawActive
            ? '0 0 10px rgba(86,200,216,0.15)'
            : '0 2px 8px rgba(0,0,0,0.4)',
          fontFamily: '"DM Sans", sans-serif',
        }}
      >
        <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
          <rect x="3" y="3" width="18" height="18" rx="2" />
        </svg>
        {drawActive ? 'Drawing…' : 'Draw Box'}
      </button>

      {/* Cache area hover tooltip */}
      {cacheTooltip && (
        <div
          style={{
            position: 'absolute',
            left: cacheTooltip.x + 12,
            top: cacheTooltip.y - 8,
            background: 'rgba(0,0,0,0.75)',
            color: '#fff',
            padding: '3px 8px',
            borderRadius: 4,
            fontSize: 12,
            pointerEvents: 'none',
            zIndex: 200,
            whiteSpace: 'nowrap',
          }}
        >
          {cacheTooltip.text}
        </div>
      )}

      {/* Status bar */}
      <div
        className="absolute bottom-0 left-0 right-0 z-10 flex items-center gap-5 px-4 py-1.5 text-[11px]"
        style={{
          background: 'rgba(8,9,13,0.9)',
          backdropFilter: 'blur(8px)',
          borderTop: '1px solid var(--border-subtle)',
          letterSpacing: '0.04em',
          fontFamily: "'JetBrains Mono', monospace",
        }}
      >
        {/* Lat/Lon */}
        <span style={{ color: 'var(--accent-cyan)' }}>
          {cursorPosition
            ? `${cursorPosition.lat.toFixed(6)}° N  ${cursorPosition.lon.toFixed(6)}° E`
            : '—  lat / lon'}
        </span>

        {/* MC Teleport Coords */}
        {mcCoords && (
          <span
            onClick={handleMcCopy}
            title="Click to copy teleport command"
            style={{
              cursor: 'pointer',
              color: mcCopied ? 'var(--accent-green, #4CAF50)' : 'var(--accent-orange, #FF9800)',
              transition: 'color 0.2s',
            }}
          >
            {mcCopied
              ? 'Copied!'
              : `mc /tp @s ${mcCoords.x} ${mcCoords.y} ${mcCoords.z}`}
          </span>
        )}

        {/* Zoom */}
        <span>
          <span style={{ color: 'var(--accent-purple)' }}>zoom</span>
          <span style={{ color: 'var(--text-muted)' }}>&nbsp;</span>
          <span style={{ color: 'var(--text-primary)' }}>{zoom.toFixed(2)}</span>
        </span>

        {/* Feature count */}
        <span>
          <span style={{ color: 'var(--accent-gold)' }}>features</span>
          <span style={{ color: 'var(--text-muted)' }}>&nbsp;</span>
          <span style={{ color: 'var(--text-primary)' }}>{featureCount}</span>
        </span>
      </div>
    </div>
  );
}
