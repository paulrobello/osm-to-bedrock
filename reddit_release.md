# osm-to-bedrock -- Convert OpenStreetMap Data into Playable Minecraft Bedrock Worlds

---

I wanted to explore my neighbourhood in Minecraft. Java Edition has a few map converters, but Bedrock Edition uses a completely different world format (LevelDB + little-endian NBT instead of Anvil + big-endian), and I couldn't find anything that handled the full pipeline well. So I built one.

**osm-to-bedrock** is a Rust CLI that takes OpenStreetMap `.osm.pbf` files and produces valid Minecraft Bedrock worlds. One block = one metre by default, though the scale is configurable. The output is a `.mcworld` file you can open directly on any Bedrock platform (Windows, Android, iOS).

## What gets converted

- **Roads** -- variable width based on highway type, with sidewalks and centre lines. Bridges and tunnels handled separately with appropriate block choices.
- **Buildings** -- floor, ceiling, perimeter walls. A wall-straightening pass snaps near-axis-aligned walls to clean right angles. Configurable height.
- **Water** -- rivers, streams, canals, ditches, lakes. Each waterway type has its own depth and width profile.
- **Land use** -- parks, forests (with procedurally placed trees), farmland, residential areas, etc.
- **Railways** -- rail blocks on gravel beds.
- **Signs & decorations** -- street name signs along named roads, address signs on building facades, POI markers at amenities/shops/tourism, decorative blocks (benches, mailboxes, campfires) at relevant POIs, individual trees from OSM tree node data.
- **Elevation** -- real-world terrain from SRTM data. Configurable vertical scale and median-filter smoothing so roads aren't jagged on hillsides.

## Data sources

You can feed it a local `.osm.pbf` file, or use `fetch-convert` to grab data straight from the Overpass API for any bounding box. Results are disk-cached (SHA-256 keyed) so repeat builds are instant. There's also Overture Maps integration if you want to supplement or replace OSM data on a per-theme basis.

## Web Explorer

Didn't want to deal with the CLI? There's a browser-based Web Explorer (Next.js + OpenLayers). Draw a bounding box on the map, load OSM data, tweak parameters (scale, building height, elevation, smoothing), hit export, and download the `.mcworld` file. It shows live Minecraft `/tp` coordinates as you move the cursor, has layer toggles, a feature inspector for clicking on things to see their OSM tags, and spawn point placement via right-click.

![Web Explorer](https://raw.githubusercontent.com/paulrobello/osm-to-bedrock/main/screenshots/screenshot-map.png)

[More screenshots](https://paulrobello.github.io/osm-to-bedrock/gallery.html)

## Technical details for the curious

The pipeline processes the world in 64x64 chunk tiles with a background LevelDB writer thread, so memory stays bounded regardless of area size. SubChunk encoding uses the smallest valid bits-per-block from `[1,2,3,4,5,6,8,16]` to keep file sizes down. The LevelDB layer registers Mojang-compatible zlib and raw-deflate compressors.

Coordinate mapping is equirectangular (East = +X, North = -Z). It's good for areas up to ~50km before projection distortion gets noticeable. Fine for cities, not ideal for countries.

## Getting started

```bash
# Build from source
git clone https://github.com/paulrobello/osm-to-bedrock
cd osm-to-bedrock && make build

# Convert a local file
osm-to-bedrock convert --input monaco.osm.pbf --output Monaco/

# Fetch from Overpass and convert in one step
osm-to-bedrock fetch-convert --bbox "48.85,2.33,48.87,2.36" --output Paris/ --signs

# Or use the Web Explorer
make dev   # API on port 3002, web UI on port 8031
```

Grab a small extract from [Geofabrik](https://download.geofabrik.de/) (Monaco is ~500KB) to try it out.

## Requirements

- Rust stable
- bun 1.1+ (only for the Web Explorer frontend)

---

**GitHub:** https://github.com/paulrobello/osm-to-bedrock
**License:** MIT

Feedback welcome -- issues and PRs on GitHub. If you end up converting your city, I'd love to see screenshots.
