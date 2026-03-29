'use client';

import { useState } from 'react';
import { ChevronDown, ChevronRight, Clock } from 'lucide-react';
import type { HistoryEntry } from '@/hooks/useConversionHistory';

interface HistoryPanelProps {
  history: HistoryEntry[];
  onLoadSettings?: (entry: HistoryEntry) => void;
}

export function HistoryPanel({ history, onLoadSettings }: HistoryPanelProps) {
  const [expanded, setExpanded] = useState(false);

  if (history.length === 0) return null;

  return (
    <div className="px-3 py-3" style={{ borderBottom: '1px solid var(--border-subtle)' }}>
      <button
        onClick={() => setExpanded((prev) => !prev)}
        className="flex w-full items-center gap-2"
        style={{ background: 'transparent', border: 'none', cursor: 'pointer' }}
      >
        {expanded ? (
          <ChevronDown className="h-3 w-3" style={{ color: 'var(--text-muted)' }} />
        ) : (
          <ChevronRight className="h-3 w-3" style={{ color: 'var(--text-muted)' }} />
        )}
        <span
          className="text-[10px] font-semibold uppercase"
          style={{ color: 'var(--text-muted)', letterSpacing: '0.18em' }}
        >
          History
        </span>
        <span
          className="ml-auto text-[10px]"
          style={{ color: 'var(--text-muted)', fontFamily: "'JetBrains Mono', monospace" }}
        >
          {history.length}
        </span>
      </button>

      {expanded && (
        <div className="mt-2 flex flex-col gap-1.5">
          {history.slice(0, 10).map((entry) => (
            <div
              key={entry.id}
              className="flex items-center justify-between rounded-md px-2 py-1.5"
              style={{ background: 'var(--bg-elevated)' }}
            >
              <div className="flex flex-col gap-0.5">
                <span className="text-xs font-medium" style={{ color: 'var(--text-primary)' }}>
                  {entry.worldName}
                </span>
                <span className="flex items-center gap-1 text-[10px]" style={{ color: 'var(--text-muted)' }}>
                  <Clock className="h-2.5 w-2.5" />
                  {new Date(entry.timestamp).toLocaleDateString()}
                </span>
              </div>
              {onLoadSettings && (
                <button
                  onClick={() => onLoadSettings(entry)}
                  className="rounded px-2 py-1 text-[10px] font-medium"
                  style={{
                    background: 'var(--bg-hover)',
                    color: 'var(--accent-gold)',
                    border: '1px solid var(--border-subtle)',
                    cursor: 'pointer',
                  }}
                >
                  Load
                </button>
              )}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
