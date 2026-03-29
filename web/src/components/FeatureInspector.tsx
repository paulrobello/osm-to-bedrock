'use client';

import { ScrollArea } from '@/components/ui/scroll-area';

interface FeatureProps {
  feature: {
    properties: Record<string, string>;
    geometry_type: string;
  } | null;
}

// Layer color tokens mapped to CSS vars
const TYPE_BADGE_COLORS: Record<string, { bg: string; text: string }> = {
  road:     { bg: 'rgba(144,152,176,0.12)', text: 'var(--layer-roads)' },
  building: { bg: 'rgba(232,93,93,0.12)',   text: 'var(--layer-buildings)' },
  water:    { bg: 'rgba(77,184,212,0.12)',   text: 'var(--layer-water)' },
  landuse:  { bg: 'rgba(107,201,93,0.12)',   text: 'var(--layer-landuse)' },
};

function TypeBadge({ type }: { type: string }) {
  const colors = TYPE_BADGE_COLORS[type] ?? {
    bg: 'rgba(139,124,246,0.12)',
    text: 'var(--accent-purple)',
  };
  return (
    <span
      className="inline-flex items-center rounded-full px-2.5 py-0.5 text-[10px] font-bold font-mono uppercase tracking-widest"
      style={{
        background: colors.bg,
        color: colors.text,
        border: `1px solid ${colors.text}`,
        borderColor: `color-mix(in srgb, ${colors.text} 30%, transparent)`,
      }}
    >
      {type}
    </span>
  );
}

export function FeatureInspector({ feature }: FeatureProps) {
  if (!feature) {
    return (
      <div
        className="rounded-lg p-4"
        style={{ background: 'var(--bg-surface)', border: '1px solid var(--border-subtle)' }}
      >
        <p
          className="text-[10px] uppercase tracking-[0.18em] font-semibold mb-3"
          style={{ color: 'var(--text-muted)', fontFamily: 'var(--font-sans, DM Sans, sans-serif)' }}
        >
          Selected Feature
        </p>
        <div className="flex items-center gap-2">
          {/* Crosshair icon */}
          <svg
            width="13"
            height="13"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.5"
            strokeLinecap="round"
            style={{ color: 'var(--text-muted)', flexShrink: 0 }}
          >
            <circle cx="12" cy="12" r="3" />
            <line x1="12" y1="2" x2="12" y2="6" />
            <line x1="12" y1="18" x2="12" y2="22" />
            <line x1="2" y1="12" x2="6" y2="12" />
            <line x1="18" y1="12" x2="22" y2="12" />
          </svg>
          <p
            className="text-xs italic"
            style={{ color: 'var(--text-muted)', fontFamily: 'var(--font-sans, DM Sans, sans-serif)' }}
          >
            Click a feature on the map
          </p>
        </div>
      </div>
    );
  }

  const { properties, geometry_type } = feature;

  // Separate internal tags (starting with _) from OSM tags
  const internalTags: Record<string, string> = {};
  const osmTags: Record<string, string> = {};

  for (const [key, value] of Object.entries(properties)) {
    if (key.startsWith('_')) {
      internalTags[key] = value;
    } else {
      osmTags[key] = value;
    }
  }

  const featureType = internalTags['_type'] ?? '';
  const nodeCount = internalTags['_node_count'];
  const osmTagEntries = Object.entries(osmTags);
  const internalTagEntries = Object.entries(internalTags);

  return (
    <div
      className="rounded-lg p-4 flex flex-col gap-3"
      style={{ background: 'var(--bg-surface)', border: '1px solid var(--border-subtle)' }}
    >
      {/* Header */}
      <p
        className="text-[10px] uppercase tracking-[0.18em] font-semibold"
        style={{ color: 'var(--text-muted)', fontFamily: 'var(--font-sans, DM Sans, sans-serif)' }}
      >
        Selected Feature
      </p>

      {/* Meta row: type badge + geometry info */}
      <div className="flex flex-wrap items-center gap-2">
        {featureType && <TypeBadge type={featureType} />}
        <span
          className="text-[11px] tabular-nums"
          style={{
            color: 'var(--text-secondary)',
            fontFamily: "'JetBrains Mono', monospace",
          }}
        >
          {geometry_type}
          {nodeCount ? (
            <>
              <span style={{ color: 'var(--text-muted)' }}> · </span>
              {nodeCount}
              <span style={{ color: 'var(--text-muted)' }}> nodes</span>
            </>
          ) : ''}
        </span>
      </div>

      {/* OSM Tags */}
      {osmTagEntries.length > 0 && (
        <div>
          <p
            className="text-[9px] uppercase tracking-[0.16em] mb-2 font-semibold"
            style={{ color: 'var(--text-muted)', fontFamily: 'var(--font-sans, DM Sans, sans-serif)' }}
          >
            OSM Tags
          </p>
          <ScrollArea className="max-h-52">
            <div className="flex flex-col pr-1" style={{ gap: '1px' }}>
              {osmTagEntries.map(([key, value], i) => (
                <div
                  key={key}
                  className="flex flex-wrap items-baseline gap-x-1.5 gap-y-0.5 rounded px-2 py-1 text-[11px] leading-5"
                  style={{
                    background: i % 2 === 0
                      ? 'rgba(255,255,255,0.025)'
                      : 'transparent',
                    fontFamily: "'JetBrains Mono', monospace",
                  }}
                >
                  <span style={{ color: 'var(--accent-cyan)' }}>{key}</span>
                  <span style={{ color: 'var(--text-muted)', fontSize: '10px' }}>=</span>
                  <span style={{ color: 'var(--text-primary)' }}>{value}</span>
                </div>
              ))}
            </div>
          </ScrollArea>
        </div>
      )}

      {/* Internal Tags */}
      {internalTagEntries.length > 0 && (
        <div>
          <p
            className="text-[9px] uppercase tracking-[0.16em] mb-2 font-semibold"
            style={{ color: 'var(--text-muted)', fontFamily: 'var(--font-sans, DM Sans, sans-serif)' }}
          >
            Internal
          </p>
          <div className="flex flex-col" style={{ gap: '1px' }}>
            {internalTagEntries.map(([key, value]) => (
              <div
                key={key}
                className="flex flex-wrap items-baseline gap-x-1.5 gap-y-0.5 rounded px-2 py-0.5 text-[10px] leading-5"
                style={{
                  fontFamily: "'JetBrains Mono', monospace",
                  opacity: 0.65,
                }}
              >
                <span style={{ color: 'var(--text-secondary)' }}>{key}</span>
                <span style={{ color: 'var(--text-muted)', fontSize: '9px' }}>=</span>
                <span style={{ color: 'var(--text-secondary)' }}>{value}</span>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
