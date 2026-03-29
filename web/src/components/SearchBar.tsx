'use client';

import React, { useCallback, useEffect, useRef, useState } from 'react';
import { Search, X, Loader2 } from 'lucide-react';

interface NominatimResult {
  place_id: number;
  display_name: string;
  lat: string;
  lon: string;
  boundingbox: [string, string, string, string]; // [south, north, west, east]
}

interface SearchBarProps {
  onSelect: (bbox: [number, number, number, number], center: [number, number]) => void;
}

export function SearchBar({ onSelect }: SearchBarProps) {
  const [query, setQuery] = useState('');
  const [results, setResults] = useState<NominatimResult[]>([]);
  const [loading, setLoading] = useState(false);
  const [open, setOpen] = useState(false);
  const [activeIndex, setActiveIndex] = useState(-1);
  const [focused, setFocused] = useState(false);

  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const containerRef = useRef<HTMLDivElement | null>(null);
  const inputRef = useRef<HTMLInputElement | null>(null);

  // Close dropdown on outside click
  useEffect(() => {
    function handleClickOutside(e: MouseEvent) {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    }
    document.addEventListener('mousedown', handleClickOutside);
    return () => document.removeEventListener('mousedown', handleClickOutside);
  }, []);

  const search = useCallback(async (q: string) => {
    if (q.trim().length < 2) {
      setResults([]);
      setOpen(false);
      return;
    }

    setLoading(true);
    try {
      const res = await fetch(`/api/geocode?q=${encodeURIComponent(q.trim())}`);
      if (!res.ok) {
        setResults([]);
        return;
      }
      const data = (await res.json()) as NominatimResult[];
      setResults(Array.isArray(data) ? data : []);
      setOpen(true);
      setActiveIndex(-1);
    } catch {
      setResults([]);
    } finally {
      setLoading(false);
    }
  }, []);

  function handleInput(e: React.ChangeEvent<HTMLInputElement>) {
    const val = e.target.value;
    setQuery(val);

    if (debounceRef.current) clearTimeout(debounceRef.current);
    debounceRef.current = setTimeout(() => {
      search(val);
    }, 300);
  }

  function handleClear() {
    setQuery('');
    setResults([]);
    setOpen(false);
    setActiveIndex(-1);
    if (debounceRef.current) clearTimeout(debounceRef.current);
    inputRef.current?.focus();
  }

  function handleSelect(result: NominatimResult) {
    // Nominatim boundingbox: [south, north, west, east]
    const [south, north, west, east] = result.boundingbox.map(Number);
    const bbox: [number, number, number, number] = [south, west, north, east];
    const center: [number, number] = [parseFloat(result.lon), parseFloat(result.lat)];
    onSelect(bbox, center);
    setQuery(result.display_name);
    setOpen(false);
    setResults([]);
  }

  function handleKeyDown(e: React.KeyboardEvent<HTMLInputElement>) {
    if (!open || results.length === 0) return;

    switch (e.key) {
      case 'ArrowDown':
        e.preventDefault();
        setActiveIndex((i) => Math.min(i + 1, results.length - 1));
        break;
      case 'ArrowUp':
        e.preventDefault();
        setActiveIndex((i) => Math.max(i - 1, -1));
        break;
      case 'Enter':
        e.preventDefault();
        if (activeIndex >= 0 && activeIndex < results.length) {
          handleSelect(results[activeIndex]);
        }
        break;
      case 'Escape':
        e.preventDefault();
        setOpen(false);
        setActiveIndex(-1);
        break;
    }
  }

  return (
    <div ref={containerRef} className="relative w-full">
      {/* Input row */}
      <div
        className="relative flex h-9 items-center gap-2 rounded-lg px-3 transition-all duration-200"
        style={{
          background: 'var(--bg-elevated)',
          border: focused
            ? '1px solid var(--accent-cyan)'
            : '1px solid var(--border-medium)',
          boxShadow: focused ? '0 0 0 2px rgba(86,200,216,0.1)' : 'none',
        }}
      >
        {loading ? (
          <Loader2
            className="shrink-0 animate-spin transition-colors duration-200"
            size={14}
            style={{ color: 'var(--accent-cyan)' }}
          />
        ) : (
          <Search
            size={14}
            className="shrink-0 transition-colors duration-200"
            style={{ color: focused ? 'var(--accent-cyan)' : 'var(--text-muted)' }}
          />
        )}

        <input
          ref={inputRef}
          type="text"
          value={query}
          onChange={handleInput}
          onKeyDown={handleKeyDown}
          onFocus={() => {
            setFocused(true);
            if (results.length > 0) setOpen(true);
          }}
          onBlur={() => setFocused(false)}
          placeholder="Search places…"
          className="flex-1 bg-transparent font-[family-name:var(--font-dm-sans)] text-xs outline-none"
          style={{
            color: 'var(--text-primary)',
          }}
          autoComplete="off"
          spellCheck={false}
        />

        {query.length > 0 && (
          <button
            type="button"
            onClick={handleClear}
            className="shrink-0 rounded p-0.5 transition-colors"
            style={{ color: 'var(--text-muted)' }}
            aria-label="Clear search"
          >
            <X size={12} />
          </button>
        )}
      </div>

      {/* Dropdown */}
      {open && results.length > 0 && (
        <div
          className="absolute left-0 right-0 top-full z-50 mt-1 overflow-hidden rounded-lg"
          style={{
            background: 'var(--bg-elevated)',
            border: '1px solid var(--border-medium)',
            boxShadow: '0 8px 32px rgba(0,0,0,0.6), 0 2px 8px rgba(0,0,0,0.4)',
          }}
        >
          <ul role="listbox" className="py-1">
            {results.map((result, idx) => (
              <li
                key={result.place_id}
                role="option"
                aria-selected={idx === activeIndex}
                onMouseEnter={() => setActiveIndex(idx)}
                onMouseDown={(e) => {
                  // Prevent input blur before click fires
                  e.preventDefault();
                  handleSelect(result);
                }}
                className="cursor-pointer px-3 py-2 transition-colors"
                style={{
                  background: idx === activeIndex ? 'var(--bg-hover)' : 'transparent',
                  borderLeft:
                    idx === activeIndex
                      ? '2px solid var(--accent-cyan)'
                      : '2px solid transparent',
                  borderBottom:
                    idx < results.length - 1 ? '1px solid var(--border-subtle)' : 'none',
                }}
              >
                <div className="flex items-start gap-2" title={result.display_name}>
                  <Search
                    size={10}
                    className="mt-0.5 shrink-0"
                    style={{
                      color:
                        idx === activeIndex ? 'var(--accent-cyan)' : 'var(--text-muted)',
                    }}
                  />
                  <span
                    className="line-clamp-2 font-[family-name:var(--font-dm-sans)] text-xs leading-tight"
                    style={{ color: 'var(--text-primary)' }}
                  >
                    {result.display_name}
                  </span>
                </div>
              </li>
            ))}
          </ul>
        </div>
      )}

      {/* No results state */}
      {open && !loading && query.trim().length >= 2 && results.length === 0 && (
        <div
          className="absolute left-0 right-0 top-full z-50 mt-1 rounded-lg px-3 py-2"
          style={{
            background: 'var(--bg-elevated)',
            border: '1px solid var(--border-medium)',
          }}
        >
          <span
            className="font-[family-name:var(--font-dm-sans)] text-xs"
            style={{ color: 'var(--text-muted)' }}
          >
            No places found
          </span>
        </div>
      )}
    </div>
  );
}
