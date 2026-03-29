'use client';

import React from 'react';
import { Eye, EyeOff } from 'lucide-react';
import { Badge } from '@/components/ui/badge';

export interface LayerConfig {
  id: string;
  name: string;
  color: string;
  count: number;
  visible: boolean;
}

interface LayerPanelProps {
  layers: LayerConfig[];
  onToggle: (id: string) => void;
}

export function LayerPanel({ layers, onToggle }: LayerPanelProps) {
  return (
    <div className="px-4 py-4" style={{ borderBottom: '1px solid var(--border-subtle)' }}>
      {/* Section header */}
      <p
        className="mb-3 text-[10px] font-semibold uppercase"
        style={{ color: 'var(--text-muted)', letterSpacing: '0.18em' }}
      >
        Layers
      </p>

      <ul className="flex flex-col gap-0.5">
        {layers.map((layer) => (
          <li
            key={layer.id}
            className="flex items-center gap-3 rounded-md px-2 py-2 transition-colors"
            style={{ cursor: 'default' }}
            onMouseEnter={(e) => {
              (e.currentTarget as HTMLLIElement).style.background = 'var(--bg-hover)';
            }}
            onMouseLeave={(e) => {
              (e.currentTarget as HTMLLIElement).style.background = 'transparent';
            }}
          >
            {/* Color indicator dot */}
            <span
              className="h-2 w-2 shrink-0 rounded-full"
              style={{
                background: layer.visible ? layer.color : 'var(--text-muted)',
                boxShadow: layer.visible ? `0 0 5px ${layer.color}70` : 'none',
                transition: 'background 0.15s, box-shadow 0.15s',
              }}
            />

            {/* Layer name */}
            <span
              className="flex-1 text-sm font-medium transition-colors"
              style={{ color: layer.visible ? 'var(--text-primary)' : 'var(--text-muted)' }}
            >
              {layer.name}
            </span>

            {/* Count badge */}
            <Badge
              variant="secondary"
              className="min-w-[36px] justify-center rounded border-none text-[10px]"
              style={{
                background: 'var(--bg-elevated)',
                color: 'var(--text-muted)',
                fontFamily: "'JetBrains Mono', ui-monospace, monospace",
              }}
            >
              {layer.count.toLocaleString()}
            </Badge>

            {/* Eye toggle button */}
            <button
              onClick={() => onToggle(layer.id)}
              className="flex items-center justify-center rounded p-0.5 transition-colors"
              style={{ color: layer.visible ? 'var(--text-secondary)' : 'var(--text-muted)' }}
              aria-label={layer.visible ? `Hide ${layer.name}` : `Show ${layer.name}`}
            >
              {layer.visible ? (
                <Eye className="h-4 w-4" />
              ) : (
                <EyeOff className="h-4 w-4" />
              )}
            </button>
          </li>
        ))}
      </ul>
    </div>
  );
}
