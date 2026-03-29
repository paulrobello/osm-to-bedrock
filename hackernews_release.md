# Show HN: osm-to-bedrock – Turn OpenStreetMap data into playable Minecraft Bedrock worlds

**URL:** https://github.com/paulrobello/osm-to-bedrock

---

I built a Rust CLI that converts OpenStreetMap `.osm.pbf` files into Minecraft Bedrock Edition worlds. Roads, buildings, waterways, forests, railways -- all mapped to appropriate blocks at 1:1 scale (one metre = one block, configurable).

## How it works

Single-pass pipeline: parse PBF, convert lat/lon to block coordinates via equirectangular projection, map OSM tags to a 56-variant Block enum, fill terrain layers, overlay features, write LevelDB with SubChunk v8 encoding. Output is a valid Bedrock world you can open directly as a `.mcworld` file.

The world builds in tiles (64x64 chunks) with a background writer thread, so memory stays bounded even for large areas. A city district converts in seconds.

## What you get

- Roads with variable width, sidewalks, and centre lines
- Buildings with walls, floors, ceilings, and wall-straightening for cleaner geometry
- Waterways with per-type depth and width (rivers, streams, canals, ditches)
- Forests with procedurally placed trees
- Street name signs, address signs, POI markers, decorative blocks (benches, mailboxes, etc.)
- Real-world terrain elevation via SRTM data with configurable vertical scale and smoothing
- Overture Maps integration -- supplement or replace OSM data per theme
- Disk-cached Overpass queries so repeat builds of the same area are instant

## Web Explorer

There's a browser UI (Next.js + OpenLayers) where you draw a bounding box on a map, fetch OSM data, tweak conversion parameters, and download a `.mcworld` directly. No CLI needed. Live Minecraft `/tp` coordinates in the footer, feature inspector, layer toggles, the usual map stuff.

## Quick start

```
cargo install --path .
osm-to-bedrock convert --input monaco.osm.pbf --output Monaco/
osm-to-bedrock fetch-convert --bbox "48.85,2.33,48.87,2.36" --output Paris/
make dev  # starts API server + web UI
```

Rust stable, MIT licensed. The web UI needs bun for the Next.js frontend.

I started this because I wanted to walk around my neighbourhood in Minecraft and couldn't find a Bedrock-compatible converter that handled the full pipeline well. Java Edition has a few options but Bedrock's LevelDB format is different enough that you can't just adapt them.

https://github.com/paulrobello/osm-to-bedrock
