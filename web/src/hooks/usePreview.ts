'use client';

import { useState, useCallback } from 'react';

export interface PreviewBlock {
  x: number;
  z: number;
  y: number;
  type: string;
}

export interface PreviewBounds {
  min_x: number;
  max_x: number;
  min_z: number;
  max_z: number;
}

export interface PreviewSpawn {
  x: number;
  y: number;
  z: number;
}

export type PreviewState = 'idle' | 'loading' | 'ready' | 'error';

export function usePreview() {
  const [state, setState] = useState<PreviewState>('idle');
  const [blocks, setBlocks] = useState<PreviewBlock[]>([]);
  const [bounds, setBounds] = useState<PreviewBounds | null>(null);
  const [spawn, setSpawn] = useState<PreviewSpawn | null>(null);
  const [error, setError] = useState<string | null>(null);

  const generatePreview = useCallback(async (file: File, options: Record<string, unknown>): Promise<boolean> => {
    setState('loading');
    setError(null);
    try {
      const form = new FormData();
      form.append('file', file);
      form.append('options', JSON.stringify(options));

      const res = await fetch('/api/preview', {
        method: 'POST',
        body: form,
        signal: AbortSignal.timeout(60_000),
      });

      if (!res.ok) {
        const data = await res.json().catch(() => ({ error: res.statusText }));
        throw new Error((data as { error?: string }).error || `HTTP ${res.status}`);
      }

      const data = await res.json() as {
        blocks: PreviewBlock[];
        bounds: PreviewBounds;
        spawn: PreviewSpawn;
      };
      setBlocks(data.blocks);
      setBounds(data.bounds);
      setSpawn(data.spawn);
      setState('ready');
      return true;
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Preview failed');
      setState('error');
      return false;
    }
  }, []);

  const generatePreviewFromBbox = useCallback(async (bbox: [number, number, number, number]): Promise<boolean> => {
    setState('loading');
    setError(null);
    try {
      const res = await fetch('/api/fetch-block-preview', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ bbox }),
        signal: AbortSignal.timeout(300_000),
      });

      if (!res.ok) {
        const data = await res.json().catch(() => ({ error: res.statusText }));
        throw new Error((data as { error?: string }).error || `HTTP ${res.status}`);
      }

      const data = await res.json() as {
        blocks: PreviewBlock[];
        bounds: PreviewBounds;
        spawn: PreviewSpawn;
      };
      setBlocks(data.blocks);
      setBounds(data.bounds);
      setSpawn(data.spawn);
      setState('ready');
      return true;
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Preview failed');
      setState('error');
      return false;
    }
  }, []);

  const reset = useCallback(() => {
    setState('idle');
    setBlocks([]);
    setBounds(null);
    setSpawn(null);
    setError(null);
  }, []);

  return { state, blocks, bounds, spawn, error, generatePreview, generatePreviewFromBbox, reset };
}
