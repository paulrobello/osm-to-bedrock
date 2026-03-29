//! Bedrock Edition world writer.
//!
//! Generates a LevelDB database with Bedrock-format SubChunks and a level.dat.
//!
//! ## Bedrock chunk key format
//! ```text
//! [chunk_x: i32 LE][chunk_z: i32 LE][tag: u8]          (9 bytes)
//! [chunk_x: i32 LE][chunk_z: i32 LE][0x2f][sy: u8]     (10 bytes, SubChunk)
//! ```
//!
//! ## SubChunk format (version 8, persistence storage)
//! ```text
//! [0x08]                  version
//! [1]                     storage count
//! [(bits<<1)|0]           bits-per-block flags (bit 0 = 0 for disk/NBT palette)
//! [u32 LE words…]         4096 packed block indices (XZY order)
//! [palette_len: u32 LE]
//! [NBT compound…]         one little-endian NBT compound per palette entry
//! ```
//!
//! ## Async writes
//!
//! [`ChunkWriter`] owns a background thread that holds the LevelDB [`DB`].
//! Callers encode subchunks on their own thread(s) and send the resulting
//! `(key, value)` byte vectors over a bounded channel.  This pipelines
//! CPU-intensive palette encoding with disk I/O.

use crate::{
    blocks::{Block, BlockState},
    nbt::{
        write_byte_tag, write_compound_start, write_end, write_float_tag, write_int_tag,
        write_long_tag, write_string_tag,
    },
};
use anyhow::{Context, Result};
use flate2::Compression;
use flate2::read::{DeflateDecoder, ZlibDecoder};
use flate2::write::{DeflateEncoder, ZlibEncoder};
use rusty_leveldb::{Compressor, CompressorList, DB, Options};
use std::collections::HashMap;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc;

// ── Mojang LevelDB compressors ──────────────────────────────────────────────
// Bedrock's LevelDB fork prefixes each block with a 1-byte compressor ID.
// Modern worlds use ID 4 (raw deflate). Older worlds may use ID 2 (zlib).

const ZLIB_ID: u8 = 2;
const RAW_DEFLATE_ID: u8 = 4;

fn compress_err(e: impl std::fmt::Display) -> rusty_leveldb::Status {
    rusty_leveldb::Status {
        code: rusty_leveldb::StatusCode::CompressionError,
        err: e.to_string(),
    }
}

struct NoneCompressor;
impl Compressor for NoneCompressor {
    fn encode(&self, block: Vec<u8>) -> rusty_leveldb::Result<Vec<u8>> {
        Ok(block)
    }
    fn decode(&self, block: Vec<u8>) -> rusty_leveldb::Result<Vec<u8>> {
        Ok(block)
    }
}

struct BedrockZlibCompressor;
impl Compressor for BedrockZlibCompressor {
    fn encode(&self, block: Vec<u8>) -> rusty_leveldb::Result<Vec<u8>> {
        let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
        enc.write_all(&block).map_err(compress_err)?;
        enc.finish().map_err(compress_err)
    }
    fn decode(&self, block: Vec<u8>) -> rusty_leveldb::Result<Vec<u8>> {
        let mut out = Vec::new();
        ZlibDecoder::new(&block[..])
            .read_to_end(&mut out)
            .map_err(compress_err)?;
        Ok(out)
    }
}

struct RawDeflateCompressor;
impl Compressor for RawDeflateCompressor {
    fn encode(&self, block: Vec<u8>) -> rusty_leveldb::Result<Vec<u8>> {
        let mut enc = DeflateEncoder::new(Vec::new(), Compression::fast());
        enc.write_all(&block).map_err(compress_err)?;
        enc.finish().map_err(compress_err)
    }
    fn decode(&self, block: Vec<u8>) -> rusty_leveldb::Result<Vec<u8>> {
        let mut out = Vec::new();
        DeflateDecoder::new(&block[..])
            .read_to_end(&mut out)
            .map_err(compress_err)?;
        Ok(out)
    }
}

fn bedrock_compressor_list() -> Rc<CompressorList> {
    let mut list = CompressorList::new();
    list.set_with_id(0, NoneCompressor);
    list.set_with_id(ZLIB_ID, BedrockZlibCompressor);
    list.set_with_id(RAW_DEFLATE_ID, RawDeflateCompressor);
    Rc::new(list)
}

// ── Chunk keys ─────────────────────────────────────────────────────────────

const TAG_VERSION: u8 = 0x2c; // ChunkVersion (Bedrock 1.16.100+)
const TAG_DATA_2D: u8 = 0x2d; // Data2D (heightmap + biomes)
const TAG_SUBCHUNK: u8 = 0x2f; // SubChunkPrefix
const TAG_BLOCK_ENTITY: u8 = 0x31; // BlockEntity
const TAG_FINALIZED: u8 = 0x36; // FinalizedState

// ── World Y-range constants (Bedrock 1.18+) ─────────────────────────────
/// Minimum Y coordinate (bottom of the world).
pub const MIN_Y: i32 = -64;
/// Maximum Y coordinate (top of the world, inclusive).
pub const MAX_Y: i32 = 319;
/// Total world height in blocks.
#[allow(dead_code)]
pub const WORLD_HEIGHT: i32 = MAX_Y - MIN_Y + 1; // 384

fn chunk_key(cx: i32, cz: i32, tag: u8) -> Vec<u8> {
    let mut k = Vec::with_capacity(9);
    k.extend_from_slice(&cx.to_le_bytes());
    k.extend_from_slice(&cz.to_le_bytes());
    k.push(tag);
    k
}

fn subchunk_key(cx: i32, cz: i32, sy: i8) -> Vec<u8> {
    let mut k = Vec::with_capacity(10);
    k.extend_from_slice(&cx.to_le_bytes());
    k.extend_from_slice(&cz.to_le_bytes());
    k.push(TAG_SUBCHUNK);
    k.push(sy as u8);
    k
}

// ── ChunkData ─────────────────────────────────────────────────────────────

/// In-memory representation of one 16×(height)×16 chunk column.
///
/// Blocks are stored in sub-chunks of 16×16×16, indexed XZY (x*256 + z*16 + y_local).
/// Only non-empty sub-chunks are allocated.
#[derive(Default)]
pub struct ChunkData {
    /// Map from sub-chunk Y index → block array (4096 entries, XZY).
    subchunks: HashMap<i8, Box<[Block; 4096]>>,
}

impl ChunkData {
    pub fn new() -> Self {
        Self::default()
    }

    fn idx(lx: i32, ly: i32, lz: i32) -> usize {
        // XZY order: x * 256 + z * 16 + y_local
        (lx as usize) * 256 + (lz as usize) * 16 + ly as usize
    }

    /// Set a block at local-x, world-y, local-z.
    pub fn set(&mut self, lx: i32, y: i32, lz: i32, block: Block) {
        let sy = y.div_euclid(16) as i8;
        let ly = y.rem_euclid(16);
        let entry = self
            .subchunks
            .entry(sy)
            .or_insert_with(|| Box::new([Block::Air; 4096]));
        entry[Self::idx(lx, ly, lz)] = block;
    }

    /// Get a block at local-x, world-y, local-z.
    pub fn get(&self, lx: i32, y: i32, lz: i32) -> Block {
        let sy = y.div_euclid(16) as i8;
        let ly = y.rem_euclid(16);
        self.subchunks
            .get(&sy)
            .map(|sc| sc[Self::idx(lx, ly, lz)])
            .unwrap_or(Block::Air)
    }

    /// Iterate sub-chunks that have at least one non-air block.
    fn non_empty_subchunks(&self) -> impl Iterator<Item = (i8, &[Block; 4096])> {
        self.subchunks
            .iter()
            .map(|(&sy, blocks)| (sy, blocks.as_ref()))
    }
}

// ── ChunkWriter ────────────────────────────────────────────────────────────

/// A background LevelDB writer.
///
/// Owns a dedicated thread that holds the [`DB`] handle and receives
/// pre-encoded `(key, value)` byte-vector pairs over a bounded channel.
/// This lets callers overlap CPU-intensive subchunk encoding with disk I/O:
///
/// ```text
/// caller thread: encode_subchunk → send (key, bytes)
///                                         ↓ channel
/// writer thread:                    db.put(key, bytes)
/// ```
///
/// Create with [`ChunkWriter::open`], send data from multiple call sites,
/// then call [`ChunkWriter::finish`] to drain the channel and join the thread.
pub struct ChunkWriter {
    tx: mpsc::SyncSender<(Vec<u8>, Vec<u8>)>,
    /// `None` after `finish()` consumes the handle.
    thread: Option<std::thread::JoinHandle<()>>,
    /// The actual LevelDB error captured by the writer thread before it exits.
    /// `send()` reads this to surface the real cause instead of the generic
    /// "writer thread terminated early" message.
    thread_error: std::sync::Arc<std::sync::Mutex<Option<String>>>,
}

impl ChunkWriter {
    /// Open the LevelDB database at `db_path` and start the writer thread.
    ///
    /// The channel buffer holds up to 1024 pending write pairs so the encoder
    /// can run ahead of the writer without stalling.
    pub fn open(db_path: PathBuf) -> Result<Self> {
        let (tx, rx) = mpsc::sync_channel::<(Vec<u8>, Vec<u8>)>(1024);
        let thread_error = std::sync::Arc::new(std::sync::Mutex::new(None::<String>));
        let thread_error_writer = thread_error.clone();

        // Open the DB *inside* the spawned thread so the non-Send Rc<CompressorList>
        // never crosses a thread boundary.
        let thread = std::thread::spawn(move || {
            let opts = Options {
                create_if_missing: true,
                compressor: RAW_DEFLATE_ID,
                compressor_list: bedrock_compressor_list(),
                // Large SSTable files (64 MB) and write buffer (64 MB) dramatically
                // reduce the total number of files on disk (from thousands to tens),
                // keeping the process well under OS open-file limits (EMFILE).
                max_file_size: 64 << 20,
                write_buffer_size: 64 << 20,
                // Conservative open-file cap: LevelDB caches at most this many
                // SSTable file handles open simultaneously.
                max_open_files: 128,
                ..Options::default()
            };
            let mut db = match DB::open(&db_path, opts) {
                Ok(db) => db,
                Err(e) => {
                    *thread_error_writer.lock().unwrap() = Some(format!("opening LevelDB: {e:?}"));
                    return;
                }
            };
            for (key, value) in rx {
                if let Err(e) = db.put(&key, &value) {
                    *thread_error_writer.lock().unwrap() = Some(format!("LevelDB put: {e:?}"));
                    return; // dropping rx closes the channel; sender will get an error
                }
            }
        });

        Ok(ChunkWriter {
            tx,
            thread: Some(thread),
            thread_error,
        })
    }

    /// Encode and enqueue a single chunk for writing.
    ///
    /// Encoding (palette building, bit-packing, NBT) happens on the calling
    /// thread; only the resulting byte buffers are sent to the writer thread.
    pub fn write_chunk(
        &self,
        cx: i32,
        cz: i32,
        chunk: &ChunkData,
        block_entities: Option<&Vec<Vec<u8>>>,
        sign_directions: &HashMap<(i32, i32, i32), i32>,
        block_directions: &HashMap<(i32, i32, i32), i32>,
    ) -> Result<()> {
        // Chunk version
        self.send(chunk_key(cx, cz, TAG_VERSION), vec![40u8])?;

        // Finalized state = 2 (terrain generated)
        self.send(
            chunk_key(cx, cz, TAG_FINALIZED),
            2i32.to_le_bytes().to_vec(),
        )?;

        // Sub-chunks
        for (sy, blocks) in chunk.non_empty_subchunks() {
            let data = encode_subchunk(blocks, cx, cz, sy, sign_directions, block_directions)?;
            self.send(subchunk_key(cx, cz, sy), data)?;
        }

        // Data2D: heightmap + biomes
        self.send(chunk_key(cx, cz, TAG_DATA_2D), encode_data2d(chunk))?;

        // Block entities (concatenated NBT blobs)
        if let Some(entities) = block_entities {
            let mut blob = Vec::new();
            for nbt in entities {
                blob.extend_from_slice(nbt);
            }
            self.send(chunk_key(cx, cz, TAG_BLOCK_ENTITY), blob)?;
        }

        Ok(())
    }

    /// Send a raw key-value pair to the writer thread.
    ///
    /// If the thread has already died (channel closed), reads the captured
    /// error and returns it instead of the generic "terminated early" message.
    fn send(&self, key: Vec<u8>, value: Vec<u8>) -> Result<()> {
        if self.tx.send((key, value)).is_err() {
            let cause = self
                .thread_error
                .lock()
                .ok()
                .and_then(|g| g.clone())
                .unwrap_or_else(|| "LevelDB writer thread terminated early".to_string());
            return Err(anyhow::anyhow!("{cause}"));
        }
        Ok(())
    }

    /// Close the send side of the channel and wait for the writer thread to finish.
    ///
    /// Returns any error that occurred during writing.
    pub fn finish(mut self) -> Result<()> {
        // Drop the sender to signal the writer loop to exit.
        drop(self.tx);
        if let Some(thread) = self.thread.take() {
            thread
                .join()
                .map_err(|_| anyhow::anyhow!("LevelDB writer thread panicked"))?;
        }
        // Surface any error the thread stored before exiting.
        if let Ok(guard) = self.thread_error.lock()
            && let Some(ref e) = *guard
        {
            return Err(anyhow::anyhow!("{e}"));
        }
        Ok(())
    }
}

// ── BedrockWorld ──────────────────────────────────────────────────────────

/// Accumulates chunk data in memory, then writes a Bedrock world to disk.
///
/// For incremental (tile-based) processing, construct with
/// [`BedrockWorld::new_bounded`] to restrict chunk creation to a specific
/// chunk-coordinate rectangle.  Blocks outside the bounds are silently
/// ignored, keeping memory usage proportional to the active tile rather than
/// the entire map.
pub struct BedrockWorld {
    chunks: HashMap<(i32, i32), ChunkData>,
    output: PathBuf,
    /// Block entity NBT blobs, keyed by chunk coordinates.
    block_entities: HashMap<(i32, i32), Vec<Vec<u8>>>,
    /// Sign direction overrides, keyed by (x, y, z) world coordinates.
    sign_directions: HashMap<(i32, i32, i32), i32>,
    /// Direction overrides for directional blocks (stairs, rails), keyed by (x, y, z).
    block_directions: HashMap<(i32, i32, i32), i32>,
    /// Optional spatial bounds for incremental tile processing.
    /// When set, `set_block` and related methods silently ignore coordinates
    /// outside the given chunk-coordinate rectangle (min_cx, max_cx, min_cz, max_cz).
    chunk_bounds: Option<(i32, i32, i32, i32)>,
}

impl BedrockWorld {
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

    /// Create a world bounded to the chunk-coordinate rectangle
    /// `[min_cx, max_cx] × [min_cz, max_cz]`.
    ///
    /// Any `set_block` / `add_block_entity` / `set_sign_direction` /
    /// `set_block_direction` call whose chunk coordinates fall outside this
    /// rectangle is silently ignored.  This keeps the active chunk set small
    /// during tile-based streaming conversion.
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

    /// Set a block at absolute (x, y, z) world coordinates.
    pub fn set_block(&mut self, x: i32, y: i32, z: i32, block: Block) {
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

    /// Insert a pre-built ChunkData at (cx, cz), replacing any existing data.
    ///
    /// Used by the parallel terrain-fill path, where each chunk is constructed
    /// independently on a worker thread and then merged into the world serially.
    pub fn insert_chunk(&mut self, cx: i32, cz: i32, chunk: ChunkData) {
        self.chunks.insert((cx, cz), chunk);
    }

    /// Get a block at absolute (x, y, z) world coordinates.
    pub fn get_block(&self, x: i32, y: i32, z: i32) -> Block {
        let cx = x.div_euclid(16);
        let cz = z.div_euclid(16);
        let lx = x.rem_euclid(16);
        let lz = z.rem_euclid(16);
        self.chunks
            .get(&(cx, cz))
            .map(|chunk| chunk.get(lx, y, lz))
            .unwrap_or(Block::Air)
    }

    /// Add a block entity NBT blob at the given world coordinates.
    /// The blob is appended to the list for the chunk containing (x, z).
    pub fn add_block_entity(&mut self, x: i32, _y: i32, z: i32, nbt: Vec<u8>) {
        let cx = x.div_euclid(16);
        let cz = z.div_euclid(16);
        if !self.in_bounds(cx, cz) {
            return;
        }
        self.block_entities.entry((cx, cz)).or_default().push(nbt);
    }

    /// Set the sign direction (0-15) for a sign block at world coordinates.
    pub fn set_sign_direction(&mut self, x: i32, y: i32, z: i32, direction: i32) {
        let cx = x.div_euclid(16);
        let cz = z.div_euclid(16);
        if !self.in_bounds(cx, cz) {
            return;
        }
        self.sign_directions.insert((x, y, z), direction);
    }

    /// Get the sign direction for a block at world coordinates, defaulting to 0.
    #[allow(dead_code)]
    pub fn get_sign_direction(&self, x: i32, y: i32, z: i32) -> i32 {
        self.sign_directions.get(&(x, y, z)).copied().unwrap_or(0)
    }

    /// Set the direction for a directional block (stairs, rails) at world coordinates.
    pub fn set_block_direction(&mut self, x: i32, y: i32, z: i32, direction: i32) {
        let cx = x.div_euclid(16);
        let cz = z.div_euclid(16);
        if !self.in_bounds(cx, cz) {
            return;
        }
        self.block_directions.insert((x, y, z), direction);
    }

    /// Extract the top-most non-Air block at each (x, z) column.
    pub fn surface_blocks(&self) -> Vec<(i32, i32, i32, String)> {
        let mut result = Vec::new();
        for (&(cx, cz), chunk) in &self.chunks {
            for lx in 0..16i32 {
                for lz in 0..16i32 {
                    let wx = cx * 16 + lx;
                    let wz = cz * 16 + lz;
                    for y in (MIN_Y..=MAX_Y).rev() {
                        let b = chunk.get(lx, y, lz);
                        if b != Block::Air {
                            result.push((wx, wz, y, format!("{:?}", b)));
                            break;
                        }
                    }
                }
            }
        }
        result
    }

    /// Return the number of chunks currently in the world.
    #[allow(dead_code)]
    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }

    /// Returns all occupied chunk coordinates (copy, for iteration).
    #[allow(dead_code)]
    pub fn occupied_chunks(&self) -> Vec<(i32, i32)> {
        self.chunks.keys().copied().collect()
    }

    /// Write all accumulated chunks to the given [`ChunkWriter`] and clear
    /// the in-memory chunk map.
    ///
    /// This is the core of the incremental/streaming write path.  After this
    /// call the world is empty and can be reused for the next tile.
    pub fn drain_chunks_to_writer(&mut self, writer: &ChunkWriter) -> Result<()> {
        for ((cx, cz), chunk) in self.chunks.drain() {
            writer.write_chunk(
                cx,
                cz,
                &chunk,
                self.block_entities.get(&(cx, cz)),
                &self.sign_directions,
                &self.block_directions,
            )?;
        }
        // Clear per-chunk auxiliary data that has been flushed.
        self.block_entities.clear();
        self.sign_directions.clear();
        self.block_directions.clear();
        Ok(())
    }

    /// Write the world to disk with spawn at the given block coordinates.
    ///
    /// Uses a [`ChunkWriter`] internally so encoding and I/O are pipelined
    /// on separate threads.
    #[allow(dead_code)]
    pub fn save(&self, spawn_x: i32, spawn_y: i32, spawn_z: i32) -> Result<()> {
        std::fs::create_dir_all(&self.output)
            .with_context(|| format!("creating output dir {}", self.output.display()))?;

        let db_path = self.output.join("db");
        std::fs::create_dir_all(&db_path)?;

        let writer = ChunkWriter::open(db_path)?;

        for (&(cx, cz), chunk) in &self.chunks {
            writer
                .write_chunk(
                    cx,
                    cz,
                    chunk,
                    self.block_entities.get(&(cx, cz)),
                    &self.sign_directions,
                    &self.block_directions,
                )
                .with_context(|| format!("writing chunk ({cx},{cz})"))?;
        }

        writer.finish()?;
        self.write_level_dat(spawn_x, spawn_y, spawn_z)?;
        Ok(())
    }

    pub fn write_level_dat(&self, spawn_x: i32, spawn_y: i32, spawn_z: i32) -> Result<()> {
        let path = self.output.join("level.dat");

        let mut nbt: Vec<u8> = Vec::new();
        // Root compound (empty name)
        write_compound_start(&mut nbt, "")?;
        write_int_tag(&mut nbt, "StorageVersion", 10)?;
        write_int_tag(&mut nbt, "NetworkVersion", 594)?;
        write_string_tag(&mut nbt, "LevelName", "OSM World")?;
        write_int_tag(&mut nbt, "SpawnX", spawn_x)?;
        write_int_tag(&mut nbt, "SpawnY", spawn_y)?;
        write_int_tag(&mut nbt, "SpawnZ", spawn_z)?;
        write_long_tag(&mut nbt, "Time", 6000)?;
        write_long_tag(&mut nbt, "LastPlayed", 0)?;
        write_int_tag(&mut nbt, "Generator", 2)?; // 2 = flat
        write_int_tag(&mut nbt, "GameType", 1)?; // 1 = creative
        write_int_tag(&mut nbt, "Difficulty", 0)?; // 0 = peaceful (no hostile mobs)
        write_byte_tag(&mut nbt, "commandsEnabled", 1)?;
        write_byte_tag(&mut nbt, "hasBeenLoadedInCreative", 1)?;
        write_byte_tag(&mut nbt, "eduLevel", 0)?;
        // Player permissions: 1 = operator
        write_int_tag(&mut nbt, "PlayerPermissionsLevel", 2)?; // 2 = operator
        write_int_tag(&mut nbt, "defaultPlayerPermissions", 2)?; // 2 = operator
        // Show coordinates & copy coordinate UI
        write_byte_tag(&mut nbt, "showcoordinates", 1)?;
        write_byte_tag(&mut nbt, "enableCopyCoordinateUI", 1)?;
        // Game rules: always day, no weather, no mobs, no friendly fire
        write_byte_tag(&mut nbt, "dodaylightcycle", 0)?;
        write_byte_tag(&mut nbt, "doweathercycle", 0)?;
        write_byte_tag(&mut nbt, "domobspawning", 0)?;
        write_byte_tag(&mut nbt, "domobloot", 0)?;
        write_byte_tag(&mut nbt, "doentitydrops", 0)?;
        write_byte_tag(&mut nbt, "pvp", 0)?; // no friendly fire / PvP
        write_float_tag(&mut nbt, "rainLevel", 0.0)?;
        write_float_tag(&mut nbt, "lightningLevel", 0.0)?;
        write_int_tag(&mut nbt, "rainTime", 0)?;
        write_end(&mut nbt)?;

        // File header: [version: u32 LE][nbt_size: u32 LE][nbt...]
        let mut file: Vec<u8> = Vec::new();
        file.extend_from_slice(&10u32.to_le_bytes()); // Storage version 10
        file.extend_from_slice(&(nbt.len() as u32).to_le_bytes());
        file.extend_from_slice(&nbt);

        std::fs::write(&path, &file).with_context(|| format!("writing {}", path.display()))?;
        Ok(())
    }
}

// ── SubChunk encoding ─────────────────────────────────────────────────────

/// A palette key that distinguishes blocks with different states (e.g. directions).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct PaletteKey {
    block: Block,
    /// Direction value for directional blocks (signs, stairs, rails).
    direction: i32,
}

fn encode_subchunk(
    blocks: &[Block; 4096],
    cx: i32,
    cz: i32,
    sy: i8,
    sign_directions: &HashMap<(i32, i32, i32), i32>,
    block_directions: &HashMap<(i32, i32, i32), i32>,
) -> Result<Vec<u8>> {
    // Build palette (Air must be index 0 if present)
    let mut palette: Vec<PaletteKey> = Vec::new();
    let mut palette_map: HashMap<PaletteKey, u32> = HashMap::new();

    // Always put Air first
    let air_key = PaletteKey {
        block: Block::Air,
        direction: 0,
    };
    palette.push(air_key);
    palette_map.insert(air_key, 0);

    // Build palette keys for all 4096 blocks, resolving directions
    let mut block_keys: [PaletteKey; 4096] = [air_key; 4096];
    for i in 0..4096usize {
        let b = blocks[i];
        let dir = if b == Block::OakSign
            || matches!(b, Block::OakStairs | Block::StoneBrickStairs | Block::Rail)
        {
            // Recover world coordinates from subchunk-local index (XZY order)
            let lx = (i / 256) as i32;
            let lz = ((i % 256) / 16) as i32;
            let ly = (i % 16) as i32;
            let wx = cx * 16 + lx;
            let wy = sy as i32 * 16 + ly;
            let wz = cz * 16 + lz;
            if b == Block::OakSign {
                sign_directions.get(&(wx, wy, wz)).copied().unwrap_or(0)
            } else {
                block_directions.get(&(wx, wy, wz)).copied().unwrap_or(0)
            }
        } else {
            0
        };
        let key = PaletteKey {
            block: b,
            direction: dir,
        };
        block_keys[i] = key;
        if let std::collections::hash_map::Entry::Vacant(e) = palette_map.entry(key) {
            let idx = palette.len() as u32;
            e.insert(idx);
            palette.push(key);
        }
    }

    // Pick the smallest valid bits-per-block: 1, 2, 3, 4, 5, 6, 8, 16
    let bits = bits_for_palette(palette.len());

    let mut data: Vec<u8> = Vec::new();

    // Version byte
    data.push(8u8);
    // Storage count
    data.push(1u8);
    // Bits-per-block flags: (bits << 1) | 0  (bit 0 = 0 for disk/NBT palette format)
    data.push((bits as u8) << 1);

    // Pack 4096 indices into 32-bit words
    let blocks_per_word = 32 / bits;
    let word_count = 4096_usize.div_ceil(blocks_per_word);
    for w in 0..word_count {
        let mut word: u32 = 0;
        for b in 0..blocks_per_word {
            let idx = w * blocks_per_word + b;
            if idx < 4096 {
                word |= palette_map[&block_keys[idx]] << (b * bits);
            }
        }
        data.extend_from_slice(&word.to_le_bytes());
    }

    // Palette length
    data.extend_from_slice(&(palette.len() as u32).to_le_bytes());

    // Palette entries as little-endian NBT compounds
    for pkey in &palette {
        let mut entry: Vec<u8> = Vec::new();
        write_compound_start(&mut entry, "")?;
        write_string_tag(&mut entry, "name", pkey.block.bedrock_name())?;
        write_compound_start(&mut entry, "states")?;

        // Determine which state key is overridden by direction so we skip the
        // default from block_states() and write only the actual direction value.
        let direction_state_key: Option<&str> = if pkey.block == Block::OakSign {
            Some("ground_sign_direction")
        } else if matches!(pkey.block, Block::OakStairs | Block::StoneBrickStairs) {
            Some("weirdo_direction")
        } else if pkey.block == Block::Rail {
            Some("rail_direction")
        } else {
            None
        };

        // Write block states from the Block's block_states() method,
        // skipping any state whose key will be overridden by direction.
        for state in pkey.block.block_states() {
            let key = match &state {
                BlockState::Int(k, _) => *k,
                BlockState::Byte(k, _) => *k,
                BlockState::String(k, _) => *k,
            };
            if direction_state_key == Some(key) {
                continue; // will be written below with the actual direction value
            }
            match state {
                BlockState::Int(k, val) => write_int_tag(&mut entry, k, val)?,
                BlockState::Byte(k, val) => write_byte_tag(&mut entry, k, val)?,
                BlockState::String(k, val) => write_string_tag(&mut entry, k, val)?,
            }
        }

        // Write the direction override for directional blocks
        if pkey.block == Block::OakSign {
            write_int_tag(&mut entry, "ground_sign_direction", pkey.direction)?;
        } else if matches!(pkey.block, Block::OakStairs | Block::StoneBrickStairs) {
            write_int_tag(&mut entry, "weirdo_direction", pkey.direction)?;
        } else if pkey.block == Block::Rail {
            write_int_tag(&mut entry, "rail_direction", pkey.direction)?;
        }

        write_end(&mut entry)?; // end states
        write_int_tag(&mut entry, "version", 18_105_860)?;
        write_end(&mut entry)?; // end root compound
        data.extend_from_slice(&entry);
    }

    Ok(data)
}

fn bits_for_palette(count: usize) -> usize {
    let valid = [1, 2, 3, 4, 5, 6, 8, 16];
    for &b in &valid {
        if (1usize << b) >= count {
            return b;
        }
    }
    16
}

fn encode_data2d(chunk: &ChunkData) -> Vec<u8> {
    let mut data: Vec<u8> = Vec::with_capacity(512 + 256);
    // Biome array in ZX order (z outer, x inner), filled during heightmap scan
    let mut biomes = [1u8; 256];

    // Heightmap: 256 LE i16, ZX order (z outer, x inner)
    // Combined with biome scan — single pass per column
    for z in 0..16i32 {
        for x in 0..16i32 {
            let mut height: i16 = MIN_Y as i16;
            for y in (MIN_Y..=MAX_Y).rev() {
                let b = chunk.get(x, y, z);
                if b != Block::Air {
                    height = (y + 1) as i16;
                    biomes[(z * 16 + x) as usize] = crate::blocks::surface_to_biome(b);
                    break;
                }
            }
            data.extend_from_slice(&height.to_le_bytes());
        }
    }

    // Biome map: 256 bytes, ZX order
    data.extend_from_slice(&biomes);

    data
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blocks::Block;

    #[test]
    fn data2d_biome_water_column() {
        let mut chunk = ChunkData::new();
        chunk.set(0, 65, 0, Block::Water);
        let data = encode_data2d(&chunk);
        assert_eq!(data.len(), 768);
        // Biome at column (z=0, x=0) = index 0 in biome section = byte 512
        assert_eq!(data[512], 7, "Water column should have river biome (7)");
    }

    #[test]
    fn data2d_biome_default_plains() {
        let mut chunk = ChunkData::new();
        chunk.set(3, 65, 5, Block::GrassBlock);
        let data = encode_data2d(&chunk);
        // Biome at column (z=5, x=3) = index 5*16+3 = 83, byte 512+83
        assert_eq!(data[512 + 83], 1, "Grass column should be plains biome (1)");
    }

    #[test]
    fn data2d_biome_forest_column() {
        let mut chunk = ChunkData::new();
        chunk.set(1, 70, 2, Block::OakLog);
        let data = encode_data2d(&chunk);
        // Biome at column (z=2, x=1) = index 2*16+1 = 33, byte 512+33
        assert_eq!(
            data[512 + 33],
            4,
            "Oak log column should be forest biome (4)"
        );
    }

    #[test]
    fn data2d_biome_empty_column_is_plains() {
        let chunk = ChunkData::new();
        let data = encode_data2d(&chunk);
        for (i, &byte) in data.iter().enumerate().skip(512).take(256) {
            assert_eq!(
                byte, 1,
                "Empty column (index {i}) should default to plains biome"
            );
        }
    }

    #[test]
    fn bounded_world_ignores_out_of_bounds() {
        // A world bounded to chunk (0,0)..(0,0) should only accept blocks in that chunk.
        let dir = tempfile::tempdir().unwrap();
        let mut world = BedrockWorld::new_bounded(dir.path(), 0, 0, 0, 0);
        world.set_block(0, 65, 0, Block::Stone); // chunk (0,0) — inside
        world.set_block(16, 65, 0, Block::Stone); // chunk (1,0) — outside
        world.set_block(0, 65, 16, Block::Stone); // chunk (0,1) — outside
        assert_eq!(world.chunk_count(), 1, "Only one chunk should be created");
        assert_eq!(world.get_block(0, 65, 0), Block::Stone);
        assert_eq!(
            world.get_block(16, 65, 0),
            Block::Air,
            "Out-of-bounds should be Air"
        );
    }
}
