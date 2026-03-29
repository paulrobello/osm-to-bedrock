# osm-to-bedrock Web Explorer

Browser-based UI for the osm-to-bedrock converter. Draw a bounding box on a live
OpenStreetMap map, fetch OSM data, preview it by layer, and export a `.mcworld` file —
all without touching the command line.

## Table of Contents

- [Prerequisites](#prerequisites)
- [Setup](#setup)
- [Configuration](#configuration)
- [Features](#features)
- [Proxy Route Table](#proxy-route-table)
- [Development Commands](#development-commands)

## Prerequisites

| Tool | Notes |
|------|-------|
| bun 1.1+ | Package manager and runtime — [bun.sh](https://bun.sh) |
| osm-to-bedrock Rust API | Must be running on port 3002 (see root `README.md`) |

The Rust API server must be running before the web frontend can fetch OSM data or
trigger conversions. Start both together from the repo root:

```bash
make dev
```

## Setup

```bash
# Install dependencies (first time only)
bun install

# Start the dev server
bun run dev
# Open http://localhost:8031
```

Or from the repo root:

```bash
make dev       # Start both Rust API (3002) and web (8031)
make web-dev   # Start web dev server only
```

## Configuration

Create `web/.env.local` (copy from `.env.local.example`) to configure the Rust API URL:

```bash
cp .env.local.example .env.local
```

| Variable | Default | Description |
|----------|---------|-------------|
| `NEXT_PUBLIC_API_URL` | `http://localhost:3002` | URL of the Rust API server. Change this when running the API on a different host or port. |

## Features

- Interactive OpenLayers map — pan, zoom, and draw a bounding box for the area to convert
- Location search — type a city or address to fly to it
- Load OSM data via the Overpass API or upload a local `.osm.pbf` file
- Custom Overpass API URL — override to use a mirror when the default is overloaded
- Layer toggles for roads, buildings, water, land use, railways, and the Overpass cache overlay
- Feature inspector — click any map feature to see its raw OSM tags
- Spawn point placement — click the map to set the player spawn position
- Export panel with all conversion options (scale, sea level, building height, signs, POI markers)
- Real-time conversion progress with download link on completion

## Proxy Route Table

All backend calls go through Next.js API routes at `src/app/api/` and are forwarded to
the Rust server at `NEXT_PUBLIC_API_URL`:

| Route | Method | Proxies to | Notes |
|-------|--------|-----------|-------|
| `/api/upload` | POST | `POST /parse` | Multipart PBF upload; returns GeoJSON + bounds + stats |
| `/api/convert` | POST | `POST /convert` | Multipart PBF + options JSON; returns job ID |
| `/api/fetch-convert` | POST | `POST /fetch-convert` | Overpass bbox + options; returns job ID |
| `/api/status/[id]` | GET | `GET /status/{id}` | Poll conversion progress |
| `/api/download` | GET | `GET /download/{id}` | Stream `.mcworld` ZIP to browser |
| `/api/cache` | GET | `GET /cache/areas` | List cached Overpass bbox entries |
| `/api/overpass` | POST | Overpass API directly | Accepts `overpass_url` override in body |
| `/api/geocode` | GET | Nominatim geocoding | Location search |

## Development Commands

```bash
bun run dev      # Dev server on http://localhost:8031
bun run build    # Production build (runs next build)
bun run start    # Production server on http://localhost:8031
bun run lint     # ESLint
```
