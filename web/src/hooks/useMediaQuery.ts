'use client';

import { useEffect, useState } from 'react';

export function useMediaQuery(query: string): boolean {
  // Initialize to false (matches SSR) — sync from window.matchMedia after mount
  const [matches, setMatches] = useState(false);

  useEffect(() => {
    const mql = window.matchMedia(query);
    // eslint-disable-next-line react-hooks/set-state-in-effect -- intentional: sync initial value after mount, SSR-safe
    setMatches(mql.matches);
    const handler = (e: MediaQueryListEvent) => setMatches(e.matches);
    mql.addEventListener('change', handler);
    return () => mql.removeEventListener('change', handler);
  }, [query]);

  return matches;
}
