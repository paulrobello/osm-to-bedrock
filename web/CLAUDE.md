@AGENTS.md

## Commands

```bash
bun install          # First-time dependency install
bun run dev          # Dev server on port 8031
bun run build        # Production build (runs next build)
bun run lint         # ESLint
```

Or from the repo root:
```bash
make web-dev         # Dev server only
make web-build       # Production build
make dev             # Both Rust API (3002) + web (8031)
```

## Key Components & Hooks

| File | Purpose |
|------|---------|
| `src/app/page.tsx` | Root page — owns all top-level state, wires components together |
| `src/components/MapView.tsx` | OpenLayers map, bbox draw, spawn mode, cache layer |
| `src/components/DataSourcePanel.tsx` | Overpass fetch + PBF upload; Advanced section has Overpass URL input (localStorage key: `overpass_url`) |
| `src/components/ExportPanel.tsx` | Conversion options + fetch-convert trigger |
| `src/components/LayerPanel.tsx` | Layer visibility toggles + feature counts |
| `src/hooks/useMap.ts` | All map state — `loadGeoJSON`, `loadCacheAreas`, `flyTo`, bbox/spawn modes |
| `src/hooks/useConversion.ts` | `ConvertOptions` type, `startFetchConvert`, conversion polling |

## API Proxy Routes

All backend calls proxy through `src/app/api/` to the Rust server at `NEXT_PUBLIC_API_URL` (default `http://localhost:3002`):

| Route | Proxies to |
|-------|-----------|
| `POST /api/overpass` | Overpass API directly (accepts `overpass_url` body field) |
| `POST /api/fetch-convert` | `POST /fetch-convert` on Rust server |
| `GET  /api/cache` | `GET /cache/areas` on Rust server |
| `POST /api/upload` | `POST /parse` on Rust server |

## Gotchas

- `useMap` layer counts for the `cache` layer are updated by `MapView` after `loadCacheAreas` — call `onLayerCounts({ cache: n })` there, not from the hook itself.
- `overpassUrl` state lives in `page.tsx`, flows down to `DataSourcePanel` (for preview fetches) and `ExportPanel` → `useConversion` → `startFetchConvert` (for conversion).
- The `refreshCacheTrigger` counter in `page.tsx` is incremented on conversion done to reload the cache layer overlay.
