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
    world::{ChunkData, MAX_Y, MIN_Y, WorldWriter},
};
use anyhow::{Context, Result};
use flate2::{Compression, write::GzEncoder, write::ZlibEncoder};
use std::collections::HashMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};

// ── JavaWorld ─────────────────────────────────────────────────────────────

/// Accumulates chunk data in memory, then writes a Java Edition world
/// using Anvil region files.
pub struct JavaWorld {
    chunks: HashMap<(i32, i32), ChunkData>,
    output: PathBuf,
    block_entities: HashMap<(i32, i32), Vec<Vec<u8>>>,
    sign_directions: HashMap<(i32, i32, i32), i32>,
    block_directions: HashMap<(i32, i32, i32), i32>,
    chunk_bounds: Option<(i32, i32, i32, i32)>,
}

impl JavaWorld {
    pub fn new(output: &Path) -> Self {
        Self {
            chunks: HashMap::new(),
            output: output.to_path_buf(),
            block_entities: HashMap::new(),
            sign_directions: HashMap::new(),
            block_directions: HashMap::new(),
            chunk_bounds: None,
        }
    }

    pub fn new_bounded(output: &Path, min_cx: i32, max_cx: i32, min_cz: i32, max_cz: i32) -> Self {
        Self {
            chunks: HashMap::new(),
            output: output.to_path_buf(),
            block_entities: HashMap::new(),
            sign_directions: HashMap::new(),
            block_directions: HashMap::new(),
            chunk_bounds: Some((min_cx, max_cx, min_cz, max_cz)),
        }
    }

    /// Return `true` if (cx, cz) falls within the optional chunk bounds.
    #[inline]
    fn in_bounds(&self, cx: i32, cz: i32) -> bool {
        match self.chunk_bounds {
            None => true,
            Some((min_cx, max_cx, min_cz, max_cz)) => {
                cx >= min_cx && cx <= max_cx && cz >= min_cz && cz <= max_cz
            }
        }
    }

    /// Write the world to disk with spawn at the given block coordinates.
    pub fn save(&self, spawn_x: i32, spawn_y: i32, spawn_z: i32) -> Result<()> {
        std::fs::create_dir_all(&self.output)
            .with_context(|| format!("creating output dir {}", self.output.display()))?;

        let region_dir = self.output.join("region");
        std::fs::create_dir_all(&region_dir)?;

        // Group chunks into 32x32 regions
        let mut regions: HashMap<(i32, i32), Vec<(i32, i32)>> = HashMap::new();
        for &(cx, cz) in self.chunks.keys() {
            let rx = cx.div_euclid(32);
            let rz = cz.div_euclid(32);
            regions.entry((rx, rz)).or_default().push((cx, cz));
        }

        for (&(rx, rz), chunks) in &regions {
            let path = region_dir.join(format!("r.{rx}.{rz}.mca"));
            let data = encode_region(chunks, &self.chunks, &self.block_entities)?;
            std::fs::write(&path, &data).with_context(|| format!("writing {}", path.display()))?;
        }

        // level.dat (gzip-compressed BE NBT)
        write_level_dat(&self.output, spawn_x, spawn_y, spawn_z)?;

        // session.lock (8-byte timestamp)
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        std::fs::write(self.output.join("session.lock"), timestamp.to_be_bytes())?;

        Ok(())
    }
}

impl WorldWriter for JavaWorld {
    fn set_block(&mut self, x: i32, y: i32, z: i32, block: Block) {
        let cx = x.div_euclid(16);
        let cz = z.div_euclid(16);
        if !self.in_bounds(cx, cz) {
            return;
        }
        let lx = x.rem_euclid(16);
        let lz = z.rem_euclid(16);
        self.chunks
            .entry((cx, cz))
            .or_default()
            .set(lx, y, lz, block);
    }

    fn get_block(&self, x: i32, y: i32, z: i32) -> Block {
        let cx = x.div_euclid(16);
        let cz = z.div_euclid(16);
        let lx = x.rem_euclid(16);
        let lz = z.rem_euclid(16);
        self.chunks
            .get(&(cx, cz))
            .map(|chunk| chunk.get(lx, y, lz))
            .unwrap_or(Block::Air)
    }

    fn insert_chunk(&mut self, cx: i32, cz: i32, chunk: ChunkData) {
        self.chunks.insert((cx, cz), chunk);
    }

    fn add_block_entity(&mut self, x: i32, _y: i32, z: i32, nbt: Vec<u8>) {
        let cx = x.div_euclid(16);
        let cz = z.div_euclid(16);
        if !self.in_bounds(cx, cz) {
            return;
        }
        self.block_entities.entry((cx, cz)).or_default().push(nbt);
    }

    fn set_sign_direction(&mut self, x: i32, y: i32, z: i32, direction: i32) {
        let cx = x.div_euclid(16);
        let cz = z.div_euclid(16);
        if !self.in_bounds(cx, cz) {
            return;
        }
        self.sign_directions.insert((x, y, z), direction);
    }

    fn set_block_direction(&mut self, x: i32, y: i32, z: i32, direction: i32) {
        let cx = x.div_euclid(16);
        let cz = z.div_euclid(16);
        if !self.in_bounds(cx, cz) {
            return;
        }
        self.block_directions.insert((x, y, z), direction);
    }

    fn chunk_count(&self) -> usize {
        self.chunks.len()
    }

    fn occupied_chunks(&self) -> Vec<(i32, i32)> {
        self.chunks.keys().copied().collect()
    }

    fn save(&self, spawn_x: i32, spawn_y: i32, spawn_z: i32) -> Result<()> {
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

/// Encode a single Anvil region file from the given chunk list.
fn encode_region(
    chunk_coords: &[(i32, i32)],
    chunks: &HashMap<(i32, i32), ChunkData>,
    block_entities: &HashMap<(i32, i32), Vec<Vec<u8>>>,
) -> Result<Vec<u8>> {
    let mut chunk_data: [Option<Vec<u8>>; 1024] = std::array::from_fn(|_| None);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as u32;

    for &(cx, cz) in chunk_coords {
        let chunk = chunks.get(&(cx, cz)).context("missing chunk")?;
        let rx = cx.rem_euclid(32);
        let rz = cz.rem_euclid(32);
        let idx = (rz * 32 + rx) as usize;

        let nbt = encode_chunk_nbt(cx, cz, chunk, block_entities.get(&(cx, cz)))?;
        let compressed = zlib_compress(&nbt)?;
        chunk_data[idx] = Some(compressed);
    }

    // Build the region file: header + chunk sectors
    let mut file: Vec<u8> = Vec::new();

    // Location table: 1024 x 4-byte entries (offset + sector count)
    // Placeholder — filled once we know actual positions
    let header_size: usize = 8192;
    file.resize(header_size, 0u8);

    let mut timestamp_table = [0u32; 1024];
    let sector_size: usize = 4096;

    for (i, opt_data) in chunk_data.iter().enumerate() {
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

        timestamp_table[i] = now;
    }

    // Fill timestamp table in header
    for (i, &ts) in timestamp_table.iter().enumerate() {
        let off = 4096 + i * 4;
        file[off..off + 4].copy_from_slice(&ts.to_be_bytes());
    }

    Ok(file)
}

/// Zlib-compress a byte slice.
fn zlib_compress(data: &[u8]) -> Result<Vec<u8>> {
    let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
    enc.write_all(data)?;
    enc.finish().context("zlib compression")
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
}
