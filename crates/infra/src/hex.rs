//! Fixed-width hexadecimal credentials shared by HTTP tools and servers.

/// Random bytes encoded as lower-case hexadecimal, with a caller-defined width.
#[derive(Debug)]
pub struct Hex;

impl Hex {
    /// Generates bytes using the operating system's random number source.
    pub fn random<const N: usize>() -> Result<[u8; N], rand::rngs::SysError> {
        use rand::TryRng;
        let mut bytes = [0; N];
        rand::rngs::SysRng.try_fill_bytes(&mut bytes)?;
        Ok(bytes)
    }

    /// Encodes one lower-case digit per nibble, retaining leading zeroes.
    pub fn encode(bytes: &[u8]) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut text = String::with_capacity(bytes.len().saturating_mul(2));
        for byte in bytes {
            text.push(char::from(HEX[usize::from(byte >> 4)]));
            text.push(char::from(HEX[usize::from(byte & 0xf)]));
        }
        text
    }

    /// Decodes exactly `N` bytes, accepting either case.
    pub fn decode<const N: usize>(text: &str) -> Option<[u8; N]> {
        let (pairs, remainder) = text.as_bytes().as_chunks::<2>();
        if pairs.len() != N || !remainder.is_empty() {
            return None;
        }
        let mut bytes = [0; N];
        for (byte, &[high, low]) in bytes.iter_mut().zip(pairs) {
            let high = u8::try_from(char::from(high).to_digit(16)?).ok()?;
            let low = u8::try_from(char::from(low).to_digit(16)?).ok()?;
            *byte = (high << 4) | low;
        }
        Some(bytes)
    }
}
