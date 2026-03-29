# CLI Reference

Complete flag reference for all `osm-to-bedrock` subcommands.

## Table of Contents

- [Global Options](#global-options)
- [convert](#convert)
- [fetch-convert](#fetch-convert)
- [overture-convert](#overture-convert)
- [terrain-convert](#terrain-convert)
- [serve](#serve)
- [cache](#cache)
- [Configuration File](#configuration-file)
- [Environment Variables](#environment-variables)

## Global Options

These flags apply to all subcommands:

| Flag | Description |
|------|-------------|
| `--config <path>` | Path to a YAML config file (overrides default search locations) |
| `--dump-config` | Print the resolved configuration as YAML and exit |
| `--version` | Print version information |
| `--help` | Print help |

## convert

Convert a local `.osm.pbf` file to a Bedrock world directory.

```bash
osm-to-bedrock convert --input city.osm.pbf --output MyWorld/
```

| Flag | Default | Description |
|------|---------|-------------|
| `-i, --input` | required | Input `.osm.pbf` file |
| `-o, --output` | required | Output world directory |
| `--scale` | `1.0` | Metres per block (higher = smaller map) |
| `--sea-level` | `65` | Y coordinate of the ground surface |
| `--building-height` | `8` | Height of generated buildings in blocks |
| `--wall-straighten-threshold` | `1` | Snap near-axis-aligned walls to straight (0 = off) |
| `--origin-lat` | centre | Override origin latitude |
| `--origin-lon` | centre | Override origin longitude |
| `--spawn-lat` | centre | Spawn point latitude |
| `--spawn-lon` | centre | Spawn point longitude |
| `--spawn-x` | -- | Spawn X block coordinate (overrides `--spawn-lat/lon`) |
| `--spawn-y` | -- | Spawn Y block coordinate |
| `--spawn-z` | -- | Spawn Z block coordinate (overrides `--spawn-lat/lon`) |
| `--signs` | off | Place street name signs along named roads |
| `--address-signs` | off | Place address signs on building facades |
| `--poi-markers` | off | Place POI markers at amenities, shops, and tourism nodes |
| `--elevation` | -- | Path to SRTM `.hgt` file or directory for real terrain |
| `--vertical-scale` | `1.0` | Blocks per metre of elevation change |
| `--elevation-smoothing` | `1` | Median-filter radius to smooth elevation jitter (0 = off) |
| `--surface-thickness` | `4` | Terrain fill depth below surface in blocks |
| `--watch` | off | Watch input file and re-convert on change |

**Examples:**

```bash
# 2:1 scale with real terrain and street signs
osm-to-bedrock convert \
  --input city.osm.pbf \
  --output MyCity/ \
  --scale 2.0 \
  --signs \
  --elevation N48W123.hgt \
  --vertical-scale 0.5

# Taller buildings, custom spawn point
osm-to-bedrock convert \
  --input city.osm.pbf \
  --output MyCity/ \
  --building-height 12 \
  --spawn-lat 51.5074 \
  --spawn-lon -0.1278

# Makefile shortcut
make convert INPUT=city.osm.pbf OUTPUT=~/games/worlds/MyCity
```

## fetch-convert

Fetch OSM data from the Overpass API for a bounding box and convert in one step. Results are cached on disk so repeat conversions of the same area are fast.

```bash
osm-to-bedrock fetch-convert \
  --bbox "south,west,north,east" \
  --output MyWorld/
```

Shares most flags with `convert` (scale, sea-level, building-height, spawn, signs, elevation, etc.) plus the following:

| Flag | Default | Description |
|------|---------|-------------|
| `--bbox` | required | Bounding box as `"south,west,north,east"` in decimal degrees |
| `-o, --output` | required | Output world directory |
| `--world-name` | `"OSM World"` | World name shown in Minecraft |
| `--overpass-url` | see env | Override the Overpass API endpoint |
| `--no-roads` | off | Exclude roads |
| `--no-buildings` | off | Exclude buildings |
| `--no-water` | off | Exclude water |
| `--no-landuse` | off | Exclude land use areas |
| `--no-railways` | off | Exclude railways |
| `--overture` | off | Also fetch and merge Overture Maps data |
| `--overture-themes` | all | Comma-separated Overture themes to fetch |
| `--overture-priority` | `""` | Per-theme priority, e.g. `"building=overture,transportation=osm"` |
| `--overture-timeout` | `120` | Timeout in seconds for the Overture CLI |

**Example:**

```bash
osm-to-bedrock fetch-convert \
  --bbox "48.85,2.33,48.87,2.36" \
  --output Paris/ \
  --signs \
  --poi-markers
```

## overture-convert

Build a world from Overture Maps data only (no OSM/Overpass required). Requires the `overturemaps` CLI to be installed.

```bash
osm-to-bedrock overture-convert \
  --bbox "48.85,2.33,48.87,2.36" \
  --output ParisOverture/
```

Shares spawn and elevation flags with `convert`, plus:

| Flag | Default | Description |
|------|---------|-------------|
| `--bbox` | required | Bounding box as `"south,west,north,east"` |
| `-o, --output` | required | Output world directory |
| `--themes` | all | Comma-separated Overture themes: `building,transportation,place,base,address` |
| `--world-name` | `"Overture World"` | World name shown in Minecraft |
| `--overture-timeout` | `120` | Timeout in seconds for the Overture CLI |
| `--scale` | `1.0` | Metres per block |
| `--sea-level` | `65` | Y coordinate of the ground surface |
| `--building-height` | `8` | Height of generated buildings in blocks |
| `--wall-straighten-threshold` | `1` | Snap near-axis-aligned walls to straight (0 = off) |
| `--signs` | off | Place street name signs |
| `--address-signs` | off | Place address signs on building facades |
| `--poi-markers` | off | Place POI markers |
| `--elevation` | -- | Path to SRTM `.hgt` file or directory |
| `--vertical-scale` | `1.0` | Blocks per metre of elevation change |
| `--elevation-smoothing` | `1` | Median-filter radius (0 = off) |
| `--surface-thickness` | `4` | Terrain fill depth below surface in blocks |

## terrain-convert

Generate a terrain-only world from SRTM elevation data with no OSM features. SRTM tiles are auto-downloaded if not supplied.

```bash
osm-to-bedrock terrain-convert \
  --bbox "47.0,-122.5,48.0,-121.5" \
  --output MtRainierTerrain/
```

| Flag | Default | Description |
|------|---------|-------------|
| `--bbox` | required | Bounding box as `"south,west,north,east"` |
| `-o, --output` | required | Output world directory |
| `--world-name` | `"Terrain World"` | World name shown in Minecraft |
| `--scale` | `1.0` | Metres per block |
| `--sea-level` | `65` | Y coordinate for sea level baseline |
| `--vertical-scale` | `1.0` | Blocks per metre of elevation change |
| `--snow-line` | `80` | Blocks above sea level where snow appears |
| `--elevation-smoothing` | `1` | Median-filter radius for elevation smoothing (0 = off) |
| `--surface-thickness` | `4` | Terrain fill depth below surface in blocks |
| `--elevation` | -- | Pre-downloaded `.hgt` file or directory (auto-downloads if omitted) |
| `--spawn-lat` | centre | Spawn point latitude |
| `--spawn-lon` | centre | Spawn point longitude |
| `--spawn-x` | -- | Spawn X block coordinate |
| `--spawn-y` | -- | Spawn Y block coordinate |
| `--spawn-z` | -- | Spawn Z block coordinate |

## serve

Start the HTTP API server that powers the Web Explorer.

```bash
osm-to-bedrock serve --port 3002 --host 127.0.0.1
```

| Flag | Default | Description |
|------|---------|-------------|
| `--port` | `3002` | Port to listen on |
| `--host` | `127.0.0.1` | Host address to bind to |
| `--clear-cache` | -- | Clear Overpass cache before starting (optional age: `7d`, `24h`, `30m`) |

## cache

Manage the Overpass and Overture disk caches.

```bash
osm-to-bedrock cache list                    # List cached entries
osm-to-bedrock cache stats                   # Show cache statistics
osm-to-bedrock cache clear                   # Clear all entries
osm-to-bedrock cache clear --older-than 7d   # Clear entries older than 7 days
```

| Subcommand | Description |
|------------|-------------|
| `list` | List all cached entries (Overpass + Overture) with bbox, size, and age |
| `stats` | Show entry counts, total size, and cache directory paths |
| `clear` | Clear cached entries |

**`clear` flags:**

| Flag | Description |
|------|-------------|
| `--older-than <AGE>` | Only remove entries older than the given age (e.g. `7d`, `24h`, `30m`) |
| `--overpass-only` | Clear only Overpass cache entries |
| `--overture-only` | Clear only Overture cache entries |

## Configuration File

All CLI flags can also be set in a YAML configuration file. The tool searches in order:

1. Path given via `--config <path>`
2. `.osm-to-bedrock.yaml` in the current directory
3. `~/.config/osm-to-bedrock/config.yaml`

CLI flags override config file values. Use `--dump-config` to print the resolved configuration.

Example `.osm-to-bedrock.yaml`:

```yaml
scale: 1.0
sea_level: 65
building_height: 8
signs: true
elevation_smoothing: 1
vertical_scale: 0.5
surface_thickness: 4
poi_decorations: true
nature_decorations: true
```

> **Note:** `poi_decorations` and `nature_decorations` are config-file-only settings (no CLI flag). Both default to `true`.

## Environment Variables

| Variable | Description |
|----------|-------------|
| `OVERPASS_URL` | Override the default Overpass API endpoint (`https://overpass-api.de/api/interpreter`) |
| `OVERPASS_CACHE_DIR` | Override the disk cache directory (default: `~/.cache/osm-to-bedrock/overpass/`) |
| `RUST_LOG` | Control log verbosity (e.g. `RUST_LOG=debug`) |

## Related Documentation

- [README](../README.md) -- Project overview and quick start
- [Developer Info](DEVELOPER_INFO.md) -- Architecture, module reference, and development guide
- [Docs Index](README.md) -- Full documentation directory
