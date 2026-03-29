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
