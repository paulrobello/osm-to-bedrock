'use client';

import dynamic from 'next/dynamic';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { FeatureCollection } from 'geojson';
import { Sidebar } from '@/components/Sidebar';
import { LayerPanel, type LayerConfig } from '@/components/LayerPanel';
import { SearchBar } from '@/components/SearchBar';
import { DataSourcePanel } from '@/components/DataSourcePanel';
import { FeatureInspector } from '@/components/FeatureInspector';
import { ExportPanel } from '@/components/ExportPanel';
import { HistoryPanel } from '@/components/HistoryPanel';
import { MapLegend } from '@/components/MapLegend';
import { ShortcutHelp } from '@/components/ShortcutHelp';
import type { SelectedFeatureData } from '@/hooks/useMap';
import { useKeyboardShortcuts } from '@/hooks/useKeyboardShortcuts';
import { useConversionHistory } from '@/hooks/useConversionHistory';
import { usePreview } from '@/hooks/usePreview';
import { estimateWorldSize } from '@/lib/geo';
import { useTheme } from '@/hooks/useTheme';
import { useMediaQuery } from '@/hooks/useMediaQuery';
import type { FeatureFilter } from '@/lib/overpass';
import { defaultFilter } from '@/lib/overpass';
import { Menu } from 'lucide-react';

const MapView = dynamic(
  () => import('@/components/MapView').then((m) => m.MapView),
  { ssr: false }
);

const ThreePreview = dynamic(
  () => import('@/components/ThreePreview').then((m) => m.ThreePreview),
  { ssr: false }
);

const DEFAULT_LAYERS: LayerConfig[] = [
  { id: 'roads',     name: 'Roads',         color: '#9098b0',             count: 0, visible: true },
  { id: 'buildings', name: 'Buildings',     color: '#e85d5d',             count: 0, visible: true },
  { id: 'water',     name: 'Water',         color: '#4db8d4',             count: 0, visible: true },
  { id: 'landuse',   name: 'Landuse',       color: '#6bc95d',             count: 0, visible: true },
  { id: 'railway',   name: 'Railway',       color: '#8D6E63',             count: 0, visible: true },
  { id: 'barrier',   name: 'Barriers',      color: '#A1887F',             count: 0, visible: true },
  { id: 'cache',     name: 'Cached Areas',  color: 'rgba(0,120,255,0.5)', count: 0, visible: true },
  { id: 'preview',   name: 'Block Preview', color: '#FFB74D',             count: 0, visible: false },
];

interface SpawnPoint {
  lat: number;
  lon: number;
}

export default function Home() {
  // Map fly-to ref
  const flyToRef = useRef<((bbox: [number, number, number, number], center: [number, number]) => Promise<void>) | null>(null);

  // State
  const [layers, setLayers] = useState<LayerConfig[]>(DEFAULT_LAYERS);
  const [selectedFeature, setSelectedFeature] = useState<SelectedFeatureData | null>(null);
  const [bbox, setBbox] = useState<[number, number, number, number] | null>(null);
  const [spawnPoint, setSpawnPoint] = useState<SpawnPoint | null>(null);
  const [geojsonData, setGeojsonData] = useState<FeatureCollection | null>(null);
  const [sourceFile, setSourceFile] = useState<File | null>(null);
  const [loading, setLoading] = useState(false);
  const [spawnMode, setSpawnMode] = useState(false);
  const [showPreview, setShowPreview] = useState(false);
  const [featureFilter, setFeatureFilter] = useState<FeatureFilter>(defaultFilter);
  const [overpassUrl, setOverpassUrl] = useState<string>('https://overpass-api.de/api/interpreter');
  const [refreshCacheTrigger, setRefreshCacheTrigger] = useState(0);
  const [overtureAvailable, setOvertureAvailable] = useState(false);
  const [overtureSettings, setOvertureSettings] = useState<{
    enabled: boolean;
    themes: string[];
    priority: Record<string, string>;
  }>({ enabled: false, themes: [], priority: {} });
  const [mcParams, setMcParams] = useState<{ scale: number; seaLevel: number; surfaceThickness: number }>({ scale: 1.0, seaLevel: 65, surfaceThickness: 4 });

  // Compute MC world origin from bbox center
  const mcOrigin = useMemo(() => {
    if (!bbox) return null;
    const [south, west, north, east] = bbox;
    return { lat: (south + north) / 2, lon: (west + east) / 2 };
  }, [bbox]);

  // Gate 3D preview on world size — lightweight surface preview handles large
  // areas but Overpass fetch is slow beyond ~500K chunks.
  const previewTooLarge = useMemo(() => {
    if (!bbox) return false;
    const [south, west, north, east] = bbox;
    const { chunks } = estimateWorldSize([west, south, east, north], mcParams.scale, mcParams.surfaceThickness);
    return chunks > 500_000;
  }, [bbox, mcParams.scale, mcParams.surfaceThickness]);

  // Detect Overture CLI availability via health endpoint
  useEffect(() => {
    fetch('/api/health')
      .then((r) => r.json())
      .then((data: { overture_available?: boolean }) => {
        setOvertureAvailable(data.overture_available ?? false);
      })
      .catch(() => {});
  }, []);

  const preview = usePreview();
  const { theme, toggleTheme } = useTheme();
  const isDesktop = useMediaQuery('(min-width: 768px)');
  const [sidebarOpen, setSidebarOpen] = useState(true);

  const { history, addEntry } = useConversionHistory();

  const { showHelp, setShowHelp } = useKeyboardShortcuts({
    cancelMode: () => {
      setSpawnMode(false);
    },
  });

  // Derive preview layer count from preview state rather than syncing via effect
  const displayLayers = useMemo(() => layers.map((l) => {
    if (l.id !== 'preview') return l;
    if (preview.state === 'ready') return { ...l, count: preview.blocks.length };
    if (preview.state === 'idle') return { ...l, count: 0, visible: false };
    return l;
  }), [layers, preview.state, preview.blocks.length]);

  // ── Handlers ──────────────────────────────────────────────────────────────

  function handleToggle(id: string) {
    if (id === 'preview') {
      const willShow = !showPreview;
      setShowPreview(willShow);
      if (willShow && preview.state !== 'ready') {
        if (sourceFile) {
          void preview.generatePreview(sourceFile, {});
        } else if (bbox) {
          void preview.generatePreviewFromBbox(bbox);
        }
      }
    }
    setLayers((prev) =>
      prev.map((l) => (l.id === id ? { ...l, visible: !l.visible } : l))
    );
  }

  function handleFlyToReady(
    flyTo: (bbox: [number, number, number, number], center: [number, number]) => Promise<void>
  ) {
    flyToRef.current = flyTo;
  }

  function handleSearchSelect(
    bboxVal: [number, number, number, number],
    center: [number, number]
  ) {
    flyToRef.current?.(bboxVal, center);
  }

  function handleBboxDrawn(drawnBbox: [number, number, number, number]) {
    setBbox(drawnBbox);
  }

  const handleFeatureSelect = useCallback((feat: SelectedFeatureData) => {
    setSelectedFeature(feat);
  }, []);

  const handleSpawnSet = useCallback((pos: { lat: number; lon: number }) => {
    // Store geographic coordinates — the Rust converter maps them to block coords using
    // the same CoordConverter (equirectangular projection) it uses for all OSM features.
    setSpawnPoint({ lat: pos.lat, lon: pos.lon });
    setSpawnMode(false);
  }, []);

  const handleDataLoaded = useCallback((geojson: FeatureCollection) => {
    setGeojsonData(geojson);
  }, []);

  const handleFileUploaded = useCallback((files: File[]) => {
    // Store the first file for conversion (Rust API handles one file at a time)
    if (files.length > 0) {
      setSourceFile(files[0]);
    }
  }, []);

  const handleLayerCounts = useCallback((counts: Record<string, number>) => {
    setLayers((prev) =>
      prev.map((l) => ({
        ...l,
        count: counts[l.id] ?? l.count,
      }))
    );
  }, []);

  // Build layer visibility map for MapView
  const layerVisibility: Record<string, boolean> = {};
  for (const l of displayLayers) {
    layerVisibility[l.id] = l.visible;
  }

  const sidebarContent = (
    <>
      {/* Search bar */}
      <div className="px-3 py-3" style={{ borderBottom: '1px solid var(--border-subtle)' }}>
        <SearchBar onSelect={handleSearchSelect} />
      </div>

      {/* Data source panel */}
      <div className="px-3 py-3" style={{ borderBottom: '1px solid var(--border-subtle)' }}>
        <DataSourcePanel
          bbox={bbox}
          onDataLoaded={handleDataLoaded}
          onFileUploaded={handleFileUploaded}
          loading={loading}
          onLoadingChange={setLoading}
          featureFilter={featureFilter}
          onOverpassUrlChange={setOverpassUrl}
          overtureAvailable={overtureAvailable}
          onOvertureSettingsChange={setOvertureSettings}
        />
      </div>

      {/* Layer panel */}
      <LayerPanel layers={displayLayers} onToggle={handleToggle} />

      {/* 3D Preview toggle */}
      <div className="px-3 py-3" style={{ borderBottom: '1px solid var(--border-subtle)' }}>
        <div className="flex items-center justify-between">
          <span className="text-xs font-semibold uppercase tracking-wider" style={{ color: 'var(--text-secondary)' }}>
            3D Preview
          </span>
          <div className="flex items-center gap-2">
            {preview.state === 'loading' && (
              <span className="text-xs" style={{ color: 'var(--text-secondary)' }}>Loading...</span>
            )}
            {preview.state === 'error' && (
              <span className="text-xs" style={{ color: '#F44336' }}>{preview.error || 'Error'}</span>
            )}
            {previewTooLarge && !sourceFile && (
              <span className="text-xs" style={{ color: 'var(--text-muted)' }}>Area too large</span>
            )}
            <button
              className="rounded px-2 py-1 text-xs font-medium transition-colors"
              style={{
                background: showPreview ? 'var(--accent)' : 'var(--bg-card)',
                color: showPreview ? '#fff' : 'var(--text-primary)',
                border: '1px solid var(--border-subtle)',
                opacity: (!sourceFile && !bbox) || preview.state === 'loading' || (previewTooLarge && !sourceFile) ? 0.5 : 1,
                cursor: (!sourceFile && !bbox) || preview.state === 'loading' || (previewTooLarge && !sourceFile) ? 'not-allowed' : 'pointer',
              }}
              disabled={(!sourceFile && !bbox) || preview.state === 'loading' || (previewTooLarge && !sourceFile)}
              onClick={async () => {
                if (showPreview) {
                  setShowPreview(false);
                  setLayers((prev) =>
                    prev.map((l) => l.id === 'preview' ? { ...l, visible: false } : l)
                  );
                } else {
                  let ok = preview.state === 'ready';
                  if (!ok) {
                    if (sourceFile) {
                      ok = await preview.generatePreview(sourceFile, {});
                    } else if (bbox) {
                      ok = await preview.generatePreviewFromBbox(bbox);
                    }
                  }
                  if (ok) {
                    setShowPreview(true);
                    setLayers((prev) =>
                      prev.map((l) => l.id === 'preview' ? { ...l, visible: true } : l)
                    );
                  }
                }
              }}
            >
              {showPreview ? 'Show Map' : '3D View'}
            </button>
          </div>
        </div>
      </div>

      {/* Feature inspector */}
      <div className="px-3 py-3" style={{ borderBottom: '1px solid var(--border-subtle)' }}>
        <FeatureInspector
          feature={
            selectedFeature
              ? {
                  properties: selectedFeature.properties,
                  geometry_type: selectedFeature.geometry_type,
                }
              : null
          }
        />
      </div>

      {/* Export panel — spawn mode toggle lives inside */}
      <div className="px-3 py-3">
        <ExportPanel
          spawnPoint={spawnPoint}
          sourceFile={sourceFile}
          spawnMode={spawnMode}
          onSpawnModeToggle={() => setSpawnMode((prev) => !prev)}
          bbox={bbox}
          featureFilter={featureFilter}
          onFilterChange={setFeatureFilter}
          onConversionDone={() => setRefreshCacheTrigger((n) => n + 1)}
          overpassUrl={overpassUrl}
          overtureSettings={overtureSettings}
          onParamsChange={setMcParams}
        />
      </div>

      {/* Conversion history */}
      <HistoryPanel
        history={history}
        onLoadSettings={(_entry) => {
          // TODO: restore settings from history entry
        }}
      />
    </>
  );

  const mapContent = (
    <div className="relative flex flex-1 overflow-hidden">
      {showPreview && preview.state === 'ready' && preview.bounds ? (
        <ThreePreview blocks={preview.blocks} bounds={preview.bounds} spawn={preview.spawn} />
      ) : (
        <MapView
          onFlyTo={handleFlyToReady}
          geojsonData={geojsonData}
          layerVisibility={layerVisibility}
          onBboxDrawn={handleBboxDrawn}
          onFeatureSelect={handleFeatureSelect}
          onSpawnSet={handleSpawnSet}
          onLayerCounts={handleLayerCounts}
          spawnMode={spawnMode}
          refreshCacheTrigger={refreshCacheTrigger}
          mcOrigin={mcOrigin}
          mcParams={mcParams}
        />
      )}
      <MapLegend layers={displayLayers} hasData={!!geojsonData} />
    </div>
  );

  return (
    <main className="flex h-screen w-screen overflow-hidden" style={{ background: 'var(--bg-deep)' }}>
      {isDesktop ? (
        <>
          <Sidebar theme={theme} onToggleTheme={toggleTheme}>
            {sidebarContent}
          </Sidebar>
          {mapContent}
        </>
      ) : (
        <>
          {mapContent}
          {!sidebarOpen && (
            <button
              onClick={() => setSidebarOpen(true)}
              className="fixed left-3 top-3 z-30 rounded-lg p-2"
              style={{
                background: 'var(--bg-elevated)',
                border: '1px solid var(--border-subtle)',
                color: 'var(--text-primary)',
                cursor: 'pointer',
              }}
            >
              <Menu className="h-5 w-5" />
            </button>
          )}
          {sidebarOpen && (
            <div className="fixed inset-x-0 bottom-0 z-20" style={{ height: '50vh' }}>
              <Sidebar
                theme={theme}
                onToggleTheme={toggleTheme}
                isMobile
                onClose={() => setSidebarOpen(false)}
              >
                {sidebarContent}
              </Sidebar>
            </div>
          )}
        </>
      )}

      {showHelp && <ShortcutHelp onClose={() => setShowHelp(false)} />}
    </main>
  );
}
