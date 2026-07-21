'use client';

import { useState, useRef, useCallback, useEffect } from 'react';
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
  etaSeconds: number | null;
  rate: number | null;
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
/** Per-poll network timeout. The fetch is also tied to the active AbortController so reset()/unmount cancels it. */
const POLL_TIMEOUT_MS = 10_000;

interface RunConversionJobOpts {
  /** Status string shown during the upload/fetch phase (e.g. 'uploading', 'fetching'). */
  uploadStatus: string;
  /** User-facing message shown during the upload/fetch phase. */
  uploadMessage: string;
  /** Message shown once the job is accepted and polling begins. */
  convertingMessage: string;
  /** Fallback error message when the underlying error is not an Error instance. */
  errorFallback: string;
  /** Optional callback fired after the converting transition but before polling begins. */
  onJobCreated?: (jobId: string) => void;
}

export function useConversion(): UseConversionReturn {
  const [conversionState, setConversionState] = useState<ConversionState>('idle');
  const [progress, setProgress] = useState<number>(0);
  const [status, setStatus] = useState<string>('');
  const [message, setMessage] = useState<string>('');
  const [etaSeconds, setEtaSeconds] = useState<number | null>(null);
  const [rate, setRate] = useState<number | null>(null);
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
    setEtaSeconds(null);
    setRate(null);
    setDownloadUrl(null);
    setError(null);
    setDownloadProgress(0);
    setDownloadTotal(0);
    setIsDownloading(false);
    setDownloadFilename('world.mcworld');
  }, [stopPolling]);

  // QA-010: cancel any in-flight poll/upload and clear the pending timer on unmount so we never
  // call setState on an unmounted component.
  useEffect(() => {
    return () => {
      if (pollTimerRef.current !== null) {
        clearTimeout(pollTimerRef.current);
        pollTimerRef.current = null;
      }
      if (abortRef.current) {
        abortRef.current.abort();
        abortRef.current = null;
      }
    };
  }, []);

  const downloadFile = useCallback(async (url: string) => {
    setIsDownloading(true);
    setDownloadProgress(0);
    setDownloadTotal(0);
    try {
      const res = await fetch(url);
      const total = parseInt(res.headers.get('content-length') || '0');
      setDownloadTotal(total);
      if (!res.body) {
        throw new Error('Download response has no body');
      }
      const reader = res.body.getReader();
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
    (jobId: string, signal: AbortSignal) => {
      const poll = async () => {
        if (signal.aborted) return;
        try {
          // Combine the long-lived controller signal with a per-request timeout so that either
          // reset()/unmount (controller) or a hung status endpoint (timeout) cancels the fetch.
          const combined = AbortSignal.any([signal, AbortSignal.timeout(POLL_TIMEOUT_MS)]);
          const res = await fetch(`/api/status/${encodeURIComponent(jobId)}`, {
            signal: combined,
          });

          if (!res.ok) {
            const text = await res.text().catch(() => res.statusText);
            throw new Error(`Status check failed (${res.status}): ${text}`);
          }

          const data = (await res.json()) as {
            state: string;
            progress: number;
            message: string;
            eta_seconds?: number;
            rate?: number;
          };

          setProgress(data.progress ?? 0);
          setMessage(data.message ?? '');
          setStatus(data.state ?? '');
          setEtaSeconds(typeof data.eta_seconds === 'number' ? data.eta_seconds : null);
          setRate(typeof data.rate === 'number' ? data.rate : null);

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
          // Silent on intentional abort (reset or unmount) — don't touch state.
          if (signal.aborted || (err instanceof Error && err.name === 'AbortError')) {
            stopPolling();
            return;
          }
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

  /**
   * Owns the shared start → poll → terminal-transition flow used by every conversion method.
   * Each public method just builds its request body and delegates here.
   */
  const runConversionJob = useCallback(
    async (
      url: string,
      body: BodyInit,
      opts: RunConversionJobOpts,
      headers?: Record<string, string>
    ) => {
      // Clean up any previous run
      stopPolling();
      if (abortRef.current) {
        abortRef.current.abort();
      }
      const controller = new AbortController();
      abortRef.current = controller;

      setConversionState('uploading');
      setProgress(0);
      setStatus(opts.uploadStatus);
      setMessage(opts.uploadMessage);
      setError(null);
      setDownloadUrl(null);
      setDownloadProgress(0);
      setDownloadTotal(0);
      setIsDownloading(false);
      setEtaSeconds(null);
      setRate(null);

      try {
        const res = await fetch(url, {
          method: 'POST',
          body,
          headers,
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
        setMessage(opts.convertingMessage);
        setProgress(0);

        opts.onJobCreated?.(data.job_id);
        pollStatus(data.job_id, controller.signal);
      } catch (err: unknown) {
        if (err instanceof Error && err.name === 'AbortError') {
          // Intentionally aborted — reset() was called
          return;
        }
        const msg = err instanceof Error ? err.message : opts.errorFallback;
        setError(msg);
        setConversionState('error');
      }
    },
    [stopPolling, pollStatus]
  );

  const startConversion = useCallback(
    async (file: File | null, options: ConvertOptions) => {
      if (!file) {
        setError('No file selected');
        setConversionState('error');
        return;
      }

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

      await runConversionJob('/api/convert', form, {
        uploadStatus: 'uploading',
        uploadMessage: 'Uploading file…',
        convertingMessage: 'Converting…',
        errorFallback: 'Conversion failed',
      });
    },
    [runConversionJob]
  );

  const startFetchConvert = useCallback(
    async (bbox: [number, number, number, number], options: ConvertOptions) => {
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
        overture_timeout: options.overtureTimeout ?? 120,
      };

      await runConversionJob('/api/fetch-convert', JSON.stringify(body), {
        uploadStatus: 'fetching',
        uploadMessage: 'Fetching from Overpass…',
        convertingMessage: 'Converting…',
        errorFallback: 'Fetch-convert failed',
      }, { 'Content-Type': 'application/json' });
    },
    [runConversionJob]
  );

  const startOvertureConvert = useCallback(
    async (bbox: [number, number, number, number], options: ConvertOptions & { themes?: string[] }) => {
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

      const filename = `${options.worldName || 'overture-world'}.mcworld`;

      await runConversionJob('/api/overture-convert', JSON.stringify(body), {
        uploadStatus: 'fetching',
        uploadMessage: 'Fetching from Overture Maps…',
        convertingMessage: 'Converting Overture data…',
        errorFallback: 'Overture-convert failed',
        onJobCreated: () => setDownloadFilename(filename),
      }, { 'Content-Type': 'application/json' });
    },
    [runConversionJob]
  );

  const startTerrainConvert = useCallback(
    async (bbox: [number, number, number, number], options: ConvertOptions) => {
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

      await runConversionJob('/api/terrain-convert', JSON.stringify(body), {
        uploadStatus: 'fetching',
        uploadMessage: 'Downloading elevation tiles…',
        convertingMessage: 'Generating terrain…',
        errorFallback: 'Terrain generation failed',
      }, { 'Content-Type': 'application/json' });
    },
    [runConversionJob]
  );

  return {
    conversionState,
    progress,
    status,
    message,
    etaSeconds,
    rate,
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
