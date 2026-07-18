//! Java Edition world writer using the Anvil region format.
//!
//! Generates `.mca` region files containing big-endian NBT chunk data,
//! plus a gzip-compressed `level.dat` and `session.lock`.
//!
//! ## Anvil region file format (`r.X.Z.mca`)
//! ```text
//! Header (8 KB = 2 sectors of 4096 bytes):
//!   Bytes 0–4095:    Location table  — 1024 × 4-byte entries
//!   Bytes 4096–8191: Timestamp table — 1024 × 4-byte BE u32 entries
//!
//! Per-chunk data (after header, sector-aligned):
//!   [4-byte BE length][1-byte compression type 0x02 = zlib][zlib-compressed BE NBT]
//! ```

use crate::{
    blocks::{Block, surface_to_java_biome},
    nbt_be::{self, TAG_COMPOUND, TAG_STRING},
    spatial::TILE_CHUNKS,
    world::{ChunkData, ChunkStore, MAX_Y, MIN_Y, WorldWriter},
};
use anyhow::{Context, Result};
use flate2::{Compression, write::GzEncoder, write::ZlibEncoder};
use std::collections::HashMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};

// ── JavaWorld ─────────────────────────────────────────────────────────────

/// Writes a Java Edition world using Anvil region files.
///
/// Two modes, selected at construction time:
/// - **In-memory** ([`Self::new`] / [`Self::new_bounded`]): accumulates every
///   chunk in RAM and writes all region files in [`Self::save`]. Simple, but
///   peak memory grows with world size — only suitable for small worlds or
///   previews.
/// - **Streaming** ([`Self::new_streaming`]): driven by the tile pipeline via
///   [`WorldWriter::set_tile_bounds`] + [`WorldWriter::flush_tile`]. Holds only
///   the current tile's chunks in the scratch [`ChunkStore`] and lazily writes
///   each 32×32 region file once the tile containing its maximum in-bounds
///   chunk has flushed (see `StreamingAnvil`). Peak memory ≈ one tile + a
///   handful of frontier region buffers, matching Bedrock's per-tile profile.
pub struct JavaWorld {
    /// Shared chunk grid + auxiliary override maps + optional tile bounds.
    /// Common to both backends; see [`ChunkStore`].
    store: ChunkStore,
    output: PathBuf,
    /// Streaming-mode state. `None` for the in-memory constructors, in which
    /// case `set_tile_bounds`/`flush_tile` are no-ops and `save` writes every
    /// accumulated chunk at once.
    streaming: Option<StreamingAnvil>,
}

impl JavaWorld {
    /// Create an unbounded `JavaWorld` that accepts block writes in any chunk.
    ///
    /// Accumulates chunks in memory; call [`JavaWorld::save`] (via the
    /// [`WorldWriter`] trait) at the end of the pipeline to write the Anvil
    /// region files, `level.dat`, and `session.lock`.
    pub fn new(output: &Path) -> Self {
        Self {
            store: ChunkStore::new(),
            output: output.to_path_buf(),
            streaming: None,
        }
    }

    /// Create a `JavaWorld` bounded to the chunk rectangle
    /// `[min_cx, max_cx] × [min_cz, max_cz]`. Writes outside the rectangle are
    /// silently dropped. Java's `set_tile_bounds` is a no-op at the trait
    /// level for the in-memory backends, so this constructor is the only way
    /// to scope in-memory Java writes.
    pub fn new_bounded(output: &Path, min_cx: i32, max_cx: i32, min_cz: i32, max_cz: i32) -> Self {
        Self {
            store: ChunkStore::new_bounded(min_cx, max_cx, min_cz, max_cz),
            output: output.to_path_buf(),
            streaming: None,
        }
    }

    /// Create a streaming `JavaWorld` bounded to the world's chunk rectangle.
    ///
    /// The tile pipeline calls `set_tile_bounds` once per tile, then
    /// `flush_tile` to drain the tile's chunks into region buffers and write
    /// any region whose last contributing tile has flushed. Peak memory stays
    /// bounded to one tile's worth of `ChunkData` plus a small set of in-flight
    /// region buffers. Call `save` once at the end to flush remaining regions
    /// and emit `level.dat` + `session.lock`.
    pub fn new_streaming(
        output: &Path,
        min_cx: i32,
        max_cx: i32,
        min_cz: i32,
        max_cz: i32,
    ) -> Result<Self> {
        Ok(Self {
            store: ChunkStore::new(),
            output: output.to_path_buf(),
            streaming: Some(StreamingAnvil::new(output, min_cx, max_cx, min_cz, max_cz)?),
        })
    }

    /// Drain the current tile's chunks into the streaming region buffers,
    /// then write any region whose last contributing tile has flushed.
    ///
    /// No-op outside streaming mode. Separated from `flush_tile` so the
    /// `&mut StreamingAnvil` borrow does not overlap the `&mut self.store`
    /// borrows (same take()-and-restore shape Bedrock's `flush_tile` uses).
    fn drain_tile_to_stream(&mut self, stream: &mut StreamingAnvil) -> Result<()> {
        let chunks = self.store.take_chunks();
        {
            let entities = self.store.block_entities();
            for ((cx, cz), chunk) in &chunks {
                let compressed = compress_chunk(*cx, *cz, chunk, entities.get(&(*cx, *cz)))?;
                stream.store_chunk(*cx, *cz, compressed);
            }
        }
        self.store.clear_aux();
        stream.seal_completed()?;
        Ok(())
    }

    /// Write the world to disk with spawn at the given block coordinates.
    ///
    /// Streaming mode flushes any remaining region buffers (most were written
    /// incrementally by `flush_tile`) then emits `level.dat` + `session.lock`.
    /// In-memory mode groups all accumulated chunks into regions and writes
    /// them all here.
    pub fn save(&mut self, spawn_x: i32, spawn_y: i32, spawn_z: i32) -> Result<()> {
        std::fs::create_dir_all(&self.output)
            .with_context(|| format!("creating output dir {}", self.output.display()))?;

        if let Some(stream) = self.streaming.as_mut() {
            stream.seal_all()?;
            write_level_dat(&self.output, spawn_x, spawn_y, spawn_z)?;
            write_session_lock(&self.output)?;
            return Ok(());
        }

        let region_dir = self.output.join("region");
        std::fs::create_dir_all(&region_dir)?;

        let chunks = self.store.chunks();

        // Group chunks into 32x32 regions
        let mut regions: HashMap<(i32, i32), Vec<(i32, i32)>> = HashMap::new();
        for &(cx, cz) in chunks.keys() {
            let rx = cx.div_euclid(32);
            let rz = cz.div_euclid(32);
            regions.entry((rx, rz)).or_default().push((cx, cz));
        }

        for (&(rx, rz), region_chunks) in &regions {
            let path = region_dir.join(format!("r.{rx}.{rz}.mca"));
            let data = encode_region(region_chunks, chunks, self.store.block_entities())?;
            std::fs::write(&path, &data).with_context(|| format!("writing {}", path.display()))?;
        }

        write_level_dat(&self.output, spawn_x, spawn_y, spawn_z)?;
        write_session_lock(&self.output)?;
        Ok(())
    }
}

impl WorldWriter for JavaWorld {
    fn set_block(&mut self, x: i32, y: i32, z: i32, block: Block) {
        self.store.set_block(x, y, z, block)
    }

    fn get_block(&self, x: i32, y: i32, z: i32) -> Block {
        self.store.get_block(x, y, z)
    }

    fn insert_chunk(&mut self, cx: i32, cz: i32, chunk: ChunkData) {
        self.store.insert_chunk(cx, cz, chunk)
    }

    fn add_block_entity(&mut self, x: i32, y: i32, z: i32, nbt: Vec<u8>) {
        self.store.add_block_entity(x, y, z, nbt)
    }

    fn set_sign_direction(&mut self, x: i32, y: i32, z: i32, direction: i32) {
        self.store.set_sign_direction(x, y, z, direction)
    }

    fn set_block_direction(&mut self, x: i32, y: i32, z: i32, direction: i32) {
        self.store.set_block_direction(x, y, z, direction)
    }

    fn chunk_count(&self) -> usize {
        self.store.chunk_count()
    }

    fn occupied_chunks(&self) -> Vec<(i32, i32)> {
        self.store.occupied_chunks()
    }

    fn surface_blocks(&self) -> Vec<(i32, i32, i32, String)> {
        self.store.surface_blocks()
    }

    /// Streaming mode records the active tile (used by the region-seal test)
    /// and scopes the scratch store to it so only the current tile's chunks
    /// accumulate. No-op for the in-memory backends (preserves `new`/
    /// `new_bounded` unbounded/bounded behaviour).
    fn set_tile_bounds(&mut self, min_cx: i32, max_cx: i32, min_cz: i32, max_cz: i32) {
        if let Some(stream) = self.streaming.as_mut() {
            stream.set_tile(min_cx, max_cx, min_cz, max_cz);
            self.store.set_tile_bounds(min_cx, max_cx, min_cz, max_cz);
        }
    }

    /// Streaming mode drains the current tile's chunks into region buffers and
    /// writes any region whose last contributing tile has now flushed. No-op
    /// for the in-memory backends.
    fn flush_tile(&mut self) -> Result<()> {
        if let Some(mut stream) = self.streaming.take() {
            let result = self.drain_tile_to_stream(&mut stream);
            self.streaming = Some(stream);
            result?;
        }
        Ok(())
    }

    fn save(&mut self, spawn_x: i32, spawn_y: i32, spawn_z: i32) -> Result<()> {
        JavaWorld::save(self, spawn_x, spawn_y, spawn_z)
    }
}

// ── Palette key for Java block states ─────────────────────────────────────

/// Key used to deduplicate blocks in a Java palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct JavaPaletteKey {
    block: Block,
    /// Direction value for directional blocks (signs, stairs, rails).
    direction: i32,
}

// ── Region encoding ───────────────────────────────────────────────────────

/// Encode a single chunk column to its zlib-compressed on-disk blob (the
/// payload that [`assemble_region_file`] packs into a sector). This is the
/// per-chunk step shared by the in-memory `encode_region` path and the
/// streaming [`StreamingAnvil`] path, so both produce byte-identical region
/// files.
fn compress_chunk(
    cx: i32,
    cz: i32,
    chunk: &ChunkData,
    entities: Option<&Vec<Vec<u8>>>,
) -> Result<Vec<u8>> {
    let nbt = encode_chunk_nbt(cx, cz, chunk, entities)?;
    zlib_compress(&nbt)
}

/// Pack 1024 optional compressed-chunk blobs into an Anvil region file.
///
/// `slots[i]` corresponds to region-local chunk `(lx, lz)` where
/// `i = lz * 32 + lx`. Empty slots produce a zero location entry (Minecraft
/// treats them as ungenerated). `timestamp` stamps every present chunk's
/// timestamp-table entry; pass `0` for deterministic test output.
fn assemble_region_file(slots: &[Option<Vec<u8>>; 1024], timestamp: u32) -> Vec<u8> {
    // Build the region file: header + chunk sectors
    let mut file: Vec<u8> = Vec::new();

    // Location table: 1024 x 4-byte entries (offset + sector count)
    // Placeholder — filled once we know actual positions
    let header_size: usize = 8192;
    file.resize(header_size, 0u8);

    let mut timestamp_table = [0u32; 1024];
    let sector_size: usize = 4096;

    for (i, opt_data) in slots.iter().enumerate() {
        let Some(data) = opt_data else { continue };

        // Chunk on-disk: [4-byte BE length][0x02 compression][zlib data]
        let chunk_on_disk_size = 4 + 1 + data.len();
        let sectors_needed = chunk_on_disk_size.div_ceil(sector_size);
        let sector_offset = file.len() / sector_size;

        // Write location entry: 3-byte offset + 1-byte sector count
        let offset_bytes = ((sector_offset as u32) << 8) | (sectors_needed as u32 & 0xFF);
        file[i * 4..i * 4 + 4].copy_from_slice(&offset_bytes.to_be_bytes());

        // Write the chunk data
        let len = (data.len() + 1) as u32; // length includes compression byte
        file.extend_from_slice(&len.to_be_bytes());
        file.push(0x02); // zlib
        file.extend_from_slice(data);

        // Pad to sector boundary
        let pad = (sector_size - (file.len() % sector_size)) % sector_size;
        file.extend(std::iter::repeat_n(0u8, pad));

        timestamp_table[i] = timestamp;
    }

    // Fill timestamp table in header
    for (i, &ts) in timestamp_table.iter().enumerate() {
        let off = 4096 + i * 4;
        file[off..off + 4].copy_from_slice(&ts.to_be_bytes());
    }

    file
}

/// Encode a single Anvil region file from the given chunk list (in-memory
/// `save` path). Thin wrapper over [`compress_chunk`] + [`assemble_region_file`]
/// so the in-memory and streaming writers share one region-file assembler.
fn encode_region(
    chunk_coords: &[(i32, i32)],
    chunks: &HashMap<(i32, i32), ChunkData>,
    block_entities: &HashMap<(i32, i32), Vec<Vec<u8>>>,
) -> Result<Vec<u8>> {
    let mut slots: [Option<Vec<u8>>; 1024] = std::array::from_fn(|_| None);
    for &(cx, cz) in chunk_coords {
        let chunk = chunks.get(&(cx, cz)).context("missing chunk")?;
        let rx = cx.rem_euclid(32);
        let rz = cz.rem_euclid(32);
        let idx = (rz * 32 + rx) as usize;
        slots[idx] = Some(compress_chunk(
            cx,
            cz,
            chunk,
            block_entities.get(&(cx, cz)),
        )?);
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as u32;
    Ok(assemble_region_file(&slots, now))
}

/// Zlib-compress a byte slice.
fn zlib_compress(data: &[u8]) -> Result<Vec<u8>> {
    let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
    enc.write_all(data)?;
    enc.finish().context("zlib compression")
}

/// Write the Java `session.lock` file (8-byte big-endian timestamp).
fn write_session_lock(output: &Path) -> Result<()> {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    std::fs::write(output.join("session.lock"), timestamp.to_be_bytes())?;
    Ok(())
}

// ── Streaming region writer (ARC-001) ─────────────────────────────────────

/// One 32×32 region's worth of compressed-chunk slots, buffered until the
/// region is sealed (all tiles that can write into it have flushed).
struct RegionBuffer {
    slots: [Option<Vec<u8>>; 1024],
}

impl RegionBuffer {
    fn new() -> Self {
        Self {
            slots: std::array::from_fn(|_| None),
        }
    }
}

/// Streaming-mode state for [`JavaWorld`]. Lazily groups each tile's drained
/// chunks into 32×32 region buffers and writes each `r.{rx}.{rz}.mca` to disk
/// once the tile containing the region's maximum in-bounds chunk has flushed.
///
/// A region (32×32 chunks) can straddle up to four 64×64 tiles, so a region
/// cannot be written the moment a single tile flushes. Instead it stays
/// buffered until its *completing* tile — the row-major-latest tile that
/// touches it, identified as the tile containing the region's max in-bounds
/// chunk — has flushed. Because the tile pipeline flushes in row-major order
/// and writes every chunk before flushing, once that completing tile has
/// flushed the region's slots are fully populated and can be sealed. See
/// [`StreamingAnvil::seal_completed`] for the predicate.
struct StreamingAnvil {
    region_dir: PathBuf,
    /// World chunk bounds: `(min_cx, max_cx, min_cz, max_cz)`.
    bounds: (i32, i32, i32, i32),
    /// Most recent `set_tile_bounds` rectangle: `(tcx0, tcx1, tcz0, tcz1)`.
    cur_tile: (i32, i32, i32, i32),
    /// Buffered regions not yet sealed: `(rx, rz) → buffer`.
    regions: HashMap<(i32, i32), RegionBuffer>,
}

impl StreamingAnvil {
    fn new(output: &Path, min_cx: i32, max_cx: i32, min_cz: i32, max_cz: i32) -> Result<Self> {
        let region_dir = output.join("region");
        std::fs::create_dir_all(&region_dir)?;
        Ok(Self {
            region_dir,
            bounds: (min_cx, max_cx, min_cz, max_cz),
            cur_tile: (min_cx, min_cx, min_cz, min_cz),
            regions: HashMap::new(),
        })
    }

    /// Record the active tile rectangle (called from `set_tile_bounds`).
    fn set_tile(&mut self, tcx0: i32, tcx1: i32, tcz0: i32, tcz1: i32) {
        self.cur_tile = (tcx0, tcx1, tcz0, tcz1);
    }

    /// Slot a compressed chunk into its region buffer at the region-local index.
    fn store_chunk(&mut self, cx: i32, cz: i32, compressed: Vec<u8>) {
        let rx = cx.div_euclid(32);
        let rz = cz.div_euclid(32);
        let lrx = cx.rem_euclid(32);
        let lrz = cz.rem_euclid(32);
        let idx = (lrz * 32 + lrx) as usize;
        self.regions
            .entry((rx, rz))
            .or_insert_with(RegionBuffer::new)
            .slots[idx] = Some(compressed);
    }

    /// Write and drop every region whose completing tile has flushed, plus any
    /// region with no in-bounds chunks. Returns the number of regions sealed.
    fn seal_completed(&mut self) -> Result<usize> {
        let to_seal: Vec<(i32, i32)> = self
            .regions
            .keys()
            .copied()
            .filter(|&(rx, rz)| self.region_is_sealed(rx, rz))
            .collect();
        // Deterministic write order so byte-parity tests are stable regardless
        // of HashMap iteration order.
        let mut count = 0usize;
        for key in to_seal {
            if let Some(buf) = self.regions.remove(&key) {
                write_region_file(&self.region_dir, key.0, key.1, &buf.slots)?;
                count += 1;
            }
        }
        Ok(count)
    }

    /// Write and drop every remaining buffered region (called from `save`).
    fn seal_all(&mut self) -> Result<()> {
        let keys: Vec<(i32, i32)> = self.regions.keys().copied().collect();
        for key in keys {
            if let Some(buf) = self.regions.remove(&key) {
                write_region_file(&self.region_dir, key.0, key.1, &buf.slots)?;
            }
        }
        Ok(())
    }

    /// A region `(rx, rz)` is sealed once the tile containing its maximum
    /// in-bounds chunk has been flushed. Tiles flush in row-major `(tcx0, tcz0)`
    /// order, so "flushed" means the completing tile's origin is at or before
    /// the current tile's origin lexicically. A region with no in-bounds chunks
    /// is reported sealed so its (empty) buffer is dropped.
    fn region_is_sealed(&self, rx: i32, rz: i32) -> bool {
        let (min_cx, max_cx, min_cz, max_cz) = self.bounds;
        let (cur_tcx0, _tcx1, cur_tcz0, _tcz1) = self.cur_tile;
        let region_min_cx = rx * 32;
        let region_min_cz = rz * 32;
        // Region entirely outside world bounds → empty buffer; drop it.
        if region_min_cx > max_cx
            || region_min_cz > max_cz
            || region_min_cx + 31 < min_cx
            || region_min_cz + 31 < min_cz
        {
            return true;
        }
        let max_chunk_cx = (region_min_cx + 31).min(max_cx);
        let max_chunk_cz = (region_min_cz + 31).min(max_cz);
        let o_tcx0 = tile_origin_of(max_chunk_cx, min_cx);
        let o_tcz0 = tile_origin_of(max_chunk_cz, min_cz);
        (o_tcx0, o_tcz0) <= (cur_tcx0, cur_tcz0)
    }
}

/// Assemble and write one region file from its slot buffer.
fn write_region_file(
    region_dir: &Path,
    rx: i32,
    rz: i32,
    slots: &[Option<Vec<u8>>; 1024],
) -> Result<()> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as u32;
    let path = region_dir.join(format!("r.{rx}.{rz}.mca"));
    let data = assemble_region_file(slots, now);
    std::fs::write(&path, &data).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// The tile origin (row-major grid starting at `min_coord`, step `TILE_CHUNKS`)
/// that contains `chunk_coord`.
fn tile_origin_of(chunk_coord: i32, min_coord: i32) -> i32 {
    min_coord + (chunk_coord - min_coord).div_euclid(TILE_CHUNKS) * TILE_CHUNKS
}

// ── Chunk NBT encoding ───────────────────────────────────────────────────

/// Encode a single chunk column as big-endian NBT.
fn encode_chunk_nbt(
    cx: i32,
    cz: i32,
    chunk: &ChunkData,
    entities: Option<&Vec<Vec<u8>>>,
) -> Result<Vec<u8>> {
    let mut buf: Vec<u8> = Vec::new();

    nbt_be::write_compound_start(&mut buf, "")?;
    nbt_be::write_int_tag(&mut buf, "DataVersion", 3465)?;
    nbt_be::write_int_tag(&mut buf, "xPos", cx)?;
    nbt_be::write_int_tag(&mut buf, "zPos", cz)?;
    nbt_be::write_int_tag(&mut buf, "yPos", -4)?; // MIN_Y / 16 = -64 / 16 = -4
    nbt_be::write_string_tag(&mut buf, "Status", "minecraft:full")?;

    // Heightmaps
    nbt_be::write_compound_start(&mut buf, "Heightmaps")?;
    let heightmap = compute_world_surface_heightmap(chunk);
    nbt_be::write_long_array_tag(&mut buf, "WORLD_SURFACE", &heightmap)?;
    nbt_be::write_end(&mut buf)?;

    // Sections
    let subchunks: Vec<_> = chunk.non_empty_subchunks().collect();
    nbt_be::write_list_start(&mut buf, "sections", TAG_COMPOUND, subchunks.len() as i32)?;
    for &(sy, blocks) in &subchunks {
        encode_section_nbt(&mut buf, sy, blocks)?;
    }

    // Block entities
    if let Some(entities) = entities
        && !entities.is_empty()
    {
        nbt_be::write_list_start(
            &mut buf,
            "block_entities",
            TAG_COMPOUND,
            entities.len() as i32,
        )?;
        for nbt_blob in entities {
            // Each entity is a pre-encoded complete NBT compound.
            // Strip the root TAG_Compound header and TAG_End to get just the
            // inner payload, since write_list_start has already written the
            // list header with item type = TAG_COMPOUND.
            //
            // Java sign entities from nbt_be already include the
            // TAG_Compound header + name + content + TAG_End. In a
            // TAG_List of TAG_Compound, each entry is just the content
            // (no type byte, no name). So we need to strip the leading
            // tag header and trailing TAG_End.
            strip_and_write_compound_payload(&mut buf, nbt_blob);
        }
    }

    nbt_be::write_end(&mut buf)?; // end root compound

    Ok(buf)
}

/// Strip the TAG_Compound header (type byte + name) and trailing TAG_End from
/// a complete NBT compound blob, writing just the inner payload suitable for
/// a TAG_List entry.
fn strip_and_write_compound_payload(buf: &mut Vec<u8>, nbt_blob: &[u8]) {
    if nbt_blob.is_empty() {
        return;
    }
    // First byte is TAG_COMPOUND (10), then name (2-byte length + chars)
    if nbt_blob[0] != TAG_COMPOUND {
        return;
    }
    let name_len = u16::from_be_bytes([nbt_blob[1], nbt_blob[2]]) as usize;
    let content_start = 3 + name_len;
    // Content ends before the last byte (TAG_End = 0)
    if nbt_blob.len() > content_start + 1 {
        buf.extend_from_slice(&nbt_blob[content_start..nbt_blob.len() - 1]);
    }
}

/// Encode one 16x16x16 section as a compound payload (no tag header).
fn encode_section_nbt(buf: &mut Vec<u8>, sy: i8, blocks: &[Block; 4096]) -> Result<()> {
    // Build palette of unique (name, properties, direction) entries
    let mut palette: Vec<JavaPaletteKey> = Vec::new();
    let mut palette_map: HashMap<JavaPaletteKey, usize> = HashMap::new();

    // Air always first
    let air_key = JavaPaletteKey {
        block: Block::Air,
        direction: 0,
    };
    palette.push(air_key);
    palette_map.insert(air_key, 0);

    let mut indices = [0usize; 4096];
    for (i, &block) in blocks.iter().enumerate() {
        let key = JavaPaletteKey {
            block,
            direction: 0, // simplified — directions are in the block states already
        };
        let idx = match palette_map.entry(key) {
            std::collections::hash_map::Entry::Occupied(e) => *e.get(),
            std::collections::hash_map::Entry::Vacant(e) => {
                let new_idx = palette.len();
                e.insert(new_idx);
                palette.push(key);
                new_idx
            }
        };
        indices[i] = idx;
    }

    // Write compound payload (inside a TAG_List, so no tag header)
    nbt_be::write_byte_tag(buf, "Y", sy)?;
    nbt_be::write_compound_start(buf, "block_states")?;

    // Palette
    nbt_be::write_list_start(buf, "palette", TAG_COMPOUND, palette.len() as i32)?;
    for pkey in &palette {
        // Each palette entry: compound payload (no tag header in list)
        write_palette_entry_payload(buf, pkey.block);
    }

    // Data (packed long array) — only if palette has more than 1 entry
    if palette.len() > 1 {
        let bits = compute_bits_per_block(palette.len());
        let longs = pack_indices_long_array(&indices, bits);
        nbt_be::write_long_array_tag(buf, "data", &longs)?;
    }

    nbt_be::write_end(buf)?; // end block_states

    // Biomes
    nbt_be::write_compound_start(buf, "biomes")?;
    let dominant_biome = compute_dominant_biome(blocks);
    nbt_be::write_list_start(buf, "palette", TAG_STRING, 1)?;
    nbt_be::write_string_payload(buf, dominant_biome)?;
    nbt_be::write_end(buf)?; // end biomes

    Ok(())
}

/// Write a palette entry compound payload (Name + optional Properties).
fn write_palette_entry_payload(buf: &mut Vec<u8>, block: Block) {
    // This is a compound payload inside a TAG_List, so no tag header.
    // But our nbt_be helpers always write tag headers. We need to work around this.
    //
    // We'll build the compound in a temp buffer and strip the tag header.
    let mut tmp: Vec<u8> = Vec::new();
    nbt_be::write_compound_start(&mut tmp, "").unwrap();
    nbt_be::write_string_tag(&mut tmp, "Name", block.java_name()).unwrap();

    let states = block.java_block_states();
    if !states.is_empty() {
        nbt_be::write_compound_start(&mut tmp, "Properties").unwrap();
        for (key, value) in &states {
            nbt_be::write_string_tag(&mut tmp, key, value).unwrap();
        }
        nbt_be::write_end(&mut tmp).unwrap(); // end Properties
    }

    nbt_be::write_end(&mut tmp).unwrap(); // end compound

    // Strip tag header (type byte + empty name = 3 bytes) and trailing TAG_End
    strip_and_write_compound_payload(buf, &tmp);
}

// ── Heightmap ─────────────────────────────────────────────────────────────

/// Compute WORLD_SURFACE heightmap: 256 Y-values packed as 9-bit entries
/// into 36 longs. Index order: z * 16 + x.
fn compute_world_surface_heightmap(chunk: &ChunkData) -> [i64; 36] {
    let mut heights = [0i32; 256];
    for z in 0..16i32 {
        for x in 0..16i32 {
            let mut h: i32 = 0;
            for y in (MIN_Y..=MAX_Y).rev() {
                if chunk.get(x, y, z) != Block::Air {
                    h = y + 1;
                    break;
                }
            }
            heights[(z * 16 + x) as usize] = h;
        }
    }
    pack_heightmap_9bit(&heights)
}

/// Pack 256 values as 9-bit entries into 36 i64s (256 * 9 = 2304 bits = 36 * 64).
fn pack_heightmap_9bit(values: &[i32; 256]) -> [i64; 36] {
    let mut longs = [0i64; 36];
    let mut bit_offset: usize = 0;
    for &val in values {
        let long_idx = bit_offset / 64;
        let bit_idx = bit_offset % 64;
        let v = (val as i64) & 0x1FF; // 9-bit mask
        longs[long_idx] |= v << bit_idx;
        // Handle overflow into next long
        if bit_idx + 9 > 64 {
            let overflow = bit_idx + 9 - 64;
            longs[long_idx + 1] |= v >> (9 - overflow);
        }
        bit_offset += 9;
    }
    longs
}

// ── Biome helpers ─────────────────────────────────────────────────────────

/// Determine the dominant biome for a section by picking the most common
/// non-Air block's biome.
fn compute_dominant_biome(blocks: &[Block; 4096]) -> &'static str {
    let mut biome_counts: HashMap<&str, usize> = HashMap::new();
    for &block in blocks {
        if block != Block::Air {
            let biome = surface_to_java_biome(block);
            *biome_counts.entry(biome).or_insert(0) += 1;
        }
    }
    biome_counts
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .map(|(biome, _)| biome)
        .unwrap_or("minecraft:plains")
}

// ── Block state packing ──────────────────────────────────────────────────

/// Compute bits-per-block for a palette of the given size.
/// Java uses: max(4, ceil(log2(palette_size))), capped at 16.
fn compute_bits_per_block(palette_size: usize) -> usize {
    if palette_size <= 1 {
        return 4; // minimum
    }
    let mut bits = 4usize;
    while (1usize << bits) < palette_size {
        bits += 1;
    }
    bits.min(16)
}

/// Pack 4096 palette indices into a long array using the given bits-per-block.
/// Java packs from low bits. Each long holds floor(64 / bits) entries.
fn pack_indices_long_array(indices: &[usize; 4096], bits: usize) -> Vec<i64> {
    let entries_per_long = 64 / bits;
    let long_count = 4096_usize.div_ceil(entries_per_long);
    let mut longs = vec![0i64; long_count];

    for (i, &idx) in indices.iter().enumerate() {
        let long_idx = i / entries_per_long;
        let bit_offset = (i % entries_per_long) * bits;
        longs[long_idx] |= (idx as i64) << bit_offset;
    }

    longs
}

// ── level.dat ─────────────────────────────────────────────────────────────

/// Write a gzip-compressed BE NBT level.dat for Java Edition.
fn write_level_dat(output: &Path, spawn_x: i32, spawn_y: i32, spawn_z: i32) -> Result<()> {
    let mut nbt: Vec<u8> = Vec::new();

    nbt_be::write_compound_start(&mut nbt, "")?;
    nbt_be::write_compound_start(&mut nbt, "Data")?;

    nbt_be::write_int_tag(&mut nbt, "DataVersion", 3465)?;

    nbt_be::write_compound_start(&mut nbt, "Version")?;
    nbt_be::write_string_tag(&mut nbt, "Name", "1.21.4")?;
    nbt_be::write_int_tag(&mut nbt, "Id", 3465)?;
    nbt_be::write_byte_tag(&mut nbt, "Snapshot", 0)?;
    nbt_be::write_end(&mut nbt)?; // end Version

    nbt_be::write_int_tag(&mut nbt, "GameType", 1)?; // creative
    nbt_be::write_int_tag(&mut nbt, "SpawnX", spawn_x)?;
    nbt_be::write_int_tag(&mut nbt, "SpawnY", spawn_y)?;
    nbt_be::write_int_tag(&mut nbt, "SpawnZ", spawn_z)?;
    nbt_be::write_byte_tag(&mut nbt, "allowCommands", 1)?;
    nbt_be::write_string_tag(&mut nbt, "LevelName", "OSM World")?;

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    nbt_be::write_long_tag(&mut nbt, "LastPlayed", now_ms)?;
    nbt_be::write_byte_tag(&mut nbt, "hardcore", 0)?;
    nbt_be::write_byte_tag(&mut nbt, "initialized", 1)?;

    nbt_be::write_compound_start(&mut nbt, "WorldGenSettings")?;
    nbt_be::write_long_tag(&mut nbt, "seed", 0)?;
    nbt_be::write_byte_tag(&mut nbt, "generate_features", 0)?;
    nbt_be::write_end(&mut nbt)?; // end WorldGenSettings

    nbt_be::write_end(&mut nbt)?; // end Data
    nbt_be::write_end(&mut nbt)?; // end root

    // Gzip-compress
    let mut gz = GzEncoder::new(Vec::new(), Compression::default());
    gz.write_all(&nbt)?;
    let compressed = gz.finish().context("gzip level.dat")?;

    let path = output.join("level.dat");
    std::fs::write(&path, &compressed).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_world_set_get_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let mut world = JavaWorld::new(dir.path());
        world.set_block(10, 65, 20, Block::Stone);
        assert_eq!(world.get_block(10, 65, 20), Block::Stone);
        assert_eq!(world.get_block(10, 66, 20), Block::Air);
        assert_eq!(world.chunk_count(), 1);
    }

    #[test]
    fn java_world_bounded_ignores_outside() {
        let dir = tempfile::tempdir().unwrap();
        let mut world = JavaWorld::new_bounded(dir.path(), 0, 1, 0, 1);
        world.set_block(0, 65, 0, Block::Stone);
        world.set_block(100, 65, 100, Block::Stone); // chunk (6,6) — out of bounds
        assert_eq!(world.chunk_count(), 1);
    }

    #[test]
    fn java_world_save_creates_files() {
        let dir = tempfile::tempdir().unwrap();
        let mut world = JavaWorld::new(dir.path());
        world.set_block(0, 65, 0, Block::Stone);
        world.save(0, 65, 0).unwrap();
        assert!(dir.path().join("level.dat").exists());
        assert!(dir.path().join("region").exists());
        assert!(dir.path().join("session.lock").exists());
        // Check that region file was created
        let region_dir = dir.path().join("region");
        let mca_files: Vec<_> = std::fs::read_dir(&region_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "mca"))
            .collect();
        assert!(!mca_files.is_empty(), "expected at least one .mca file");
    }

    #[test]
    fn pack_indices_roundtrip() {
        let mut indices = [0usize; 4096];
        indices[0] = 3;
        indices[1] = 7;
        indices[4095] = 1;
        let bits = compute_bits_per_block(8);
        assert_eq!(bits, 4);
        let longs = pack_indices_long_array(&indices, bits);
        let mask = (1i64 << bits) - 1;
        assert_eq!((longs[0] & mask) as usize, 3);
        assert_eq!(((longs[0] >> bits) & mask) as usize, 7);
    }

    // ── Streaming writer (ARC-001) ──────────────────────────────────────────
    //
    // The streaming scenario below exercises everything at once: a world whose
    // `min_cx` (16) is NOT a multiple of 32, so tiles and regions misalign.
    // Bounds cx=[16..90] cz=[0..0] → two tiles [16..79] and [80..90]. The
    // region grid (32×32) puts chunks in region 0 (0..31), region 1 (32..63),
    // and region 2 (64..95); region 2 straddles both tiles (chunk 68 in tile
    // [16..79], chunk 84 in tile [80..90]) so it cannot seal until the second
    // tile flushes.

    #[test]
    fn streaming_drains_store_per_tile_and_writes_region_files() {
        let dir = tempfile::tempdir().unwrap();
        let mut w = JavaWorld::new_streaming(dir.path(), 16, 90, 0, 0).unwrap();

        // Tile [16..79]: three chunks across regions 0, 1, and 2.
        w.set_tile_bounds(16, 79, 0, 0);
        w.set_block(16 * 16, 65, 0, Block::Stone);
        w.set_block(40 * 16, 65, 0, Block::Dirt);
        w.set_block(68 * 16, 65, 0, Block::GrassBlock);
        assert_eq!(w.chunk_count(), 3);
        w.flush_tile().unwrap();
        // The scratch ChunkStore is drained — only compressed bytes remain in
        // the frontier region buffers (region 2 stays buffered; 0 and 1 seal).
        assert_eq!(w.chunk_count(), 0, "store drained after flush_tile");

        // Tile [80..90]: one chunk in the straddling region 2.
        w.set_tile_bounds(80, 90, 0, 0);
        w.set_block(84 * 16, 65, 0, Block::Cobblestone);
        assert_eq!(w.chunk_count(), 1);
        w.flush_tile().unwrap();
        assert_eq!(w.chunk_count(), 0, "store drained after second flush_tile");

        w.save(0, 65, 0).unwrap();

        let region_dir = dir.path().join("region");
        for (rx, rz) in [(0, 0), (1, 0), (2, 0)] {
            assert!(
                region_dir.join(format!("r.{rx}.{rz}.mca")).exists(),
                "expected region r.{rx}.{rz}.mca",
            );
        }
        assert!(dir.path().join("level.dat").exists());
        assert!(dir.path().join("session.lock").exists());
    }

    #[test]
    fn streaming_output_matches_in_memory_byte_for_byte() {
        // The streaming writer must produce the exact same region bytes as the
        // in-memory writer for the same blocks. This is the primary oracle: if
        // region-slot assignment, sealing, or assemble_region_file diverge, the
        // location table or chunk sectors will differ. Only the timestamp table
        // (bytes 4096..8192, stamped wall-clock now()) may differ.
        let dir_mem = tempfile::tempdir().unwrap();
        let dir_stream = tempfile::tempdir().unwrap();

        // In-memory (unbounded): write all four blocks, save once.
        let mut mem = JavaWorld::new(dir_mem.path());
        mem.set_block(16 * 16, 65, 0, Block::Stone);
        mem.set_block(40 * 16, 65, 0, Block::Dirt);
        mem.set_block(68 * 16, 65, 0, Block::GrassBlock);
        mem.set_block(84 * 16, 65, 0, Block::Cobblestone);
        mem.save(0, 65, 0).unwrap();

        // Streaming: same blocks, driven through two tiles.
        let mut stream = JavaWorld::new_streaming(dir_stream.path(), 16, 90, 0, 0).unwrap();
        stream.set_tile_bounds(16, 79, 0, 0);
        stream.set_block(16 * 16, 65, 0, Block::Stone);
        stream.set_block(40 * 16, 65, 0, Block::Dirt);
        stream.set_block(68 * 16, 65, 0, Block::GrassBlock);
        stream.flush_tile().unwrap();
        stream.set_tile_bounds(80, 90, 0, 0);
        stream.set_block(84 * 16, 65, 0, Block::Cobblestone);
        stream.flush_tile().unwrap();
        stream.save(0, 65, 0).unwrap();

        let mem_region = dir_mem.path().join("region");
        let stream_region = dir_stream.path().join("region");
        let mut mem_files: Vec<String> = std::fs::read_dir(&mem_region)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        mem_files.sort();
        assert_eq!(
            mem_files,
            vec!["r.0.0.mca", "r.1.0.mca", "r.2.0.mca"],
            "expected three region files",
        );
        for name in &mem_files {
            let a = std::fs::read(mem_region.join(name)).unwrap();
            let b = std::fs::read(stream_region.join(name))
                .expect("streaming must produce the same region files");
            assert!(
                a.len() == b.len() && a[..4096] == b[..4096] && a[8192..] == b[8192..],
                "region {name}: location table or chunk sectors differ \
                 (len a={}, b={})",
                a.len(),
                b.len(),
            );
        }
    }

    #[test]
    fn streaming_set_tile_bounds_is_noop_in_in_memory_mode() {
        // In-memory backends must keep the default no-op set_tile_bounds /
        // flush_tile behaviour (only streaming mode scopes the store).
        let dir = tempfile::tempdir().unwrap();
        let mut w = JavaWorld::new(dir.path());
        w.set_tile_bounds(0, 0, 0, 0);
        w.set_block(1000, 65, 1000, Block::Stone); // far outside the "tile"
        w.flush_tile().unwrap();
        // Unbounded in-memory writer still holds the chunk (not drained, not
        // filtered): flush_tile was a no-op.
        assert_eq!(w.chunk_count(), 1);
    }
}
