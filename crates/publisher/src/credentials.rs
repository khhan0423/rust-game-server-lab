//! OS-backed credential generation, wire encoding, and API-key comparison.

use rand::TryRng;

pub(super) fn random_bytes() -> Result<[u8; 32], rand::rngs::SysError> {
    let mut bytes = [0; 32];
    rand::rngs::SysRng.try_fill_bytes(&mut bytes)?;
    Ok(bytes)
}

pub(super) fn encode(bytes: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(64);
    for byte in bytes {
        text.push(char::from(HEX[usize::from(byte >> 4)]));
        text.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    text
}

pub(super) fn decode(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 {
        return None;
    }
    let mut bytes = [0; 32];
    let (pairs, _) = text.as_bytes().as_chunks::<2>();
    for (byte, &[high, low]) in bytes.iter_mut().zip(pairs) {
        let high = u8::try_from(char::from(high).to_digit(16)?).ok()?;
        let low = u8::try_from(char::from(low).to_digit(16)?).ok()?;
        *byte = (high << 4) | low;
    }
    Some(bytes)
}

pub(super) fn key_matches(expected: &[u8], supplied: &[u8]) -> bool {
    // Timing depends on the supplied length, not on which bytes match.
    let mut diff = expected.len() ^ supplied.len();
    for (index, byte) in supplied.iter().enumerate() {
        let other = expected.get(index).copied().unwrap_or_default();
        diff |= usize::from(std::hint::black_box(byte ^ other));
    }
    diff == 0
}
