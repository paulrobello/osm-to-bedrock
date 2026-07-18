//! Minimal little-endian NBT writer for Bedrock Edition.
//!
//! Bedrock uses little-endian NBT (vs Java's big-endian).
//! We only implement the subset needed for SubChunk palettes and level.dat.

use anyhow::Result;
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
