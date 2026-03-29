'use client';

import { useEffect, useState } from 'react';

export function useTheme() {
  // Initialize to 'dark' (matches SSR) — sync from localStorage after mount
  const [theme, setTheme] = useState<'dark' | 'light'>('dark');

  useEffect(() => {
    const stored = localStorage.getItem('osm-theme') as 'dark' | 'light' | null;
    if (stored) {
      // eslint-disable-next-line react-hooks/set-state-in-effect -- intentional: sync from localStorage after mount, SSR-safe
      setTheme(stored);
      document.documentElement.setAttribute('data-theme', stored);
    }
  }, []);

  const toggleTheme = () => {
    const next = theme === 'dark' ? 'light' : 'dark';
    setTheme(next);
    localStorage.setItem('osm-theme', next);
    document.documentElement.setAttribute('data-theme', next);
  };

  return { theme, toggleTheme };
}
