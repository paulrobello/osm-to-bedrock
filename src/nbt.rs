//! Minimal little-endian NBT writer for Bedrock Edition.
//!
//! Bedrock uses little-endian NBT (vs Java's big-endian).
//! We only implement the subset needed for SubChunk palettes and level.dat.

use anyhow::{Context, Result, bail, ensure};
use std::io::Write;

// NBT tag type IDs
pub const TAG_END: u8 = 0;
pub const TAG_BYTE: u8 = 1;
pub const TAG_INT: u8 = 3;
pub const TAG_LONG: u8 = 4;
pub const TAG_FLOAT: u8 = 5;
pub const TAG_STRING: u8 = 8;
pub const TAG_COMPOUND: u8 = 10;

/// Write a raw u16 (LE) — used for string length prefixes.
fn write_u16_le(w: &mut impl Write, v: u16) -> Result<()> {
    w.write_all(&v.to_le_bytes())?;
    Ok(())
}

/// Write a string payload (length-prefixed, LE).
pub fn write_string_payload(w: &mut impl Write, s: &str) -> Result<()> {
    write_u16_le(w, s.len() as u16)?;
    w.write_all(s.as_bytes())?;
    Ok(())
}

/// Write a named tag header: [tag_type][name_len_LE][name_bytes].
fn write_tag_header(w: &mut impl Write, tag_type: u8, name: &str) -> Result<()> {
    w.write_all(&[tag_type])?;
    write_string_payload(w, name)?;
    Ok(())
}

/// Open a TAG_Compound (writes type byte + name). Caller must close with `write_end`.
pub fn write_compound_start(w: &mut impl Write, name: &str) -> Result<()> {
    write_tag_header(w, TAG_COMPOUND, name)
}

/// Close a TAG_Compound or TAG_List with TAG_End.
pub fn write_end(w: &mut impl Write) -> Result<()> {
    w.write_all(&[TAG_END])?;
    Ok(())
}

/// Write a named TAG_String.
pub fn write_string_tag(w: &mut impl Write, name: &str, value: &str) -> Result<()> {
    write_tag_header(w, TAG_STRING, name)?;
    write_string_payload(w, value)?;
    Ok(())
}

/// Write a named TAG_Int (LE i32).
pub fn write_int_tag(w: &mut impl Write, name: &str, value: i32) -> Result<()> {
    write_tag_header(w, TAG_INT, name)?;
    w.write_all(&value.to_le_bytes())?;
    Ok(())
}

/// Write a named TAG_Long (LE i64).
pub fn write_long_tag(w: &mut impl Write, name: &str, value: i64) -> Result<()> {
    write_tag_header(w, TAG_LONG, name)?;
    w.write_all(&value.to_le_bytes())?;
    Ok(())
}

/// Write a named TAG_Float (LE f32).
pub fn write_float_tag(w: &mut impl Write, name: &str, value: f32) -> Result<()> {
    write_tag_header(w, TAG_FLOAT, name)?;
    w.write_all(&value.to_le_bytes())?;
    Ok(())
}

/// Write a named TAG_Byte (i8).
pub fn write_byte_tag(w: &mut impl Write, name: &str, value: i8) -> Result<()> {
    write_tag_header(w, TAG_BYTE, name)?;
    w.write_all(&[value as u8])?;
    Ok(())
}

/// Encode a sign block entity NBT blob for Bedrock Edition.
///
/// `text` is the sign front text (lines separated by `\n`).
/// Returns a complete NBT compound (little-endian) ready to be stored as a block entity.
pub fn encode_sign_block_entity(x: i32, y: i32, z: i32, text: &str) -> Vec<u8> {
    let mut buf: Vec<u8> = Vec::new();

    // Writing to a `Vec<u8>` is infallible; `.expect` documents this invariant.
    fn write_text_compound(buf: &mut Vec<u8>, name: &str, text: &str) {
        write_compound_start(buf, name).expect("Vec<u8> write is infallible");
        write_string_tag(buf, "Text", text).expect("Vec<u8> write is infallible");
        write_int_tag(buf, "SignTextColor", -16_777_216).expect("Vec<u8> write is infallible"); // 0xFF000000 black
        write_byte_tag(buf, "IgnoreLighting", 0).expect("Vec<u8> write is infallible");
        write_byte_tag(buf, "HideGlowOutline", 0).expect("Vec<u8> write is infallible");
        write_byte_tag(buf, "PersistFormatting", 1).expect("Vec<u8> write is infallible");
        write_string_tag(buf, "TextOwner", "").expect("Vec<u8> write is infallible");
        write_end(buf).expect("Vec<u8> write is infallible");
    }

    write_compound_start(&mut buf, "").expect("Vec<u8> write is infallible");
    write_string_tag(&mut buf, "id", "Sign").expect("Vec<u8> write is infallible");
    write_int_tag(&mut buf, "x", x).expect("Vec<u8> write is infallible");
    write_int_tag(&mut buf, "y", y).expect("Vec<u8> write is infallible");
    write_int_tag(&mut buf, "z", z).expect("Vec<u8> write is infallible");
    write_byte_tag(&mut buf, "isMovable", 1).expect("Vec<u8> write is infallible");
    write_text_compound(&mut buf, "FrontText", text);
    write_text_compound(&mut buf, "BackText", "");
    write_byte_tag(&mut buf, "IsWaxed", 0).expect("Vec<u8> write is infallible");
    write_end(&mut buf).expect("Vec<u8> write is infallible");

    buf
}

// ── Little-endian NBT reader ───────────────────────────────────────────────
//
// The read counterpart to the writer primitives above. It covers exactly the
// tag subset this crate emits for Bedrock (`byte`, `int`, `long`, `float`,
// `string`, `compound`) and is used by the round-trip tests to prove the
// writers are self-consistent — serialize a structure, parse it back, and
// compare the values.

/// A little-endian NBT value, mirroring the tag subset written by this module.
#[derive(Debug, Clone, PartialEq)]
pub enum NbtValue {
    Byte(i8),
    Int(i32),
    Long(i64),
    Float(f32),
    String(String),
    /// Insertion-ordered name/value pairs (duplicates are preserved).
    Compound(Vec<(String, NbtValue)>),
}

impl NbtValue {
    /// Look up the first field named `name` in a compound.
    ///
    /// Returns `None` for non-compounds or when the name is absent.
    pub fn get(&self, name: &str) -> Option<&NbtValue> {
        match self {
            NbtValue::Compound(fields) => fields.iter().find_map(|(k, v)| (k == name).then_some(v)),
            _ => None,
        }
    }
}

/// Read exactly `n` bytes from the cursor, advancing it past them.
fn read_bytes<'a>(r: &mut &'a [u8], n: usize, what: &str) -> Result<&'a [u8]> {
    if r.len() < n {
        bail!(
            "unexpected end of NBT input reading {what}: needed {n} bytes, have {}",
            r.len()
        );
    }
    let (head, tail) = std::mem::take(r).split_at(n);
    *r = tail;
    Ok(head)
}

fn read_u8(r: &mut &[u8], what: &str) -> Result<u8> {
    Ok(read_bytes(r, 1, what)?[0])
}

fn read_u16_le(r: &mut &[u8], what: &str) -> Result<u16> {
    let b = read_bytes(r, 2, what)?;
    Ok(u16::from_le_bytes([b[0], b[1]]))
}

fn read_i32_le(r: &mut &[u8]) -> Result<i32> {
    let b = read_bytes(r, 4, "i32")?;
    Ok(i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn read_i64_le(r: &mut &[u8]) -> Result<i64> {
    let b = read_bytes(r, 8, "i64")?;
    Ok(i64::from_le_bytes([
        b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
    ]))
}

fn read_f32_le(r: &mut &[u8]) -> Result<f32> {
    let b = read_bytes(r, 4, "f32")?;
    Ok(f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn read_string(r: &mut &[u8]) -> Result<String> {
    let len = read_u16_le(r, "string length")? as usize;
    let bytes = read_bytes(r, len, "string bytes")?;
    std::str::from_utf8(bytes)
        .context("NBT string is not valid UTF-8")
        .map(String::from)
}

/// Read the payload of a tag of the given type (no header).
fn read_payload(r: &mut &[u8], tag_type: u8) -> Result<NbtValue> {
    Ok(match tag_type {
        TAG_BYTE => NbtValue::Byte(read_u8(r, "byte payload")? as i8),
        TAG_INT => NbtValue::Int(read_i32_le(r)?),
        TAG_LONG => NbtValue::Long(read_i64_le(r)?),
        TAG_FLOAT => NbtValue::Float(read_f32_le(r)?),
        TAG_STRING => NbtValue::String(read_string(r)?),
        TAG_COMPOUND => NbtValue::Compound(read_compound_body(r)?),
        TAG_END => bail!("unexpected TAG_END where a payload was expected"),
        other => bail!("unsupported little-endian NBT tag type {other}"),
    })
}

/// Read compound fields until TAG_END; each field is `[tag_type][name][payload]`.
fn read_compound_body(r: &mut &[u8]) -> Result<Vec<(String, NbtValue)>> {
    let mut fields = Vec::new();
    loop {
        let tag_type = read_u8(r, "compound field tag type")?;
        if tag_type == TAG_END {
            break;
        }
        let name = read_string(r)?;
        let value = read_payload(r, tag_type)?;
        fields.push((name, value));
    }
    Ok(fields)
}

/// Parse a complete little-endian NBT document from `bytes`.
///
/// Expects the standard Bedrock root form: a single named tag (conventionally
/// a compound with an empty name). The root name is discarded and any trailing
/// bytes are ignored, so this can pull one entry out of a concatenated stream
/// such as SubChunk palette entries written back-to-back.
pub fn parse_nbt(bytes: &[u8]) -> Result<NbtValue> {
    let mut r = bytes;
    let tag_type = read_u8(&mut r, "root tag type")?;
    ensure!(tag_type != TAG_END, "NBT root tag cannot be TAG_END");
    let _root_name = read_string(&mut r)?;
    read_payload(&mut r, tag_type)
}

// ── Tests ─────────────────────────────────────────────────────────────────
//
// The little-endian NBT writer is the on-disk contract for every Bedrock
// SubChunk palette entry and every level.dat field. These tests pin the
// byte-level shape of each primitive so a regression here is caught before
// it can corrupt a world on disk.

#[cfg(test)]
mod tests {
    use super::*;

    // ── Primitive writers ────────────────────────────────────────────────

    #[test]
    fn write_string_payload_emits_le_u16_length_prefix_then_bytes() {
        let mut buf: Vec<u8> = Vec::new();
        write_string_payload(&mut buf, "ab").unwrap();
        // Length 2 as little-endian u16, then the UTF-8 bytes.
        assert_eq!(buf, vec![0x02, 0x00, b'a', b'b']);
    }

    #[test]
    fn write_string_payload_empty_is_just_zero_length() {
        let mut buf: Vec<u8> = Vec::new();
        write_string_payload(&mut buf, "").unwrap();
        assert_eq!(buf, vec![0x00, 0x00]);
    }

    #[test]
    fn write_string_tag_emits_header_plus_payload() {
        let mut buf: Vec<u8> = Vec::new();
        write_string_tag(&mut buf, "id", "Sign").unwrap();
        // [TAG_STRING=8]
        //   [name_len_le=2][b'i', b'd']
        //   [value_len_le=4][b'S', b'i', b'g', b'n']
        assert_eq!(
            buf,
            vec![
                TAG_STRING, // 8
                0x02, 0x00, b'i', b'd', // name "id"
                0x04, 0x00, b'S', b'i', b'g', b'n', // value "Sign"
            ],
        );
    }

    #[test]
    fn write_int_tag_emits_le_i32_payload() {
        let mut buf: Vec<u8> = Vec::new();
        write_int_tag(&mut buf, "x", 0x1234_5678).unwrap();
        // Header: [TAG_INT=3][len=1][b'x']; payload: little-endian i32.
        let mut expected = vec![TAG_INT, 0x01, 0x00, b'x'];
        expected.extend_from_slice(&0x1234_5678i32.to_le_bytes());
        assert_eq!(buf, expected);
    }

    #[test]
    fn write_int_tag_negative_two_complement() {
        let mut buf: Vec<u8> = Vec::new();
        write_int_tag(&mut buf, "n", -1).unwrap();
        // The last 4 bytes must be 0xFF 0xFF 0xFF 0xFF.
        assert_eq!(&buf[buf.len() - 4..], &[0xFF, 0xFF, 0xFF, 0xFF]);
    }

    #[test]
    fn write_long_tag_emits_le_i64_payload() {
        let mut buf: Vec<u8> = Vec::new();
        write_long_tag(&mut buf, "Time", 6000).unwrap();
        assert_eq!(&buf[buf.len() - 8..], &6000i64.to_le_bytes());
    }

    #[test]
    fn write_byte_tag_emits_single_u8_payload() {
        let mut buf: Vec<u8> = Vec::new();
        write_byte_tag(&mut buf, "flag", 1).unwrap();
        // Header is [TAG_BYTE][name_len_le=4][f,l,a,g] (7 bytes) + exactly one payload byte.
        assert_eq!(buf.last(), Some(&1u8));
        assert_eq!(buf.len(), 7 + 1);
    }

    #[test]
    fn write_float_tag_emits_le_f32_payload() {
        let mut buf: Vec<u8> = Vec::new();
        write_float_tag(&mut buf, "rainLevel", 0.0).unwrap();
        assert_eq!(&buf[buf.len() - 4..], &0.0f32.to_le_bytes());
    }

    #[test]
    fn write_compound_start_and_end_balance() {
        let mut buf: Vec<u8> = Vec::new();
        write_compound_start(&mut buf, "").unwrap();
        write_end(&mut buf).unwrap();
        // Empty-named compound: [TAG_COMPOUND][len=0][TAG_END]
        assert_eq!(buf, vec![TAG_COMPOUND, 0x00, 0x00, TAG_END]);
    }

    // ── encode_sign_block_entity: structural invariants ──────────────────
    //
    // We can't decode the blob here (no little-endian NBT reader in this
    // crate), so we assert the *structure* the encoders rely on: opening
    // byte, mandatory fields, coordinate round-trip, and terminating TAG_End.

    #[test]
    fn sign_block_entity_starts_with_compound_and_ends_with_tag_end() {
        let blob = encode_sign_block_entity(10, 64, -20, "Main St");
        assert_eq!(blob[0], TAG_COMPOUND, "must open with TAG_Compound");
        assert_eq!(*blob.last().unwrap(), TAG_END, "must close with TAG_End");
    }

    #[test]
    fn sign_block_entity_contains_id_sign_and_xyz_fields() {
        let blob = encode_sign_block_entity(10, 64, -20, "Main St");
        // The literal "Sign" id, the field names, and the text payload must
        // all appear as UTF-8 substrings somewhere in the blob.
        assert_contains(&blob, b"Sign");
        assert_contains(&blob, b"id");
        assert_contains(&blob, b"x");
        assert_contains(&blob, b"y");
        assert_contains(&blob, b"z");
        assert_contains(&blob, b"FrontText");
        assert_contains(&blob, b"BackText");
        assert_contains(&blob, b"Main St");
    }

    #[test]
    fn sign_block_entity_embeds_xyz_int_payloads() {
        let blob = encode_sign_block_entity(10, 64, -20, "");
        // The little-endian i32 payloads for x/y/z must appear in the blob.
        assert_contains(&blob, &10i32.to_le_bytes());
        assert_contains(&blob, &64i32.to_le_bytes());
        assert_contains(&blob, &(-20i32).to_le_bytes());
    }

    #[test]
    fn sign_block_entity_text_round_trips_through_payload_substring() {
        let text = "line one\nline two";
        let blob = encode_sign_block_entity(0, 0, 0, text);
        // The full text must appear verbatim (length-prefix validated by
        // surrounding structure but the bytes are intact).
        assert_contains(&blob, text.as_bytes());
    }

    #[test]
    fn sign_block_entity_is_deterministic() {
        // Same inputs → identical bytes. Non-determinism here would break
        // any downstream hashing / deduplication.
        let a = encode_sign_block_entity(1, 2, 3, "hello");
        let b = encode_sign_block_entity(1, 2, 3, "hello");
        assert_eq!(a, b);
    }

    // ── Round-trip through the little-endian NBT reader ──────────────────
    //
    // The byte-level tests above pin what each writer emits; these prove the
    // reader inverts the writers, so serialize → deserialize → compare catches
    // any drift between them (endianness, length prefix, nesting, UTF-8).

    #[test]
    fn parse_nbt_round_trips_every_primitive_tag() {
        let mut buf: Vec<u8> = Vec::new();
        write_compound_start(&mut buf, "root").unwrap();
        write_byte_tag(&mut buf, "b", -1).unwrap();
        write_int_tag(&mut buf, "i", 0x1234_5678).unwrap();
        write_long_tag(&mut buf, "l", -9_000_000_000).unwrap();
        write_float_tag(&mut buf, "f", 1.5).unwrap();
        write_string_tag(&mut buf, "s", "héllo").unwrap();
        write_end(&mut buf).unwrap();

        let root = parse_nbt(&buf).expect("compound must parse");
        assert_eq!(root.get("b"), Some(&NbtValue::Byte(-1)));
        assert_eq!(root.get("i"), Some(&NbtValue::Int(0x1234_5678)));
        assert_eq!(root.get("l"), Some(&NbtValue::Long(-9_000_000_000)));
        assert_eq!(root.get("f"), Some(&NbtValue::Float(1.5)));
        assert_eq!(root.get("s"), Some(&NbtValue::String("héllo".into())));
    }

    #[test]
    fn parse_nbt_round_trips_arbitrarily_nested_compounds() {
        let mut buf: Vec<u8> = Vec::new();
        write_compound_start(&mut buf, "").unwrap();
        write_compound_start(&mut buf, "outer").unwrap();
        write_compound_start(&mut buf, "inner").unwrap();
        write_int_tag(&mut buf, "depth", 2).unwrap();
        write_end(&mut buf).unwrap(); // inner
        write_end(&mut buf).unwrap(); // outer
        write_end(&mut buf).unwrap(); // root

        let root = parse_nbt(&buf).expect("nested compounds must parse");
        assert_eq!(
            root.get("outer")
                .unwrap()
                .get("inner")
                .unwrap()
                .get("depth"),
            Some(&NbtValue::Int(2)),
        );
    }

    #[test]
    fn parse_nbt_round_trips_empty_string_field() {
        let mut buf: Vec<u8> = Vec::new();
        write_compound_start(&mut buf, "").unwrap();
        write_string_tag(&mut buf, "empty", "").unwrap();
        write_end(&mut buf).unwrap();

        let root = parse_nbt(&buf).expect("must parse");
        assert_eq!(root.get("empty"), Some(&NbtValue::String(String::new())));
    }

    #[test]
    fn sign_block_entity_round_trips_through_reader() {
        let blob = encode_sign_block_entity(10, 64, -20, "Main St\nAve");
        let root = parse_nbt(&blob).expect("sign blob must parse");

        assert_eq!(root.get("id"), Some(&NbtValue::String("Sign".into())));
        assert_eq!(root.get("x"), Some(&NbtValue::Int(10)));
        assert_eq!(root.get("y"), Some(&NbtValue::Int(64)));
        assert_eq!(root.get("z"), Some(&NbtValue::Int(-20)));
        assert_eq!(root.get("isMovable"), Some(&NbtValue::Byte(1)));
        assert_eq!(root.get("IsWaxed"), Some(&NbtValue::Byte(0)));

        let front = root.get("FrontText").expect("FrontText compound present");
        assert_eq!(
            front.get("Text"),
            Some(&NbtValue::String("Main St\nAve".into()))
        );
        assert_eq!(
            front.get("SignTextColor"),
            Some(&NbtValue::Int(-16_777_216))
        );
        assert_eq!(front.get("IgnoreLighting"), Some(&NbtValue::Byte(0)));
        assert_eq!(front.get("HideGlowOutline"), Some(&NbtValue::Byte(0)));
        assert_eq!(front.get("PersistFormatting"), Some(&NbtValue::Byte(1)));
        assert_eq!(
            front.get("TextOwner"),
            Some(&NbtValue::String(String::new()))
        );

        let back = root.get("BackText").expect("BackText compound present");
        assert_eq!(back.get("Text"), Some(&NbtValue::String(String::new())));
    }

    #[test]
    fn parse_nbt_errors_on_empty_input() {
        assert!(parse_nbt(&[]).is_err());
    }

    #[test]
    fn parse_nbt_errors_when_compound_is_not_terminated() {
        // Root compound + one int field, but no closing TAG_END: the reader
        // must hit EOF while looking for the next field's tag type.
        let mut buf: Vec<u8> = Vec::new();
        write_compound_start(&mut buf, "").unwrap();
        write_int_tag(&mut buf, "x", 1).unwrap();
        // intentionally no write_end()
        assert!(parse_nbt(&buf).is_err());
    }

    #[test]
    fn parse_nbt_errors_on_unsupported_tag_type() {
        // Root tag type 9 (TAG_LIST) — never emitted by the LE writer.
        let buf = [9u8, 0x00, 0x00]; // type=9, empty root name
        let err = parse_nbt(&buf).unwrap_err().to_string();
        assert!(
            err.contains("unsupported"),
            "unsupported tag type should be rejected, got: {err}"
        );
    }

    // ── Helpers ──────────────────────────────────────────────────────────

    /// Panic with a helpful message if `needle` is not a contiguous substring
    /// of `haystack`.
    fn assert_contains(haystack: &[u8], needle: &[u8]) {
        if !haystack.windows(needle.len().max(1)).any(|w| w == needle) {
            panic!(
                "needle {:?} not found in blob ({} bytes):\n{:?}",
                needle,
                haystack.len(),
                haystack,
            );
        }
    }
}
