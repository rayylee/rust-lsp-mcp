//! LSP positions use 0-based lines and UTF-16 code-unit columns.
//! This module converts them to byte offsets for editing file content as bytes.

pub fn byte_offset(text: &str, line: u32, character: u32) -> usize {
    let mut offset = 0;
    for (n, line_text) in text.split_inclusive('\n').enumerate() {
        if n as u32 == line {
            return offset + column_offset(line_text, character);
        }
        offset += line_text.len();
    }
    text.len()
}

fn column_offset(line: &str, character: u32) -> usize {
    let line = line.strip_suffix('\n').unwrap_or(line);
    let line = line.strip_suffix('\r').unwrap_or(line);

    let mut counted = 0u32;
    for (offset, ch) in line.char_indices() {
        if counted >= character {
            return offset;
        }
        let after = counted + ch.len_utf16() as u32;
        if after > character {
            return offset;
        }
        counted = after;
    }
    line.len()
}
