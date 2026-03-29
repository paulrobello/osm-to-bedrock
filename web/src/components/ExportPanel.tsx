'use client';

import { useState, useMemo, useEffect, useRef } from 'react';
import { useConversion, type ConvertOptions } from '@/hooks/useConversion';
import { estimateWorldSize, formatBytes } from '@/lib/geo';
import type { FeatureFilter } from '@/lib/overpass';
import { defaultFilter } from '@/lib/overpass';
import {
  ConversionParametersForm,
  type ConversionParameters,
} from '@/components/ConversionParametersForm';
import { ConversionControls } from '@/components/ConversionControls';
import { DownloadProgress } from '@/components/DownloadProgress';

interface SpawnPoint {
  lat: number;
  lon: number;
}

interface ExportPanelProps {
  spawnPoint: SpawnPoint | null;
  sourceFile: File | null;
  onConvert?: () => void;
  onConversionDone?: () => void;
  spawnMode?: boolean;
  onSpawnModeToggle?: () => void;
  bbox?: [number, number, number, number] | null;
  featureFilter: FeatureFilter;
  onFilterChange?: (filter: FeatureFilter) => void;
  overpassUrl?: string;
  overtureSettings?: {
    enabled: boolean;
    themes: string[];
    priority: Record<string, string>;
  };
  onParamsChange?: (params: { scale: number; seaLevel: number; surfaceThickness: number }) => void;
}

const DEFAULT_PARAMS: ConversionParameters = {
  worldName: 'OSM World',
  scale: 1.0,
  buildingHeight: 8,
  seaLevel: 65,
  signs: true,
  addressSigns: true,
  poiMarkers: true,
  poiDecorations: true,
  natureDecorations: true,
  useElevation: false,
  verticalScale: 1.0,
  elevationSmoothing: 1,
  wallStraightenThreshold: 1,
  surfaceThickness: 4,
  terrainOnly: false,
  activePreset: 'Custom',
};

export function ExportPanel({
  spawnPoint,
  sourceFile,
  onConvert,
  onConversionDone,
  spawnMode = false,
  onSpawnModeToggle,
  bbox,
  featureFilter,
  onFilterChange,
  overpassUrl,
  overtureSettings,
  onParamsChange,
}: ExportPanelProps) {
  const [params, setParams] = useState<ConversionParameters>(DEFAULT_PARAMS);

  const updateParams = (updated: Partial<ConversionParameters>) => {
    setParams((prev) => {
      const next = { ...prev, ...updated };
      if (updated.scale !== undefined || updated.seaLevel !== undefined || updated.surfaceThickness !== undefined) {
        onParamsChange?.({ scale: next.scale, seaLevel: next.seaLevel, surfaceThickness: next.surfaceThickness });
      }
      return next;
    });
  };

  // Report initial params on mount
  useEffect(() => {
    onParamsChange?.({ scale: DEFAULT_PARAMS.scale, seaLevel: DEFAULT_PARAMS.seaLevel, surfaceThickness: DEFAULT_PARAMS.surfaceThickness });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const {
    conversionState,
    progress,
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
    reset,
  } = useConversion();

  const isRunning = conversionState === 'uploading' || conversionState === 'converting';

  // Reset conversion state when the user draws a new bounding box
  const prevBboxRef = useRef(bbox);
  useEffect(() => {
    if (prevBboxRef.current !== bbox) {
      prevBboxRef.current = bbox;
      if (conversionState !== 'idle') {
        reset();
      }
    }
  }, [bbox, conversionState, reset]);

  // Notify parent when conversion completes
  const prevConversionStateRef = useRef(conversionState);
  useEffect(() => {
    if (prevConversionStateRef.current !== 'done' && conversionState === 'done') {
      onConversionDone?.();
    }
    prevConversionStateRef.current = conversionState;
  }, [conversionState, onConversionDone]);

  const buildOptions = (): ConvertOptions => ({
    worldName: params.worldName,
    scale: params.scale,
    buildingHeight: params.buildingHeight,
    seaLevel: params.seaLevel,
    signs: params.signs,
    addressSigns: params.addressSigns,
    poiMarkers: params.poiMarkers,
    poiDecorations: params.poiDecorations,
    natureDecorations: params.natureDecorations,
    filter: featureFilter,
    useElevation: params.useElevation,
    verticalScale: params.verticalScale,
    elevationSmoothing: params.elevationSmoothing,
    wallStraightenThreshold: params.wallStraightenThreshold,
    surfaceThickness: params.surfaceThickness,
    overpassUrl,
    overture: overtureSettings?.enabled ?? false,
    overtureThemes: overtureSettings?.themes ?? [],
    overturePriority: overtureSettings?.priority ?? {},
    ...(spawnPoint ? { spawnLat: spawnPoint.lat, spawnLon: spawnPoint.lon } : {}),
  });

  const handleConvert = () => {
    onConvert?.();
    void startConversion(sourceFile, buildOptions());
  };

  const handleFetchConvert = () => {
    if (!bbox) return;
    startFetchConvert(bbox, buildOptions());
    onConvert?.();
  };

  const handleTerrainConvert = () => {
    if (!bbox) return;
    startTerrainConvert(bbox, buildOptions());
    onConvert?.();
  };

  // Bbox estimation
  const estimation = useMemo(() => {
    if (!bbox || !params.scale) return null;
    return estimateWorldSize(bbox, params.scale, params.surfaceThickness);
  }, [bbox, params.scale, params.surfaceThickness]);

  return (
    <div
      className="flex flex-col gap-4 rounded-xl p-4"
      style={{
        background: 'var(--bg-elevated)',
        border: '1px solid var(--border-subtle)',
      }}
    >
      {/* Header */}
      <div className="flex items-center justify-between">
        <span
          className="text-[10px] font-semibold uppercase"
          style={{ color: 'var(--accent-gold)', letterSpacing: '0.18em' }}
        >
          Export
        </span>
      </div>

      {/* Divider */}
      <div style={{ height: 1, background: 'var(--border-subtle)' }} />

      {/* Conversion parameters form */}
      <ConversionParametersForm
        params={params}
        onParamsChange={updateParams}
        featureFilter={featureFilter}
        onFilterChange={onFilterChange}
        spawnPoint={spawnPoint}
        spawnMode={spawnMode}
        onSpawnModeToggle={onSpawnModeToggle}
        hasBbox={!!bbox}
        disabled={isRunning}
      />

      {/* Bbox size estimation */}
      {estimation && (
        <>
          <div style={{ height: 1, background: 'var(--border-subtle)' }} />
          <div
            className="flex flex-col gap-1.5 rounded-lg px-3 py-2.5"
            style={{
              background: 'var(--bg-hover)',
              border: '1px solid var(--border-subtle)',
            }}
          >
            <span
              className="text-[10px] font-semibold uppercase"
              style={{ color: 'var(--text-muted)', letterSpacing: '0.12em' }}
            >
              Estimated World Size
            </span>
            <div
              className="flex flex-col gap-1 text-[11px]"
              style={{
                fontFamily: "'JetBrains Mono', ui-monospace, monospace",
                color: 'var(--text-secondary)',
              }}
            >
              <div className="flex justify-between">
                <span>Blocks</span>
                <span>{estimation.widthBlocks.toLocaleString()} x {estimation.depthBlocks.toLocaleString()}</span>
              </div>
              <div className="flex justify-between">
                <span>Chunks</span>
                <span
                  style={{
                    color:
                      estimation.chunks > 500_000
                        ? 'var(--error, #F44336)'
                        : estimation.chunks > 100_000
                          ? 'var(--warning, #FFC107)'
                          : 'var(--text-secondary)',
                  }}
                >
                  ~{estimation.chunks.toLocaleString()}
                </span>
              </div>
              <div className="flex justify-between">
                <span>Est. File Size</span>
                <span>~{formatBytes(estimation.fileSizeBytes)}</span>
              </div>
            </div>
            {estimation.chunks > 500_000 && (
              <p className="text-[10px] leading-relaxed" style={{ color: 'var(--error, #F44336)' }}>
                Very large world (~{formatBytes(estimation.fileSizeBytes)}) — conversion may take hours or fail due to disk space
              </p>
            )}
            {estimation.chunks > 100_000 && estimation.chunks <= 500_000 && (
              <p className="text-[10px] leading-relaxed" style={{ color: 'var(--warning, #FFC107)' }}>
                Large world (~{formatBytes(estimation.fileSizeBytes)}) — conversion may take a while
              </p>
            )}
          </div>
        </>
      )}

      {/* Divider */}
      <div style={{ height: 1, background: 'var(--border-subtle)' }} />

      {/* Action buttons */}
      <ConversionControls
        conversionState={conversionState}
        hasBbox={!!bbox}
        hasSourceFile={!!sourceFile}
        terrainOnly={params.terrainOnly}
        onConvert={handleConvert}
        onFetchConvert={handleFetchConvert}
        onTerrainConvert={handleTerrainConvert}
      />

      {/* Progress and download */}
      <DownloadProgress
        conversionState={conversionState}
        progress={progress}
        message={message}
        downloadUrl={downloadUrl}
        downloadFilename={downloadFilename}
        error={error}
        downloadProgress={downloadProgress}
        downloadTotal={downloadTotal}
        isDownloading={isDownloading}
        onReset={reset}
      />
    </div>
  );
}
