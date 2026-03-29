'use client';

import { useState } from 'react';
import { ChevronDown, ChevronUp } from 'lucide-react';
import type { LayerConfig } from '@/components/LayerPanel';

interface MapLegendProps {
  layers: LayerConfig[];
  hasData: boolean;
}

export function MapLegend({ layers, hasData }: MapLegendProps) {
  const [collapsed, setCollapsed] = useState(false);

  if (!hasData) return null;

  const visibleLayers = layers.filter((l) => l.visible && l.count > 0);
  if (visibleLayers.length === 0) return null;

  return (
    <div
      className="absolute bottom-4 right-4 z-10 rounded-lg"
      style={{
        background: 'rgba(14, 16, 24, 0.85)',
        backdropFilter: 'blur(8px)',
        border: '1px solid var(--border-subtle)',
        minWidth: 120,
      }}
    >
      <button
        onClick={() => setCollapsed((prev) => !prev)}
        className="flex w-full items-center justify-between px-3 py-2"
        style={{ cursor: 'pointer', background: 'transparent', border: 'none' }}
      >
        <span
          className="text-[9px] font-semibold uppercase"
          style={{ color: 'var(--text-muted)', letterSpacing: '0.15em' }}
        >
          Legend
        </span>
        {collapsed ? (
          <ChevronUp className="h-3 w-3" style={{ color: 'var(--text-muted)' }} />
        ) : (
          <ChevronDown className="h-3 w-3" style={{ color: 'var(--text-muted)' }} />
        )}
      </button>

      {!collapsed && (
        <div className="flex flex-col gap-1.5 px-3 pb-2.5">
          {visibleLayers.map((layer) => (
            <div key={layer.id} className="flex items-center gap-2">
              <span
                className="h-2 w-2 shrink-0 rounded-full"
                style={{ background: layer.color, boxShadow: `0 0 4px ${layer.color}50` }}
              />
              <span className="text-[11px]" style={{ color: 'var(--text-secondary)' }}>
                {layer.name}
              </span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
