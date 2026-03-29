'use client';

/**
 * ConversionParametersForm
 *
 * Renders all conversion parameter fields extracted from ExportPanel:
 * preset selector, world name, scale, building height, sea level,
 * wall-straighten threshold, signs/POI toggles, elevation, terrain-only,
 * spawn point display, and feature-filter switches.
 */

import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { Switch } from '@/components/ui/switch';
import { PRESETS, type Preset } from '@/lib/presets';
import type { FeatureFilter } from '@/lib/overpass';

interface SpawnPoint {
  lat: number;
  lon: number;
}

export interface ConversionParameters {
  worldName: string;
  scale: number;
  buildingHeight: number;
  seaLevel: number;
  signs: boolean;
  addressSigns: boolean;
  poiMarkers: boolean;
  poiDecorations: boolean;
  natureDecorations: boolean;
  useElevation: boolean;
  verticalScale: number;
  elevationSmoothing: number;
  wallStraightenThreshold: number;
  surfaceThickness: number;
  terrainOnly: boolean;
  activePreset: string;
}

interface ConversionParametersFormProps {
  params: ConversionParameters;
  onParamsChange: (updated: Partial<ConversionParameters>) => void;
  featureFilter: FeatureFilter;
  onFilterChange?: (filter: FeatureFilter) => void;
  spawnPoint: SpawnPoint | null;
  spawnMode: boolean;
  onSpawnModeToggle?: () => void;
  hasBbox: boolean;
  disabled: boolean;
}

/** Inline toggle button matching the existing ExportPanel style. */
function ToggleButton({
  checked,
  onChange,
  disabled,
}: {
  checked: boolean;
  onChange: (next: boolean) => void;
  disabled: boolean;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      onClick={() => { if (!disabled) onChange(!checked); }}
      disabled={disabled}
      className="relative shrink-0 rounded-full transition-all"
      style={{
        width: 36,
        height: 20,
        background: checked ? 'var(--accent-cyan, #4db8d4)' : 'var(--bg-hover)',
        border: checked
          ? '1px solid rgba(77, 184, 212, 0.6)'
          : '1px solid var(--border-medium)',
        cursor: disabled ? 'not-allowed' : 'pointer',
        boxShadow: checked ? '0 0 8px rgba(77, 184, 212, 0.3)' : 'none',
        opacity: disabled ? 0.5 : 1,
        transition: 'background 0.2s, box-shadow 0.2s',
      }}
    >
      <span
        className="absolute rounded-full"
        style={{
          width: 14,
          height: 14,
          top: 2,
          left: checked ? 18 : 2,
          background: checked ? '#fff' : 'var(--text-muted)',
          transition: 'left 0.2s',
        }}
      />
    </button>
  );
}

export function ConversionParametersForm({
  params,
  onParamsChange,
  featureFilter,
  onFilterChange,
  spawnPoint,
  spawnMode,
  onSpawnModeToggle,
  hasBbox,
  disabled,
}: ConversionParametersFormProps) {
  const markCustom = () => onParamsChange({ activePreset: 'Custom' });

  const handlePresetChange = (presetName: string) => {
    if (presetName === 'Custom') {
      onParamsChange({ activePreset: 'Custom' });
      return;
    }
    const preset = PRESETS.find((p: Preset) => p.name === presetName);
    if (preset) {
      onParamsChange({
        activePreset: presetName,
        scale: preset.scale,
        buildingHeight: preset.buildingHeight,
        seaLevel: preset.seaLevel,
        signs: preset.signs,
      });
    }
  };

  const handleFilterToggle = (key: keyof FeatureFilter, value: boolean) => {
    onFilterChange?.({ ...featureFilter, [key]: value });
  };

  return (
    <div className="flex flex-col gap-3">
      {/* Preset dropdown */}
      <div className="flex flex-col gap-1.5">
        <Label
          htmlFor="preset"
          className="text-[11px] font-medium"
          style={{ color: 'var(--text-secondary)' }}
        >
          Preset
        </Label>
        <select
          id="preset"
          value={params.activePreset}
          onChange={(e) => handlePresetChange(e.target.value)}
          disabled={disabled}
          className="rounded-md px-3 py-2 text-[0.8125rem]"
          style={{
            background: 'var(--bg-hover)',
            border: '1px solid var(--border-medium)',
            color: 'var(--text-primary)',
            cursor: disabled ? 'not-allowed' : 'pointer',
            opacity: disabled ? 0.5 : 1,
          }}
        >
          <option value="Custom">Custom</option>
          {PRESETS.map((p: Preset) => (
            <option key={p.name} value={p.name}>
              {p.name}
            </option>
          ))}
        </select>
      </div>

      {/* World Name */}
      <div className="flex flex-col gap-1.5">
        <Label
          htmlFor="world-name"
          className="text-[11px] font-medium"
          style={{ color: 'var(--text-secondary)' }}
        >
          World Name
        </Label>
        <Input
          id="world-name"
          type="text"
          value={params.worldName}
          onChange={(e) => onParamsChange({ worldName: e.target.value })}
          disabled={disabled}
          placeholder="OSM World"
          style={{
            background: 'var(--bg-hover)',
            border: '1px solid var(--border-medium)',
            color: 'var(--text-primary)',
            fontSize: '0.8125rem',
          }}
        />
      </div>

      {/* Scale */}
      <div className="flex flex-col gap-1.5">
        <Label htmlFor="scale" className="text-[11px] font-medium" style={{ color: 'var(--text-secondary)' }}>
          Scale
        </Label>
        <Input
          id="scale"
          type="number"
          value={params.scale}
          onChange={(e) => { onParamsChange({ scale: Number(e.target.value) }); markCustom(); }}
          disabled={disabled}
          min={0.1}
          max={10}
          step={0.1}
          style={{ background: 'var(--bg-hover)', border: '1px solid var(--border-medium)', color: 'var(--text-primary)', fontSize: '0.8125rem' }}
        />
      </div>

      {/* Building Height */}
      <div className="flex flex-col gap-1.5">
        <Label htmlFor="building-height" className="text-[11px] font-medium" style={{ color: 'var(--text-secondary)' }}>
          Building Height (blocks)
        </Label>
        <Input
          id="building-height"
          type="number"
          value={params.buildingHeight}
          onChange={(e) => { onParamsChange({ buildingHeight: Number(e.target.value) }); markCustom(); }}
          disabled={disabled}
          min={1}
          max={64}
          step={1}
          style={{ background: 'var(--bg-hover)', border: '1px solid var(--border-medium)', color: 'var(--text-primary)', fontSize: '0.8125rem' }}
        />
      </div>

      {/* Wall Straighten Threshold */}
      <div className="flex flex-col gap-1.5">
        <Label htmlFor="wall-straighten" className="text-[11px] font-medium" style={{ color: 'var(--text-secondary)' }}>
          Wall Straighten (blocks)
        </Label>
        <Input
          id="wall-straighten"
          type="number"
          value={params.wallStraightenThreshold}
          onChange={(e) => { onParamsChange({ wallStraightenThreshold: Number(e.target.value) }); markCustom(); }}
          disabled={disabled}
          min={0}
          max={10}
          step={1}
          title="Snaps nearly-straight walls to axis-aligned. 0 = off."
          style={{ background: 'var(--bg-hover)', border: '1px solid var(--border-medium)', color: 'var(--text-primary)', fontSize: '0.8125rem' }}
        />
      </div>

      {/* Surface Thickness */}
      <div className="flex flex-col gap-1.5">
        <Label htmlFor="surface-thickness" className="text-[11px] font-medium" style={{ color: 'var(--text-secondary)' }}>
          Surface Thickness
        </Label>
        <Input
          id="surface-thickness"
          type="number"
          value={params.surfaceThickness}
          onChange={(e) => { onParamsChange({ surfaceThickness: Number(e.target.value) }); markCustom(); }}
          disabled={disabled}
          min={1}
          max={128}
          step={1}
          title="Terrain fill depth below surface. Lower = faster conversion and smaller worlds. Default 4."
          style={{ background: 'var(--bg-hover)', border: '1px solid var(--border-medium)', color: 'var(--text-primary)', fontSize: '0.8125rem' }}
        />
      </div>

      {/* Sea Level */}
      <div className="flex flex-col gap-1.5">
        <Label htmlFor="sea-level" className="text-[11px] font-medium" style={{ color: 'var(--text-secondary)' }}>
          Sea Level (Y)
        </Label>
        <Input
          id="sea-level"
          type="number"
          value={params.seaLevel}
          onChange={(e) => { onParamsChange({ seaLevel: Number(e.target.value) }); markCustom(); }}
          disabled={disabled}
          min={0}
          max={255}
          step={1}
          style={{ background: 'var(--bg-hover)', border: '1px solid var(--border-medium)', color: 'var(--text-primary)', fontSize: '0.8125rem' }}
        />
      </div>

      {/* Signs group */}
      {(
        [
          ['Street Names', 'signs', 'Road name signs every ~50 blocks'],
          ['Address Signs', 'addressSigns', 'Hanging signs on building facades'],
          ['POI Markers', 'poiMarkers', 'Signs at amenities, shops & tourism'],
          ['POI Decorations', 'poiDecorations', 'Decorative blocks at POI locations'],
          ['Nature', 'natureDecorations', 'Individual trees from map data'],
        ] as [string, keyof ConversionParameters, string][]
      ).map(([label, key, hint]) => (
        <div key={key} className="flex flex-col gap-1.5">
          <div className="flex items-center justify-between">
            <Label className="text-[11px] font-medium" style={{ color: 'var(--text-secondary)' }}>
              {label}
            </Label>
            <ToggleButton
              checked={!!params[key]}
              onChange={(next) => { onParamsChange({ [key]: next }); markCustom(); }}
              disabled={disabled}
            />
          </div>
          {!!params[key] && (
            <p className="text-[10px] leading-relaxed" style={{ color: 'var(--text-muted)' }}>
              {hint}
            </p>
          )}
        </div>
      ))}

      {/* Elevation */}
      <div className="flex flex-col gap-1.5">
        <div className="flex items-center justify-between">
          <Label className="text-[11px] font-medium" style={{ color: 'var(--text-secondary)' }}>
            Real-World Elevation
          </Label>
          <ToggleButton
            checked={params.useElevation}
            onChange={(next) => onParamsChange({ useElevation: next })}
            disabled={disabled}
          />
        </div>
        {params.useElevation && (
          <>
            <p className="text-[10px] leading-relaxed" style={{ color: 'var(--text-muted)' }}>
              Downloads SRTM elevation tiles (~26 MB/tile) for hilly terrain
            </p>
            <div className="flex flex-col gap-1 mt-1">
              <Label htmlFor="vertical-scale" className="text-[11px]" style={{ color: 'var(--text-secondary)' }}>
                Vertical Scale
              </Label>
              <Input
                id="vertical-scale"
                type="number"
                value={params.verticalScale}
                onChange={(e) => onParamsChange({ verticalScale: Number(e.target.value) })}
                disabled={disabled}
                min={0.1}
                max={10}
                step={0.1}
                style={{ background: 'var(--bg-hover)', border: '1px solid var(--border-medium)', color: 'var(--text-primary)', fontSize: '0.8125rem' }}
              />
            </div>
            <div className="flex flex-col gap-1 mt-1">
              <Label htmlFor="elevation-smoothing" className="text-[11px]" style={{ color: 'var(--text-secondary)' }}>
                Elevation Smoothing
              </Label>
              <Input
                id="elevation-smoothing"
                type="number"
                value={params.elevationSmoothing}
                onChange={(e) => { onParamsChange({ elevationSmoothing: Number(e.target.value) }); markCustom(); }}
                disabled={disabled}
                min={0}
                max={5}
                step={1}
                title="Smoothing radius to reduce elevation jitter. 0 = raw terrain, 1 = gentle (default), 2+ = aggressive."
                style={{ background: 'var(--bg-hover)', border: '1px solid var(--border-medium)', color: 'var(--text-primary)', fontSize: '0.8125rem' }}
              />
            </div>
          </>
        )}
      </div>

      {/* Terrain Only toggle — only useful when a bbox is drawn */}
      {hasBbox && (
        <div className="flex flex-col gap-1.5">
          <div className="flex items-center justify-between">
            <Label className="text-[11px] font-medium" style={{ color: 'var(--text-secondary)' }}>
              Terrain Only
            </Label>
            <ToggleButton
              checked={params.terrainOnly}
              onChange={(next) => onParamsChange({ terrainOnly: next })}
              disabled={disabled}
            />
          </div>
          {params.terrainOnly && (
            <p className="text-[10px] leading-relaxed" style={{ color: 'var(--text-muted)' }}>
              Generate terrain from SRTM elevation only — no roads, buildings, or OSM data
            </p>
          )}
        </div>
      )}

      {/* Spawn Point row */}
      <div className="flex flex-col gap-1.5">
        <Label className="text-[11px] font-medium" style={{ color: 'var(--text-secondary)' }}>
          Spawn Point
        </Label>
        <div className="flex gap-2">
          <div
            className="flex-1 rounded-md px-3 py-2 text-xs"
            style={{
              background: 'var(--bg-hover)',
              border: '1px solid var(--border-subtle)',
              color: spawnPoint ? 'var(--layer-landuse)' : 'var(--text-muted)',
              fontFamily: "'JetBrains Mono', ui-monospace, monospace",
              fontSize: '0.75rem',
            }}
          >
            {spawnPoint
              ? `${spawnPoint.lat.toFixed(5)}, ${spawnPoint.lon.toFixed(5)}`
              : 'not set'}
          </div>
          {onSpawnModeToggle && (
            <button
              onClick={onSpawnModeToggle}
              className="shrink-0 rounded-md px-3 py-2 text-xs font-medium transition-all"
              style={{
                background: spawnMode ? 'rgba(232, 184, 77, 0.18)' : 'var(--bg-hover)',
                color: spawnMode ? 'var(--accent-gold)' : 'var(--text-secondary)',
                border: spawnMode
                  ? '1px solid rgba(232, 184, 77, 0.35)'
                  : '1px solid var(--border-medium)',
                cursor: 'pointer',
                whiteSpace: 'nowrap',
              }}
            >
              {spawnMode ? 'Click map\u2026' : 'Set'}
            </button>
          )}
        </div>
      </div>

      {/* Feature Toggles */}
      <div className="flex flex-col gap-1.5">
        <Label className="text-[11px] font-medium" style={{ color: 'var(--text-secondary)' }}>
          Features
        </Label>
        {(
          [
            ['roads', 'Roads'],
            ['buildings', 'Buildings'],
            ['water', 'Water'],
            ['landuse', 'Landuse'],
            ['railways', 'Railways'],
          ] as [keyof FeatureFilter, string][]
        ).map(([key, label]) => (
          <div key={key} className="flex items-center justify-between">
            <Label
              htmlFor={`filter-${key}`}
              className="text-[11px]"
              style={{ color: 'var(--text-secondary)' }}
            >
              {label}
            </Label>
            <Switch
              id={`filter-${key}`}
              checked={featureFilter[key]}
              onCheckedChange={(v) => handleFilterToggle(key, v)}
              disabled={disabled}
            />
          </div>
        ))}
      </div>
    </div>
  );
}
