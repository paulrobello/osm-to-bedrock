# osm-to-bedrock

[![CI](https://github.com/paulrobello/osm-to-bedrock/actions/workflows/ci.yml/badge.svg)](https://github.com/paulrobello/osm-to-bedrock/actions/workflows/ci.yml)
![Runs on Linux | MacOS | Windows](https://img.shields.io/badge/runs%20on-Linux%20%7C%20MacOS%20%7C%20Windows-blue)
![Arch x86-64 | ARM | AppleSilicon](https://img.shields.io/badge/arch-x86--64%20%7C%20ARM%20%7C%20AppleSilicon-blue)
![License](https://img.shields.io/badge/license-MIT-green)

Convert [OpenStreetMap](https://www.openstreetmap.org/) data into playable **Minecraft Bedrock Edition** worlds. Roads, buildings, waterways, forests, and land-use areas are all mapped to appropriate Minecraft blocks at 1:1 scale (one block = one metre, configurable). Includes a browser-based Web Explorer for selecting areas on a live map and exporting directly to `.mcworld` files.

![Web Explorer Map View](https://raw.githubusercontent.com/paulrobello/osm-to-bedrock/main/screenshots/screenshot-map.png)

[View all screenshots](https://paulrobello.github.io/osm-to-bedrock/gallery.html)

[!["Buy Me A Coffee"](https://www.buymeacoffee.com/assets/img/custom_images/orange_img.png)](https://buymeacoffee.com/probello3)

## Table of Contents

- [Getting Started](#getting-started)
- [Features](#features)
- [Installation](#installation)
- [Quick Start](#quick-start)
- [Configuration](#configuration)
- [Subcommands](#subcommands)
- [Web Explorer](#web-explorer)
- [Documentation](#documentation)
- [Getting OSM Data](#getting-osm-data)
- [Adding the World to Minecraft Bedrock](#adding-the-world-to-minecraft-bedrock)
- [Architecture](#architecture)
- [Known Limitations](#known-limitations)
- [Contributing](#contributing)
- [License](#license)
- [Author](#author)
- [Links](#links)

## Getting Started

New to osm-to-bedrock? Here are the quickest paths to a working Minecraft world:

- **[Installation](#installation)** — Build from source or install via cargo
- **[Quick Start](#quick-start)** — Convert your first map in one command
- **[Web Explorer](#web-explorer)** — Draw a bounding box on a live map and export directly
- **[Configuration](#configuration)** — Persistent YAML config for all CLI flags
- **[Developer Info](docs/DEVELOPER_INFO.md)** — Architecture details and module reference

## Features

### Conversion Pipeline
- Parses `.osm.pbf` files (the standard compressed OSM format)
- Converts geographic coordinates (lat/lon) to Minecraft block coordinates at 1:1 scale
- Generates a Bedrock-compatible LevelDB world with roads, buildings, waterways, forests, and land use
- Multipolygon relation support (buildings with holes, complex land-use boundaries)

### Terrain & Elevation
- Real-world terrain elevation via SRTM HGT files with configurable vertical scale and smoothing
- Terrain-only world generation (`terrain-convert`) from SRTM data with no OSM features

### Signs & Decorations
- Street name signs, address signs, and POI markers (amenities, shops)
- POI decorations (benches, mailboxes, etc.) and individual tree placement

### Data Sources
- Overture Maps data integration — supplement or replace OSM data per theme
- Disk-based Overpass and Overture response cache with CLI management

### Web Explorer
- Browser-based Web Explorer — draw a bounding box, fetch live OSM data, export `.mcworld` in one step
- Web UI shows live Minecraft `/tp` teleport coordinates with click-to-copy
- HTTP API server for integration with external tools or custom frontends

### Configuration & Deployment
- YAML configuration file for persistent settings (`.osm-to-bedrock.yaml`)
- Docker image for self-hosted deployment
- File watch mode (`--watch`) for auto re-conversion on input file change

## Installation

### Pre-built Binaries

Download the latest binary for your platform from the [Releases page](https://github.com/paulrobello/osm-to-bedrock/releases):

| Platform | Binary |
|----------|--------|
| Linux x86_64 | `osm-to-bedrock-linux-x86_64` |
| Linux ARM64 | `osm-to-bedrock-linux-aarch64` |
| macOS x86_64 | `osm-to-bedrock-macos-x86_64` |
| macOS ARM64 (Apple Silicon) | `osm-to-bedrock-macos-aarch64` |
| Windows x86_64 | `osm-to-bedrock-windows-x86_64.exe` |

```bash
# Example: Linux x86_64
curl -LO https://github.com/paulrobello/osm-to-bedrock/releases/latest/download/osm-to-bedrock-linux-x86_64
chmod +x osm-to-bedrock-linux-x86_64
mv osm-to-bedrock-linux-x86_64 ~/.local/bin/osm-to-bedrock
```

### Cargo Install

```bash
# Install from crates.io
cargo install osm_to_bedrock

# Or install from local source
cargo install --path .
```

### From Source

Requires Rust stable (install via [rustup](https://rustup.rs/)):

```bash
git clone https://github.com/paulrobello/osm-to-bedrock
cd osm-to-bedrock
make build
# binary at target/release/osm-to-bedrock
```

### Prerequisites

| Tool | Version | Notes |
|------|---------|-------|
| Rust | stable | Only needed for building from source: [rustup.rs](https://rustup.rs/) |
| bun | 1.1+ | Only needed for the Web Explorer: [bun.sh](https://bun.sh) |

## Quick Start

```bash
# Convert a local PBF file
osm-to-bedrock convert --input monaco-latest.osm.pbf --output MonacoWorld/

# Fetch a bounding box from Overpass and convert in one step
osm-to-bedrock fetch-convert \
  --bbox "51.50,-0.13,51.52,-0.10" \
  --output LondonCity/

# Start the Web Explorer (browser UI on http://localhost:8031)
make dev
```

A city extract such as `monaco-latest.osm.pbf` (~500 KB from Geofabrik) is a good
starting point.

## Configuration

All CLI flags can also be set in a YAML configuration file. The tool searches in order:

1. Path given via `--config <path>`
2. `.osm-to-bedrock.yaml` in the current directory
3. `~/.config/osm-to-bedrock/config.yaml`

Use `--dump-config` with any subcommand to print the resolved configuration.

Example `.osm-to-bedrock.yaml`:

```yaml
scale: 1.0
sea_level: 65
building_height: 8
signs: true
elevation_smoothing: 1
vertical_scale: 0.5
```

## Subcommands

Run `osm-to-bedrock <subcommand> --help` for inline help, or see the **[CLI Reference](docs/CLI.md)** for detailed flag tables.

### convert

Convert a local `.osm.pbf` file to a Bedrock world directory.

```bash
osm-to-bedrock convert --input city.osm.pbf --output MyWorld/

# With terrain, signs, and 2:1 scale
osm-to-bedrock convert \
  --input city.osm.pbf --output MyCity/ \
  --scale 2.0 --signs --elevation N48W123.hgt --vertical-scale 0.5

# Makefile shortcut
make convert INPUT=city.osm.pbf OUTPUT=~/games/worlds/MyCity
```

Key options: `--scale`, `--sea-level`, `--building-height`, `--signs`, `--address-signs`, `--poi-markers`, `--elevation`, `--vertical-scale`, `--elevation-smoothing`, `--spawn-lat/lon`, `--watch`

### fetch-convert

Fetch OSM data from Overpass for a bounding box and convert in one step. Results are cached on disk.

```bash
osm-to-bedrock fetch-convert \
  --bbox "48.85,2.33,48.87,2.36" --output Paris/ --signs --poi-markers
```

Supports all `convert` flags plus: `--bbox`, `--world-name`, `--overpass-url`, `--no-roads`, `--no-buildings`, `--no-water`, `--no-landuse`, `--no-railways`, `--overture`, `--overture-themes`, `--overture-priority`

### overture-convert

Build a world from Overture Maps data only (no OSM/Overpass). Requires the `overturemaps` CLI.

```bash
osm-to-bedrock overture-convert \
  --bbox "48.85,2.33,48.87,2.36" --output ParisOverture/
```

Key options: `--bbox`, `--themes`, `--world-name`, `--overture-timeout`, plus shared flags (`--scale`, `--elevation`, etc.)

### terrain-convert

Generate a terrain-only world from SRTM elevation data. SRTM tiles auto-download if not supplied.

```bash
osm-to-bedrock terrain-convert \
  --bbox "47.0,-122.5,48.0,-121.5" --output MtRainierTerrain/
```

Key options: `--bbox`, `--scale`, `--sea-level`, `--vertical-scale`, `--snow-line`, `--elevation`, `--surface-thickness`

### serve

Start the HTTP API server that powers the Web Explorer.

```bash
osm-to-bedrock serve --port 3002 --host 127.0.0.1
```

Options: `--port` (default 3002), `--host` (default 127.0.0.1), `--clear-cache` (optional age: `7d`, `24h`, `30m`)

### cache

Manage the Overpass and Overture disk caches.

```bash
osm-to-bedrock cache list                    # List cached entries
osm-to-bedrock cache stats                   # Show cache statistics
osm-to-bedrock cache clear --older-than 7d   # Clear stale entries
```

## Web Explorer

The Web Explorer is a browser UI for selecting a map area, loading OSM data, and
exporting `.mcworld` files without using the CLI.

```bash
# First time: install web dependencies
cd web && bun install

# Start both the Rust API server (port 3002) and the web UI (port 8031)
make dev

# Open the Web Explorer
open http://localhost:8031
```

**Features:**

- Interactive map with OpenLayers — pan, zoom, draw a bounding box
- Location search (geocoding)
- Load OSM data via Overpass API or upload a local `.osm.pbf`
- Layer toggles for roads, buildings, water, land use, railways, cache areas
- Feature inspector — click any feature to see its OSM tags
- Spawn point placement (click map or right-click context menu)
- Live Minecraft `/tp` teleport coordinates in footer — click to copy, right-click for context menu
- Configurable conversion parameters (scale, elevation, smoothing, building height, etc.)
- Overture Maps data integration toggle
- Conversion with real-time progress tracking
- Direct `.mcworld` download

**Configuration:**

Set `NEXT_PUBLIC_API_URL` in `web/.env.local` to point the frontend at a non-default
Rust API server:

```bash
NEXT_PUBLIC_API_URL=http://localhost:3002
```

See `web/.env.local.example` for a complete template.

## Documentation

- **[Architecture](docs/ARCHITECTURE.md)** — Pipeline, module map, coordinate system, streaming tiles, server
- **[CLI Reference](docs/CLI.md)** — Complete flag reference for all subcommands
- **[Web Explorer](docs/WEB_UI.md)** — Components, hooks, API proxy, map features, keyboard shortcuts
- **[Developer Info](docs/DEVELOPER_INFO.md)** — Module reference, block mappings, and development guide
- **[Docs Index](docs/README.md)** — Full documentation directory
- **[Changelog](CHANGELOG.md)** — Version history
- **[Contributing](CONTRIBUTING.md)** — How to contribute

## Getting OSM Data

Download `.osm.pbf` extracts from:

- [Geofabrik](https://download.geofabrik.de/) — continent, country, and region extracts
- [BBBike](https://extract.bbbike.org/) — custom bounding box extracts
- [Overpass Turbo](https://overpass-turbo.eu/) — query then Export → OpenStreetMap XML,
  then convert to PBF with `osmconvert`

Or use `fetch-convert` / the Web Explorer to fetch directly from Overpass.

## Adding the World to Minecraft Bedrock

Open the `.mcworld` file directly and Minecraft Bedrock will import it automatically. This works on all platforms (Windows, Android, iOS).

Alternatively, copy the output directory into your Bedrock worlds folder manually:
- **Windows**: `%LocalAppData%\Packages\Microsoft.MinecraftUWP_*\LocalState\games\com.mojang\minecraftWorlds\`
- **Android**: `/sdcard/games/com.mojang/minecraftWorlds/`
- **iOS**: Use a file manager app to navigate to the Minecraft documents folder

## Architecture

Parse → Convert coordinates → Map OSM tags to blocks → Build terrain + overlay features → Write LevelDB world.

The CLI also includes an Axum HTTP server powering the Web Explorer, with Overpass and Overture data fetching, disk caching, and SRTM elevation support.

See **[Developer Info](docs/DEVELOPER_INFO.md)** for the full module reference, source map, and architecture diagram.

## Known Limitations

- **Projection distortion**: equirectangular projection; distortion grows with distance from
  the origin and with higher latitudes (significant beyond ~50 km)
- **XML/OSM format**: only `.osm.pbf` is supported for local files; convert `.osm` to PBF with `osmconvert`, or use `fetch-convert` to fetch directly from Overpass

## Contributing

Contributions are welcome! See [CONTRIBUTING.md](CONTRIBUTING.md) for the full development setup, commit message format, and PR process.

Before submitting a pull request:

```bash
make build      # Release build
make test       # Run unit tests
make lint       # cargo clippy
make fmt        # rustfmt
make typecheck  # cargo check
make web-check  # Lint and build-check the Next.js frontend
make checkall   # fmt + lint + typecheck + test + web-check
make clean      # cargo clean
make install    # cargo install --path .
make dev        # Start both Rust API + Web Explorer
make serve      # Start Rust API server only
make stop       # Gracefully stop both dev servers
make kill       # Force-kill both dev servers
```

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## Author

Paul Robello - probello@gmail.com

## Links

- **GitHub**: [https://github.com/paulrobello/osm-to-bedrock](https://github.com/paulrobello/osm-to-bedrock)
