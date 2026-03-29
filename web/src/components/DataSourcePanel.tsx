'use client';

import { useRef, useState, useCallback, useEffect, type DragEvent, type ChangeEvent } from 'react';
import type GeoJSON from 'geojson';
import { Button } from '@/components/ui/button';
import { Switch } from '@/components/ui/switch';
import { type FeatureFilter, defaultFilter } from '@/lib/overpass';

const DEFAULT_OVERPASS_URL = 'https://overpass-api.de/api/interpreter';

type Mode = 'overpass' | 'upload';

interface UploadResult {
  geojson: GeoJSON.FeatureCollection;
  bounds?: [number, number, number, number];
  stats?: Record<string, number>;
}

/** Merge multiple upload results into a single FeatureCollection. */
function mergeResults(results: UploadResult[]): GeoJSON.FeatureCollection {
  const allFeatures: GeoJSON.Feature[] = [];
  let minLon = Infinity;
  let minLat = Infinity;
  let maxLon = -Infinity;
  let maxLat = -Infinity;

  for (const r of results) {
    allFeatures.push(...r.geojson.features);
    if (r.bounds) {
      const [lon1, lat1, lon2, lat2] = r.bounds;
      minLon = Math.min(minLon, lon1);
      minLat = Math.min(minLat, lat1);
      maxLon = Math.max(maxLon, lon2);
      maxLat = Math.max(maxLat, lat2);
    }
  }

  const merged: GeoJSON.FeatureCollection = {
    type: 'FeatureCollection',
    features: allFeatures,
  };

  if (Number.isFinite(minLon)) {
    (merged as GeoJSON.FeatureCollection & { bbox: number[] }).bbox = [
      minLon,
      minLat,
      maxLon,
      maxLat,
    ];
  }

  return merged;
}

interface DataSourcePanelProps {
  bbox: [number, number, number, number] | null;
  onDataLoaded: (geojson: GeoJSON.FeatureCollection) => void;
  onFileUploaded?: (files: File[]) => void;
  loading: boolean;
  onLoadingChange: (loading: boolean) => void;
  featureFilter?: FeatureFilter;
  onOverpassUrlChange?: (url: string) => void;
  overtureAvailable?: boolean;
  onOvertureSettingsChange?: (settings: {
    enabled: boolean;
    themes: string[];
    priority: Record<string, string>;
  }) => void;
}

const OVERTURE_THEMES = ['building', 'transportation', 'place', 'base', 'address'] as const;
type OvertureTheme = typeof OVERTURE_THEMES[number];

export function DataSourcePanel({
  bbox,
  onDataLoaded,
  onFileUploaded,
  loading,
  onLoadingChange,
  featureFilter = defaultFilter,
  onOverpassUrlChange,
  overtureAvailable = false,
  onOvertureSettingsChange,
}: DataSourcePanelProps) {
  const [mode, setMode] = useState<Mode>('overpass');
  const [error, setError] = useState<string | null>(null);
  const [dragging, setDragging] = useState(false);
  const [fileCount, setFileCount] = useState(0);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const [overpassUrl, setOverpassUrl] = useState<string>(() => {
    if (typeof window === 'undefined') return DEFAULT_OVERPASS_URL;
    return localStorage.getItem('overpass_url') ?? DEFAULT_OVERPASS_URL;
  });
  const [showAdvanced, setShowAdvanced] = useState(false);

  // Overture Maps state
  const [overtureEnabled, setOvertureEnabled] = useState(true);
  const [overtureThemes, setOvertureThemes] = useState<Record<OvertureTheme, boolean>>({
    building: true,
    transportation: true,
    place: true,
    base: true,
    address: true,
  });
  const [overturePriority, setOverturePriority] = useState<Record<OvertureTheme, string>>({
    building: 'overture',
    transportation: 'overture',
    place: 'overture',
    base: 'overture',
    address: 'overture',
  });

  const handleOverpassUrlChange = (url: string) => {
    setOverpassUrl(url);
    localStorage.setItem('overpass_url', url);
    onOverpassUrlChange?.(url);
  };

  // Notify parent whenever overture settings change
  useEffect(() => {
    const enabledThemes = OVERTURE_THEMES.filter((t) => overtureThemes[t]);
    const priority: Record<string, string> = {};
    for (const t of enabledThemes) {
      priority[t] = overturePriority[t];
    }
    onOvertureSettingsChange?.({
      enabled: overtureEnabled,
      themes: enabledThemes,
      priority,
    });
  }, [overtureEnabled, overtureThemes, overturePriority, onOvertureSettingsChange]);

  const clearError = () => setError(null);

  // ── Overpass fetch (via Rust backend — cache-aware) ─────────────────────────

  const handleFetch = useCallback(async () => {
    if (!bbox) return;
    clearError();
    onLoadingChange(true);
    try {
      const res = await fetch('/api/fetch-preview', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          bbox,
          filter: featureFilter ?? defaultFilter,
          overpass_url: overpassUrl !== DEFAULT_OVERPASS_URL ? overpassUrl : undefined,
        }),
        signal: AbortSignal.timeout(120_000),
      });

      if (!res.ok) {
        const json = (await res.json().catch(() => ({ error: res.statusText }))) as {
          error?: string;
        };
        throw new Error(json.error ?? `HTTP ${res.status}`);
      }

      const data = (await res.json()) as { geojson: GeoJSON.FeatureCollection };
      onDataLoaded(data.geojson);
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : 'Failed to fetch data');
    } finally {
      onLoadingChange(false);
    }
  }, [bbox, featureFilter, overpassUrl, onDataLoaded, onLoadingChange]);

  // ── Upload (single or multiple files) ─────────────────────────────────────

  const uploadSingleFile = async (file: File): Promise<UploadResult> => {
    const form = new FormData();
    form.append('file', file);

    const res = await fetch('/api/upload', {
      method: 'POST',
      body: form,
      signal: AbortSignal.timeout(40_000),
    });

    if (!res.ok) {
      const json = (await res.json().catch(() => ({ error: res.statusText }))) as {
        error?: string;
      };
      throw new Error(json.error ?? `HTTP ${res.status}`);
    }

    const data = (await res.json()) as UploadResult;
    if (!data.geojson) throw new Error('API response missing geojson field');
    return data;
  };

  const handleFiles = useCallback(
    async (files: File[]) => {
      // Validate all files first
      for (const file of files) {
        if (
          !file.name.endsWith('.osm.pbf') &&
          !file.name.endsWith('.pbf') &&
          !file.name.endsWith('.osm')
        ) {
          setError(`Invalid file: ${file.name} — please upload .osm.pbf or .osm files.`);
          return;
        }
      }

      clearError();
      setFileCount(files.length);
      onLoadingChange(true);
      try {
        // Upload each file sequentially and collect results
        const results: UploadResult[] = [];
        for (const file of files) {
          const result = await uploadSingleFile(file);
          results.push(result);
        }

        // Merge results if multiple files, otherwise use the single result
        const merged = results.length === 1 ? results[0].geojson : mergeResults(results);
        onDataLoaded(merged);
        onFileUploaded?.(files);
      } catch (err: unknown) {
        setError(err instanceof Error ? err.message : 'Upload failed');
      } finally {
        onLoadingChange(false);
      }
    },
    [onDataLoaded, onFileUploaded, onLoadingChange],
  );

  const handleFileInput = (e: ChangeEvent<HTMLInputElement>) => {
    const fileList = e.target.files;
    if (fileList && fileList.length > 0) {
      void handleFiles(Array.from(fileList));
    }
    // Reset input so the same file(s) can be re-selected
    e.target.value = '';
  };

  const handleDrop = (e: DragEvent<HTMLDivElement>) => {
    e.preventDefault();
    setDragging(false);
    const fileList = e.dataTransfer.files;
    if (fileList.length > 0) {
      void handleFiles(Array.from(fileList));
    }
  };

  const handleDragOver = (e: DragEvent<HTMLDivElement>) => {
    e.preventDefault();
    setDragging(true);
  };

  const handleDragLeave = () => setDragging(false);

  return (
    <div
      className="flex flex-col gap-3 rounded-xl p-4"
      style={{
        background: 'var(--bg-surface)',
        border: '1px solid var(--border-subtle)',
      }}
    >
      {/* Section header */}
      <div className="flex items-center justify-between">
        <span
          className="font-[family-name:var(--font-dm-sans)] text-[10px] font-semibold uppercase tracking-[0.15em]"
          style={{ color: 'var(--text-muted)' }}
        >
          Data Source
        </span>
      </div>

      {/* Mode toggle — pill-shaped toggle group */}
      <div
        className="flex gap-1 rounded-full p-1"
        style={{
          background: 'var(--bg-deep)',
          border: '1px solid var(--border-subtle)',
        }}
      >
        {(['overpass', 'upload'] as const).map((m) => {
          const active = mode === m;
          return (
            <button
              key={m}
              onClick={() => {
                setMode(m);
                clearError();
              }}
              className="flex-1 rounded-full py-1 font-[family-name:var(--font-dm-sans)] text-xs font-medium capitalize transition-all duration-200"
              style={{
                background: active ? 'var(--accent-cyan)' : 'transparent',
                color: active ? 'var(--bg-deep)' : 'var(--text-secondary)',
                boxShadow: active ? '0 0 12px rgba(86,200,216,0.35)' : 'none',
              }}
            >
              {m === 'overpass' ? 'Overpass' : 'Upload'}
            </button>
          );
        })}
      </div>

      {/* Divider */}
      <div
        style={{
          height: 1,
          background: 'var(--border-subtle)',
        }}
      />

      {/* Mode content */}
      {mode === 'overpass' ? (
        <div className="flex flex-col gap-3">
          <p
            className="font-[family-name:var(--font-dm-sans)] text-xs leading-relaxed"
            style={{ color: 'var(--text-secondary)' }}
          >
            Draw a rectangle on the map, then click{' '}
            <span style={{ color: 'var(--accent-cyan)' }}>Fetch Data</span>.
          </p>

          {bbox && (
            <div
              className="rounded-md px-2 py-1.5 font-[family-name:var(--font-jetbrains-mono)] text-[10px]"
              style={{
                background: 'var(--bg-deep)',
                color: 'var(--text-muted)',
                border: '1px solid var(--border-subtle)',
              }}
            >
              [{bbox.map((v) => v.toFixed(5)).join(', ')}]
            </div>
          )}

          <Button
            onClick={() => void handleFetch()}
            disabled={!bbox || loading}
            size="sm"
            className="w-full font-[family-name:var(--font-dm-sans)] text-xs font-medium transition-all duration-200"
            style={
              bbox && !loading
                ? {
                    background: 'rgba(86,200,216,0.15)',
                    color: 'var(--accent-cyan)',
                    border: '1px solid rgba(86,200,216,0.3)',
                    boxShadow: '0 0 8px rgba(86,200,216,0.1)',
                  }
                : {
                    background: 'var(--bg-deep)',
                    color: 'var(--text-muted)',
                    border: '1px solid var(--border-subtle)',
                  }
            }
          >
            {loading ? (
              <span className="flex items-center gap-2">
                <span
                  className="inline-block size-3 animate-spin rounded-full border-2 border-current border-t-transparent"
                  style={{ animationDuration: '0.8s' }}
                />
                Fetching…
              </span>
            ) : (
              'Fetch Data'
            )}
          </Button>

          {/* Advanced: Overpass URL */}
          <div style={{ marginTop: '8px' }}>
            <button
              type="button"
              onClick={() => setShowAdvanced((v) => !v)}
              style={{
                background: 'none',
                border: 'none',
                cursor: 'pointer',
                color: 'var(--text-secondary)',
                fontSize: '11px',
                padding: '2px 0',
                display: 'flex',
                alignItems: 'center',
                gap: '4px',
              }}
            >
              <span style={{ transform: showAdvanced ? 'rotate(90deg)' : 'none', display: 'inline-block', transition: 'transform 0.15s' }}>▶</span>
              Advanced
            </button>
            {showAdvanced && (
              <div style={{ marginTop: '6px' }}>
                <label style={{ fontSize: '11px', color: 'var(--text-secondary)', display: 'block', marginBottom: '3px' }}>
                  Overpass API URL
                </label>
                <input
                  type="url"
                  value={overpassUrl}
                  onChange={(e) => handleOverpassUrlChange(e.target.value)}
                  placeholder={DEFAULT_OVERPASS_URL}
                  style={{
                    width: '100%',
                    background: 'var(--bg-card)',
                    border: '1px solid var(--border-subtle)',
                    borderRadius: '4px',
                    color: 'var(--text-primary)',
                    fontSize: '11px',
                    padding: '4px 6px',
                    boxSizing: 'border-box',
                  }}
                />
                <button
                  type="button"
                  onClick={() => handleOverpassUrlChange(DEFAULT_OVERPASS_URL)}
                  style={{
                    marginTop: '3px',
                    background: 'none',
                    border: 'none',
                    cursor: 'pointer',
                    color: 'var(--text-secondary)',
                    fontSize: '10px',
                    padding: '0',
                  }}
                >
                  Reset to default
                </button>
              </div>
            )}
          </div>
        </div>
      ) : (
        <div className="flex flex-col gap-3">
          {/* Drop zone */}
          <div
            onDrop={handleDrop}
            onDragOver={handleDragOver}
            onDragLeave={handleDragLeave}
            onClick={() => fileInputRef.current?.click()}
            className="flex cursor-pointer flex-col items-center justify-center gap-2 rounded-lg py-6 transition-all duration-200"
            style={{
              border: `1.5px dashed ${dragging ? 'var(--accent-cyan)' : 'var(--border-medium)'}`,
              background: dragging ? 'rgba(86,200,216,0.06)' : 'var(--bg-deep)',
              boxShadow: dragging ? '0 0 16px rgba(86,200,216,0.08) inset' : 'none',
            }}
          >
            {/* Cloud-upload icon */}
            <svg
              width="26"
              height="26"
              viewBox="0 0 24 24"
              fill="none"
              strokeWidth="1.5"
              strokeLinecap="round"
              strokeLinejoin="round"
              style={{
                stroke: dragging ? 'var(--accent-cyan)' : 'var(--text-muted)',
                transition: 'stroke 0.2s',
              }}
            >
              <path d="M4 14.899A7 7 0 1 1 15.71 8h1.79a4.5 4.5 0 0 1 2.5 8.242" />
              <path d="M12 12v9" />
              <path d="m16 16-4-4-4 4" />
            </svg>
            <span
              className="font-[family-name:var(--font-dm-sans)] text-xs transition-colors duration-200"
              style={{ color: dragging ? 'var(--accent-cyan)' : 'var(--text-secondary)' }}
            >
              {dragging ? 'Drop to upload' : 'Drop .osm.pbf or .osm file(s) here or click to browse'}
            </span>
          </div>

          <input
            ref={fileInputRef}
            type="file"
            accept=".osm.pbf,.pbf,.osm"
            multiple
            className="hidden"
            onChange={handleFileInput}
          />

          {/* File count indicator */}
          {fileCount > 1 && !loading && (
            <div
              className="font-[family-name:var(--font-dm-sans)] text-xs"
              style={{ color: 'var(--text-secondary)' }}
            >
              {fileCount} files selected
            </div>
          )}

          {loading && (
            <div
              className="flex animate-pulse items-center gap-2 font-[family-name:var(--font-dm-sans)] text-xs"
              style={{ color: 'var(--accent-cyan)' }}
            >
              <span className="inline-block size-3 animate-spin rounded-full border-2 border-current border-t-transparent" />
              {fileCount > 1
                ? `Uploading and parsing ${fileCount} files…`
                : 'Uploading and parsing…'}
            </div>
          )}
        </div>
      )}

      {/* Overture Maps section */}
      <div
        style={{
          borderTop: '1px solid var(--border-subtle)',
          paddingTop: '12px',
          marginTop: '4px',
        }}
      >
        {/* Header row with toggle */}
        <div className="flex items-center justify-between" style={{ marginBottom: overtureEnabled && overtureAvailable ? '10px' : 0 }}>
          <span
            className="font-[family-name:var(--font-dm-sans)] text-[11px] font-semibold"
            style={{ color: 'var(--text-secondary)' }}
          >
            Overture Maps
          </span>
          <Switch
            size="sm"
            checked={overtureEnabled}
            onCheckedChange={(checked) => setOvertureEnabled(checked)}
            disabled={!overtureAvailable}
          />
        </div>

        {/* Availability warning */}
        {!overtureAvailable && (
          <p
            className="font-[family-name:var(--font-dm-sans)] text-[10px] leading-relaxed"
            style={{ color: 'var(--text-muted)', marginTop: '6px' }}
          >
            Overture CLI not installed on server
          </p>
        )}

        {/* Per-theme controls (only when enabled and available) */}
        {overtureEnabled && overtureAvailable && (
          <div className="flex flex-col gap-1.5">
            {OVERTURE_THEMES.map((theme) => (
              <div
                key={theme}
                className="flex items-center gap-2 rounded-md px-2 py-1.5"
                style={{
                  background: 'var(--bg-deep)',
                  border: '1px solid var(--border-subtle)',
                }}
              >
                <Switch
                  size="sm"
                  checked={overtureThemes[theme]}
                  onCheckedChange={(checked) =>
                    setOvertureThemes((prev) => ({ ...prev, [theme]: checked }))
                  }
                />
                <span
                  className="flex-1 font-[family-name:var(--font-dm-sans)] text-[11px] capitalize"
                  style={{ color: overtureThemes[theme] ? 'var(--text-primary)' : 'var(--text-muted)' }}
                >
                  {theme}
                </span>
                <select
                  value={overturePriority[theme]}
                  onChange={(e) =>
                    setOverturePriority((prev) => ({ ...prev, [theme]: e.target.value }))
                  }
                  disabled={!overtureThemes[theme]}
                  className="rounded font-[family-name:var(--font-dm-sans)] text-[10px]"
                  style={{
                    background: 'var(--bg-card, #1a1a2e)',
                    border: '1px solid var(--border-subtle)',
                    color: overtureThemes[theme] ? 'var(--text-secondary)' : 'var(--text-muted)',
                    padding: '2px 4px',
                    cursor: overtureThemes[theme] ? 'pointer' : 'not-allowed',
                    opacity: overtureThemes[theme] ? 1 : 0.5,
                  }}
                >
                  <option value="both">both</option>
                  <option value="overture">overture</option>
                  <option value="osm">osm</option>
                </select>
              </div>
            ))}
          </div>
        )}
      </div>

      {/* Error display */}
      {error && (
        <div
          className="rounded-md px-3 py-2 font-[family-name:var(--font-dm-sans)] text-xs leading-relaxed"
          style={{
            background: 'rgba(232,93,93,0.08)',
            border: '1px solid rgba(232,93,93,0.25)',
            color: '#f4a0a0',
          }}
        >
          <span style={{ color: 'var(--error)', fontWeight: 600 }}>Error: </span>
          {error}
        </div>
      )}
    </div>
  );
}
