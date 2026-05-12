//! Minimal big-endian NBT writer for Java Edition.
//!
//! Java Edition uses big-endian NBT (vs Bedrock's little-endian).
//! Includes additional tag types needed by the Anvil chunk format
//! (TAG_SHORT, TAG_LIST, TAG_INT_ARRAY, TAG_LONG_ARRAY).

use anyhow::Result;
use std::io::Write;

// NBT tag type IDs
pub const TAG_END: u8 = 0;
pub const TAG_BYTE: u8 = 1;
pub const TAG_SHORT: u8 = 2;
pub const TAG_INT: u8 = 3;
pub const TAG_LONG: u8 = 4;
pub const TAG_FLOAT: u8 = 5;
pub const TAG_DOUBLE: u8 = 6;
pub const TAG_STRING: u8 = 8;
pub const TAG_LIST: u8 = 9;
pub const TAG_COMPOUND: u8 = 10;
pub const TAG_INT_ARRAY: u8 = 11;
pub const TAG_LONG_ARRAY: u8 = 12;

/// Write a string payload (length-prefixed, BE).
pub fn write_string_payload(w: &mut impl Write, s: &str) -> Result<()> {
    w.write_all(&(s.len() as u16).to_be_bytes())?;
    w.write_all(s.as_bytes())?;
    Ok(())
}

/// Write a named tag header: [tag_type][name_len_BE][name_bytes].
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

/// Write a named TAG_Int (BE i32).
pub fn write_int_tag(w: &mut impl Write, name: &str, value: i32) -> Result<()> {
    write_tag_header(w, TAG_INT, name)?;
    w.write_all(&value.to_be_bytes())?;
    Ok(())
}

/// Write a named TAG_Long (BE i64).
pub fn write_long_tag(w: &mut impl Write, name: &str, value: i64) -> Result<()> {
    write_tag_header(w, TAG_LONG, name)?;
    w.write_all(&value.to_be_bytes())?;
    Ok(())
}

/// Write a named TAG_Float (BE f32).
pub fn write_float_tag(w: &mut impl Write, name: &str, value: f32) -> Result<()> {
    write_tag_header(w, TAG_FLOAT, name)?;
    w.write_all(&value.to_be_bytes())?;
    Ok(())
}

/// Write a named TAG_Byte (i8).
pub fn write_byte_tag(w: &mut impl Write, name: &str, value: i8) -> Result<()> {
    write_tag_header(w, TAG_BYTE, name)?;
    w.write_all(&[value as u8])?;
    Ok(())
}

/// Write a named TAG_Short (BE i16).
pub fn write_short_tag(w: &mut impl Write, name: &str, value: i16) -> Result<()> {
    write_tag_header(w, TAG_SHORT, name)?;
    w.write_all(&value.to_be_bytes())?;
    Ok(())
}

/// Write a named TAG_Double (BE f64).
pub fn write_double_tag(w: &mut impl Write, name: &str, value: f64) -> Result<()> {
    write_tag_header(w, TAG_DOUBLE, name)?;
    w.write_all(&value.to_be_bytes())?;
    Ok(())
}

/// Write a TAG_List header: [tag header][item_type byte][length BE i32].
///
/// The caller must then write `length` payloads of type `item_type` (no per-element headers).
pub fn write_list_start(w: &mut impl Write, name: &str, item_type: u8, length: i32) -> Result<()> {
    write_tag_header(w, TAG_LIST, name)?;
    w.write_all(&[item_type])?;
    w.write_all(&length.to_be_bytes())?;
    Ok(())
}

/// Write a named TAG_Int_Array: [tag header][count BE i32][values BE].
pub fn write_int_array_tag(w: &mut impl Write, name: &str, values: &[i32]) -> Result<()> {
    write_tag_header(w, TAG_INT_ARRAY, name)?;
    w.write_all(&(values.len() as i32).to_be_bytes())?;
    for &v in values {
        w.write_all(&v.to_be_bytes())?;
    }
    Ok(())
}

/// Write a named TAG_Long_Array: [tag header][count BE i32][values BE].
pub fn write_long_array_tag(w: &mut impl Write, name: &str, values: &[i64]) -> Result<()> {
    write_tag_header(w, TAG_LONG_ARRAY, name)?;
    w.write_all(&(values.len() as i32).to_be_bytes())?;
    for &v in values {
        w.write_all(&v.to_be_bytes())?;
    }
    Ok(())
}

/// Encode a sign block entity NBT blob for Java Edition.
///
/// `text` is the sign front text (lines separated by `\n`).
/// Lines are split into max 4; missing lines are left empty.
/// Returns a complete big-endian NBT compound.
pub fn encode_java_sign_entity(x: i32, y: i32, z: i32, text: &str) -> Vec<u8> {
    let mut buf: Vec<u8> = Vec::new();

    fn write_text_face(buf: &mut Vec<u8>, name: &str, lines: [&str; 4]) {
        write_compound_start(buf, name).expect("Vec<u8> write is infallible");
        write_list_start(buf, "messages", TAG_STRING, 4).expect("Vec<u8> write is infallible");
        for line in &lines {
            let json = format!("{{\"text\":\"{}\"}}", line);
            write_string_payload(buf, &json).expect("Vec<u8> write is infallible");
        }
        write_byte_tag(buf, "has_glowing_text", 0).expect("Vec<u8> write is infallible");
        write_int_tag(buf, "color", -16_777_216).expect("Vec<u8> write is infallible"); // 0xFF000000 black
        write_end(buf).expect("Vec<u8> write is infallible");
    }

    let raw_lines: Vec<&str> = text.split('\n').collect();
    let mut lines = ["", "", "", ""];
    for (i, &line) in raw_lines.iter().take(4).enumerate() {
        lines[i] = line;
    }

    write_compound_start(&mut buf, "").expect("Vec<u8> write is infallible");
    write_string_tag(&mut buf, "id", "minecraft:sign").expect("Vec<u8> write is infallible");
    write_int_tag(&mut buf, "x", x).expect("Vec<u8> write is infallible");
    write_int_tag(&mut buf, "y", y).expect("Vec<u8> write is infallible");
    write_int_tag(&mut buf, "z", z).expect("Vec<u8> write is infallible");
    write_text_face(&mut buf, "front_text", lines);
    write_text_face(&mut buf, "back_text", ["", "", "", ""]);
    write_byte_tag(&mut buf, "is_waxed", 0).expect("Vec<u8> write is infallible");
    write_end(&mut buf).expect("Vec<u8> write is infallible");

    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn be_string_tag_bytes() {
        let mut buf = Vec::new();
        write_string_tag(&mut buf, "hi", "world").unwrap();
        assert_eq!(&buf[0..1], &[TAG_STRING]);
        assert_eq!(&buf[1..3], &[0, 2]); // name length BE
        assert_eq!(&buf[3..5], b"hi");
        assert_eq!(&buf[5..7], &[0, 5]); // string length BE
        assert_eq!(&buf[7..12], b"world");
    }

    #[test]
    fn be_int_tag_bytes() {
        let mut buf = Vec::new();
        write_int_tag(&mut buf, "x", 42).unwrap();
        let val_bytes = &buf[buf.len() - 4..];
        assert_eq!(val_bytes, &42i32.to_be_bytes());
    }

    #[test]
    fn be_long_array_tag() {
        let mut buf = Vec::new();
        write_long_array_tag(&mut buf, "data", &[1i64, 2, 3]).unwrap();
        assert_eq!(buf[0], TAG_LONG_ARRAY);
    }

    #[test]
    fn java_sign_entity_has_correct_id() {
        let nbt = encode_java_sign_entity(10, 64, 20, "Hello\nWorld");
        assert!(
            nbt.windows(b"minecraft:sign".len())
                .any(|w| w == b"minecraft:sign")
        );
    }

    #[test]
    fn be_short_tag_bytes() {
        let mut buf = Vec::new();
        write_short_tag(&mut buf, "s", 1000).unwrap();
        let val_bytes = &buf[buf.len() - 2..];
        assert_eq!(val_bytes, &1000i16.to_be_bytes());
    }

    #[test]
    fn be_double_tag_bytes() {
        let mut buf = Vec::new();
        write_double_tag(&mut buf, "d", 1.2345).unwrap();
        let val_bytes = &buf[buf.len() - 8..];
        assert_eq!(val_bytes, &1.2345f64.to_be_bytes());
    }

    #[test]
    fn be_int_array_tag_bytes() {
        let mut buf = Vec::new();
        write_int_array_tag(&mut buf, "arr", &[10, 20, 30]).unwrap();
        assert_eq!(buf[0], TAG_INT_ARRAY);
        // After tag header, check the count (BE i32)
        let name_len = u16::from_be_bytes([buf[1], buf[2]]) as usize;
        let count_offset = 3 + name_len;
        let count = i32::from_be_bytes([
            buf[count_offset],
            buf[count_offset + 1],
            buf[count_offset + 2],
            buf[count_offset + 3],
        ]);
        assert_eq!(count, 3);
    }

    #[test]
    fn be_list_start_bytes() {
        let mut buf = Vec::new();
        write_list_start(&mut buf, "items", TAG_INT, 5).unwrap();
        assert_eq!(buf[0], TAG_LIST);
        // After tag header, item_type byte then count BE i32
        let name_len = u16::from_be_bytes([buf[1], buf[2]]) as usize;
        let item_type_offset = 3 + name_len;
        assert_eq!(buf[item_type_offset], TAG_INT);
        let count = i32::from_be_bytes([
            buf[item_type_offset + 1],
            buf[item_type_offset + 2],
            buf[item_type_offset + 3],
            buf[item_type_offset + 4],
        ]);
        assert_eq!(count, 5);
    }

    #[test]
    fn be_long_tag_bytes() {
        let mut buf = Vec::new();
        write_long_tag(&mut buf, "l", 123_456_789i64).unwrap();
        let val_bytes = &buf[buf.len() - 8..];
        assert_eq!(val_bytes, &123_456_789i64.to_be_bytes());
    }

    #[test]
    fn be_float_tag_bytes() {
        let mut buf = Vec::new();
        write_float_tag(&mut buf, "f", 1.5f32).unwrap();
        let val_bytes = &buf[buf.len() - 4..];
        assert_eq!(val_bytes, &1.5f32.to_be_bytes());
    }

    #[test]
    fn java_sign_entity_four_lines() {
        let nbt = encode_java_sign_entity(0, 0, 0, "A\nB\nC\nD");
        // Should contain all four lines as JSON text components
        assert!(
            nbt.windows(b"\"text\":\"A\"".len())
                .any(|w| w == b"\"text\":\"A\"")
        );
        assert!(
            nbt.windows(b"\"text\":\"D\"".len())
                .any(|w| w == b"\"text\":\"D\"")
        );
    }

    #[test]
    fn java_sign_entity_fewer_than_four_lines() {
        let nbt = encode_java_sign_entity(0, 0, 0, "Only");
        // Lines 2-4 should be empty text components
        let empty_json = br#"{"text":""}"#;
        // Count occurrences — should be at least 3 (back_text has 4, front_text has 3 empty)
        let count = nbt
            .windows(empty_json.len())
            .filter(|w| *w == empty_json)
            .count();
        assert!(
            count >= 7,
            "Expected at least 7 empty text components (3 front + 4 back), got {count}"
        );
    }
}
