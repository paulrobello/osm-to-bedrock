'use client';

import { useState, useRef, useCallback } from 'react';
import type { FeatureFilter } from '@/lib/overpass';

export type ConversionState = 'idle' | 'uploading' | 'converting' | 'done' | 'error';

export interface ConvertOptions {
  worldName: string;
  scale: number;
  buildingHeight: number;
  seaLevel: number;
  signs?: boolean;
  addressSigns?: boolean;
  poiMarkers?: boolean;
  /** Explicit spawn block coordinates — take priority over spawnLat/spawnLon. */
  spawnX?: number;
  spawnY?: number;
  spawnZ?: number;
  /** Spawn position as geographic coordinates — converted to block coords by the Rust converter. */
  spawnLat?: number;
  spawnLon?: number;
  /** Feature filter — controls which OSM types are converted */
  filter?: FeatureFilter;
  /** Download real-world elevation (SRTM) and apply to terrain */
  useElevation?: boolean;
  /** Vertical exaggeration multiplier for elevation (default 1.0) */
  verticalScale?: number;
  /** Median-filter radius for elevation smoothing. 0 = raw terrain, 1 = gentle (default), 2+ = aggressive. */
  elevationSmoothing?: number;
  /** Terrain fill depth below surface in blocks. Lower = faster conversion and smaller worlds. Default 4. */
  surfaceThickness?: number;
  /** Snap building walls within this many blocks of axis-aligned to straight. 0 = off. Default 1. */
  wallStraightenThreshold?: number;
  /** Overpass API URL override. Uses server default if omitted. */
  overpassUrl?: string;
  /** Enable Overture Maps data supplement */
  overture?: boolean;
  /** Overture themes to include */
  overtureThemes?: string[];
  /** Per-theme source priority */
  overturePriority?: Record<string, string>;
  /** Timeout for Overture CLI calls in seconds */
  overtureTimeout?: number;
  /** Place decorative blocks at POI locations */
  poiDecorations?: boolean;
  /** Place individual trees from tree node data */
  natureDecorations?: boolean;
}

export interface UseConversionReturn {
  conversionState: ConversionState;
  progress: number;
  status: string;
  message: string;
  downloadUrl: string | null;
  error: string | null;
  downloadProgress: number;
  downloadTotal: number;
  isDownloading: boolean;
  downloadFilename: string;
  startConversion: (file: File | null, options: ConvertOptions) => Promise<void>;
  startFetchConvert: (
    bbox: [number, number, number, number],
    options: ConvertOptions
  ) => Promise<void>;
  startTerrainConvert: (
    bbox: [number, number, number, number],
    options: ConvertOptions
  ) => Promise<void>;
  startOvertureConvert: (
    bbox: [number, number, number, number],
    options: ConvertOptions & { themes?: string[] }
  ) => Promise<void>;
  reset: () => void;
}

const POLL_INTERVAL_MS = 2_000;

export function useConversion(): UseConversionReturn {
  const [conversionState, setConversionState] = useState<ConversionState>('idle');
  const [progress, setProgress] = useState<number>(0);
  const [status, setStatus] = useState<string>('');
  const [message, setMessage] = useState<string>('');
  const [downloadUrl, setDownloadUrl] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [downloadProgress, setDownloadProgress] = useState<number>(0);
  const [downloadTotal, setDownloadTotal] = useState<number>(0);
  const [isDownloading, setIsDownloading] = useState<boolean>(false);
  const [downloadFilename, setDownloadFilename] = useState<string>('world.mcworld');

  const pollTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const abortRef = useRef<AbortController | null>(null);

  const stopPolling = useCallback(() => {
    if (pollTimerRef.current !== null) {
      clearTimeout(pollTimerRef.current);
      pollTimerRef.current = null;
    }
  }, []);

  const reset = useCallback(() => {
    stopPolling();
    if (abortRef.current) {
      abortRef.current.abort();
      abortRef.current = null;
    }
    setConversionState('idle');
    setProgress(0);
    setStatus('');
    setMessage('');
    setDownloadUrl(null);
    setError(null);
    setDownloadProgress(0);
    setDownloadTotal(0);
    setIsDownloading(false);
    setDownloadFilename('world.mcworld');
  }, [stopPolling]);

  const downloadFile = useCallback(async (url: string) => {
    setIsDownloading(true);
    setDownloadProgress(0);
    setDownloadTotal(0);
    try {
      const res = await fetch(url);
      const total = parseInt(res.headers.get('content-length') || '0');
      setDownloadTotal(total);
      const reader = res.body!.getReader();
      const chunks: Uint8Array[] = [];
      let received = 0;
      while (true) {
        const { done, value } = await reader.read();
        if (done) break;
        chunks.push(value);
        received += value.length;
        setDownloadProgress(received);
      }
      const blob = new Blob(chunks as BlobPart[], { type: 'application/octet-stream' });
      const blobUrl = URL.createObjectURL(blob);
      setDownloadUrl(blobUrl);
      // Extract filename from Content-Disposition or use default
      const disposition = res.headers.get('content-disposition') || '';
      const match = disposition.match(/filename="([^"]+)"/) || disposition.match(/filename=(\S+)/);
      setDownloadFilename(match?.[1] || 'world.mcworld');
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : 'Download failed';
      setError(msg);
      setConversionState('error');
    } finally {
      setIsDownloading(false);
    }
  }, []);

  const pollStatus = useCallback(
    (jobId: string) => {
      const poll = async () => {
        try {
          const res = await fetch(`/api/status/${encodeURIComponent(jobId)}`, {
            signal: AbortSignal.timeout(10_000),
          });

          if (!res.ok) {
            const text = await res.text().catch(() => res.statusText);
            throw new Error(`Status check failed (${res.status}): ${text}`);
          }

          const data = (await res.json()) as {
            state: string;
            progress: number;
            message: string;
          };

          setProgress(data.progress ?? 0);
          setMessage(data.message ?? '');
          setStatus(data.state ?? '');

          if (data.state === 'done' || data.state === 'complete' || data.state === 'completed') {
            stopPolling();
            setConversionState('done');
            const dlUrl = `/api/download?id=${encodeURIComponent(jobId)}`;
            void downloadFile(dlUrl);
          } else if (data.state === 'error' || data.state === 'failed') {
            stopPolling();
            setConversionState('error');
            setError(data.message ?? 'Conversion failed');
          } else {
            // Still in progress — schedule next poll
            pollTimerRef.current = setTimeout(() => void poll(), POLL_INTERVAL_MS);
          }
        } catch (err: unknown) {
          stopPolling();
          const msg = err instanceof Error ? err.message : 'Polling failed';
          setError(msg);
          setConversionState('error');
        }
      };

      pollTimerRef.current = setTimeout(() => void poll(), POLL_INTERVAL_MS);
    },
    [stopPolling, downloadFile]
  );

  const startConversion = useCallback(
    async (file: File | null, options: ConvertOptions) => {
      if (!file) {
        setError('No file selected');
        setConversionState('error');
        return;
      }

      // Clean up any previous run
      stopPolling();
      if (abortRef.current) {
        abortRef.current.abort();
      }
      const controller = new AbortController();
      abortRef.current = controller;

      setConversionState('uploading');
      setProgress(0);
      setStatus('uploading');
      setMessage('Uploading file…');
      setError(null);
      setDownloadUrl(null);
      setDownloadProgress(0);
      setDownloadTotal(0);
      setIsDownloading(false);

      try {
        const form = new FormData();
        form.append('file', file);
        // Rust expects snake_case field names
        form.append('options', JSON.stringify({
          world_name: options.worldName,
          scale: options.scale,
          building_height: options.buildingHeight,
          sea_level: options.seaLevel,
          signs: options.signs,
          address_signs: options.addressSigns,
          poi_markers: options.poiMarkers,
          spawn_x: options.spawnX,
          spawn_y: options.spawnY,
          spawn_z: options.spawnZ,
          spawn_lat: options.spawnLat,
          spawn_lon: options.spawnLon,
          roads: options.filter?.roads ?? true,
          buildings: options.filter?.buildings ?? true,
          water: options.filter?.water ?? true,
          landuse: options.filter?.landuse ?? true,
          railways: options.filter?.railways ?? true,
          use_elevation: options.useElevation ?? false,
          vertical_scale: options.verticalScale ?? 1.0,
          elevation_smoothing: options.elevationSmoothing ?? 1,
          surface_thickness: options.surfaceThickness ?? 4,
          wall_straighten_threshold: options.wallStraightenThreshold ?? 1,
          poi_decorations: options.poiDecorations ?? true,
          nature_decorations: options.natureDecorations ?? true,
        }));

        const res = await fetch('/api/convert', {
          method: 'POST',
          body: form,
          signal: controller.signal,
        });

        if (!res.ok) {
          const json = (await res.json().catch(() => ({ error: res.statusText }))) as {
            error?: string;
          };
          throw new Error(json.error ?? `HTTP ${res.status}`);
        }

        const data = (await res.json()) as { job_id: string };
        if (!data.job_id) {
          throw new Error('API response missing job_id');
        }

        setConversionState('converting');
        setStatus('converting');
        setMessage('Converting…');
        setProgress(0);

        pollStatus(data.job_id);
      } catch (err: unknown) {
        if (err instanceof Error && err.name === 'AbortError') {
          // Intentionally aborted — reset() was called
          return;
        }
        const msg = err instanceof Error ? err.message : 'Conversion failed';
        setError(msg);
        setConversionState('error');
      }
    },
    [stopPolling, pollStatus]
  );

  const startFetchConvert = useCallback(
    async (bbox: [number, number, number, number], options: ConvertOptions) => {
      stopPolling();
      if (abortRef.current) abortRef.current.abort();
      const controller = new AbortController();
      abortRef.current = controller;

      setConversionState('uploading');
      setProgress(0);
      setStatus('fetching');
      setMessage('Fetching from Overpass…');
      setError(null);
      setDownloadUrl(null);
      setDownloadProgress(0);
      setDownloadTotal(0);
      setIsDownloading(false);

      try {
        const body = {
          bbox,
          options: {
            world_name: options.worldName,
            scale: options.scale,
            building_height: options.buildingHeight,
            sea_level: options.seaLevel,
            signs: options.signs,
            address_signs: options.addressSigns,
            poi_markers: options.poiMarkers,
            spawn_lat: options.spawnLat,
            spawn_lon: options.spawnLon,
            spawn_x: options.spawnX,
            spawn_y: options.spawnY,
            spawn_z: options.spawnZ,
            use_elevation: options.useElevation ?? false,
            vertical_scale: options.verticalScale ?? 1.0,
            elevation_smoothing: options.elevationSmoothing ?? 1,
            surface_thickness: options.surfaceThickness ?? 4,
            wall_straighten_threshold: options.wallStraightenThreshold ?? 1,
          poi_decorations: options.poiDecorations ?? true,
          nature_decorations: options.natureDecorations ?? true,
          },
          filter: options.filter ?? {
            roads: true, buildings: true, water: true, landuse: true, railways: true,
          },
          ...(options.overpassUrl ? { overpass_url: options.overpassUrl } : {}),
          overture: options.overture ?? false,
          overture_themes: options.overtureThemes ?? [],
          overture_priority: options.overturePriority ?? {},
          overture_timeout: options.overtureTimeout ?? 120,
        };

        const res = await fetch('/api/fetch-convert', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(body),
          signal: controller.signal,
        });

        if (!res.ok) {
          const json = (await res.json().catch(() => ({ error: res.statusText }))) as { error?: string };
          throw new Error(json.error ?? `HTTP ${res.status}`);
        }

        const data = (await res.json()) as { job_id: string };
        if (!data.job_id) throw new Error('API response missing job_id');

        setConversionState('converting');
        setStatus('converting');
        setMessage('Converting…');
        setProgress(0);
        pollStatus(data.job_id);
      } catch (err: unknown) {
        if (err instanceof Error && err.name === 'AbortError') return;
        setError(err instanceof Error ? err.message : 'Fetch-convert failed');
        setConversionState('error');
      }
    },
    [stopPolling, pollStatus]
  );

  const startOvertureConvert = useCallback(
    async (bbox: [number, number, number, number], options: ConvertOptions & { themes?: string[] }) => {
      stopPolling();
      if (abortRef.current) abortRef.current.abort();
      const controller = new AbortController();
      abortRef.current = controller;

      setConversionState('uploading');
      setProgress(0);
      setStatus('fetching');
      setMessage('Fetching from Overture Maps…');
      setError(null);
      setDownloadUrl(null);
      setDownloadProgress(0);
      setDownloadTotal(0);
      setIsDownloading(false);

      try {
        // Frontend bbox is [minLon, minLat, maxLon, maxLat]
        // Rust API expects [south, west, north, east] = [minLat, minLon, maxLat, maxLon]
        const [minLon, minLat, maxLon, maxLat] = bbox;
        const rustBbox: [number, number, number, number] = [minLat, minLon, maxLat, maxLon];

        const body = {
          bbox: rustBbox,
          options: {
            world_name: options.worldName,
            scale: options.scale,
            building_height: options.buildingHeight,
            sea_level: options.seaLevel,
            signs: options.signs ?? false,
            address_signs: options.addressSigns ?? false,
            poi_markers: options.poiMarkers ?? false,
            spawn_x: options.spawnX ?? null,
            spawn_y: options.spawnY ?? null,
            spawn_z: options.spawnZ ?? null,
            spawn_lat: options.spawnLat ?? null,
            spawn_lon: options.spawnLon ?? null,
            use_elevation: options.useElevation ?? false,
            vertical_scale: options.verticalScale ?? 1.0,
            elevation_smoothing: options.elevationSmoothing ?? 1,
            surface_thickness: options.surfaceThickness ?? 4,
            wall_straighten_threshold: options.wallStraightenThreshold ?? 1,
          poi_decorations: options.poiDecorations ?? true,
          nature_decorations: options.natureDecorations ?? true,
          },
          themes: options.themes ?? options.overtureThemes ?? ['building', 'transportation'],
          timeout: options.overtureTimeout ?? 120,
        };

        const res = await fetch('/api/overture-convert', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(body),
          signal: controller.signal,
        });

        if (!res.ok) {
          const json = (await res.json().catch(() => ({ error: res.statusText }))) as { error?: string };
          throw new Error(json.error ?? `HTTP ${res.status}`);
        }

        const data = (await res.json()) as { job_id: string };
        if (!data.job_id) throw new Error('API response missing job_id');

        setConversionState('converting');
        setStatus('converting');
        setMessage('Converting Overture data…');
        setProgress(0);

        const filename = `${options.worldName || 'overture-world'}.mcworld`;
        setDownloadFilename(filename);

        pollStatus(data.job_id);
      } catch (err: unknown) {
        if (err instanceof Error && err.name === 'AbortError') return;
        setError(err instanceof Error ? err.message : 'Overture-convert failed');
        setConversionState('error');
      }
    },
    [stopPolling, pollStatus]
  );

  const startTerrainConvert = useCallback(
    async (bbox: [number, number, number, number], options: ConvertOptions) => {
      stopPolling();
      if (abortRef.current) abortRef.current.abort();
      const controller = new AbortController();
      abortRef.current = controller;

      setConversionState('uploading');
      setProgress(0);
      setStatus('fetching');
      setMessage('Downloading elevation tiles…');
      setError(null);
      setDownloadUrl(null);
      setDownloadProgress(0);
      setDownloadTotal(0);
      setIsDownloading(false);

      try {
        const body = {
          bbox,
          options: {
            world_name: options.worldName,
            scale: options.scale,
            sea_level: options.seaLevel,
            vertical_scale: options.verticalScale ?? 1.0,
            elevation_smoothing: options.elevationSmoothing ?? 1,
            surface_thickness: options.surfaceThickness ?? 4,
            wall_straighten_threshold: options.wallStraightenThreshold ?? 1,
          poi_decorations: options.poiDecorations ?? true,
          nature_decorations: options.natureDecorations ?? true,
            use_elevation: options.useElevation ?? true,
            spawn_lat: options.spawnLat,
            spawn_lon: options.spawnLon,
            spawn_x: options.spawnX,
            spawn_y: options.spawnY,
            spawn_z: options.spawnZ,
          },
        };

        const res = await fetch('/api/terrain-convert', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(body),
          signal: controller.signal,
        });

        if (!res.ok) {
          const json = (await res.json().catch(() => ({ error: res.statusText }))) as { error?: string };
          throw new Error(json.error ?? `HTTP ${res.status}`);
        }

        const data = (await res.json()) as { job_id: string };
        if (!data.job_id) throw new Error('API response missing job_id');

        setConversionState('converting');
        setStatus('converting');
        setMessage('Generating terrain…');
        setProgress(0);
        pollStatus(data.job_id);
      } catch (err: unknown) {
        if (err instanceof Error && err.name === 'AbortError') return;
        setError(err instanceof Error ? err.message : 'Terrain generation failed');
        setConversionState('error');
      }
    },
    [stopPolling, pollStatus]
  );

  return {
    conversionState,
    progress,
    status,
    message,
    downloadUrl,
    error,
    downloadProgress,
    downloadTotal,
    isDownloading,
    downloadFilename,
    startConversion,
    startFetchConvert,
    startTerrainConvert,
    startOvertureConvert,
    reset,
  };
}
