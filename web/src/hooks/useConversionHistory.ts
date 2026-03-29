'use client';

import { useState, useCallback, useEffect } from 'react';

export interface HistoryEntry {
  id: string;
  worldName: string;
  timestamp: number;
  settings: {
    scale: number;
    buildingHeight: number;
    seaLevel: number;
    signs: boolean;
  };
  bbox: [number, number, number, number] | null;
}

const STORAGE_KEY = 'osm-conversion-history';
const MAX_ENTRIES = 20;

export function useConversionHistory() {
  // Initialize to empty (matches SSR) — load from localStorage after mount
  const [history, setHistory] = useState<HistoryEntry[]>([]);

  useEffect(() => {
    try {
      const stored = localStorage.getItem(STORAGE_KEY);
      // eslint-disable-next-line react-hooks/set-state-in-effect -- intentional: sync from localStorage after mount, SSR-safe
      if (stored) setHistory(JSON.parse(stored));
    } catch { /* ignore parse errors */ }
  }, []);

  const addEntry = useCallback((entry: Omit<HistoryEntry, 'id' | 'timestamp'>) => {
    setHistory((prev) => {
      const newEntry: HistoryEntry = {
        ...entry,
        id: crypto.randomUUID(),
        timestamp: Date.now(),
      };
      const updated = [newEntry, ...prev].slice(0, MAX_ENTRIES);
      localStorage.setItem(STORAGE_KEY, JSON.stringify(updated));
      return updated;
    });
  }, []);

  const clearHistory = useCallback(() => {
    setHistory([]);
    localStorage.removeItem(STORAGE_KEY);
  }, []);

  return { history, addEntry, clearHistory };
}
