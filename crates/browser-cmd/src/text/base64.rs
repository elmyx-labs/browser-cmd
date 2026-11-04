//! Standard base64, for proxy credentials and the bytes CDP returns.

const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub(crate) fn base64(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let block = chunk.iter().fold(0_u32, |block, byte| (block << 8) | u32::from(*byte));
        let block = block << (8 * (3 - chunk.len()));
        for index in 0..4 {
            text.push(if index <= chunk.len() {
                ALPHABET[(block >> (18 - 6 * index)) as usize & 0x3f] as char
            } else {
                '='
            });
        }
    }
    text
}

/// The inverse of [`base64`], lenient about padding and line breaks. `None`
/// for a character outside the alphabet.
pub(crate) fn unbase64(text: &str) -> Option<Vec<u8>> {
    let value = |byte: u8| match byte {
        b'A'..=b'Z' => Some(byte - b'A'),
        b'a'..=b'z' => Some(byte - b'a' + 26),
        b'0'..=b'9' => Some(byte - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    };
    let mut bytes = Vec::with_capacity(text.len() / 4 * 3);
    let (mut block, mut bits) = (0_u32, 0);
    for byte in text.bytes().filter(|byte| !byte.is_ascii_whitespace() && *byte != b'=') {
        block = (block << 6) | u32::from(value(byte)?);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            bytes.push((block >> bits) as u8);
            block &= (1 << bits) - 1;
        }
    }
    Some(bytes)
}
