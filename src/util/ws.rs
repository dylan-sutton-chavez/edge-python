use alloc::string::String;
use alloc::vec::Vec;

const MAGIC: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

// RFC 6455 accept, base64 of sha1 of the client key plus the magic guid.
pub fn accept_key(key: &str) -> String {
    let mut input = String::with_capacity(key.len() + MAGIC.len());
    input.push_str(key);
    input.push_str(MAGIC);
    base64_encode(&sha1(input.as_bytes()))
}

// A masked frame when mask is set (client), unmasked otherwise (server), FIN with the opcode.
pub fn encode_frame(opcode: u8, payload: &[u8], mask: Option<[u8; 4]>) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 14);
    out.push(0x80 | opcode);
    let flag = if mask.is_some() { 0x80 } else { 0 };
    let len = payload.len();
    if len < 126 {
        out.push(flag | len as u8);
    } else if len < 65536 {
        out.push(flag | 126);
        out.extend_from_slice(&(len as u16).to_be_bytes());
    } else {
        out.push(flag | 127);
        out.extend_from_slice(&(len as u64).to_be_bytes());
    }
    match mask {
        Some(m) => {
            out.extend_from_slice(&m);
            for (i, b) in payload.iter().enumerate() {
                out.push(b ^ m[i % 4]);
            }
        }
        None => out.extend_from_slice(payload),
    }
    out
}

// Reads one frame, returns opcode payload and bytes consumed, None when short.
pub fn parse_frame(buf: &[u8]) -> Option<(u8, Vec<u8>, usize)> {
    if buf.len() < 2 {
        return None;
    }
    let opcode = buf[0] & 0x0f;
    let masked = buf[1] & 0x80 != 0;
    let mut len = (buf[1] & 0x7f) as usize;
    let mut off = 2;
    if len == 126 {
        if buf.len() < off + 2 {
            return None;
        }
        len = u16::from_be_bytes([buf[off], buf[off + 1]]) as usize;
        off += 2;
    } else if len == 127 {
        if buf.len() < off + 8 {
            return None;
        }
        let mut n = [0u8; 8];
        n.copy_from_slice(&buf[off..off + 8]);
        len = u64::from_be_bytes(n) as usize;
        off += 8;
    }
    let mask = if masked {
        if buf.len() < off + 4 {
            return None;
        }
        let m = [buf[off], buf[off + 1], buf[off + 2], buf[off + 3]];
        off += 4;
        Some(m)
    } else {
        None
    };
    if buf.len() < off + len {
        return None;
    }
    let mut payload = buf[off..off + len].to_vec();
    if let Some(m) = mask {
        for (i, b) in payload.iter_mut().enumerate() {
            *b ^= m[i % 4];
        }
    }
    Some((opcode, payload, off + len))
}

// FIPS 180-4 SHA-1, 20-byte digest, padding shape shared with sha256.
pub fn sha1(input: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];

    let mut buf: Vec<u8> = Vec::with_capacity(input.len() + 72);
    buf.extend_from_slice(input);
    buf.push(0x80);
    while buf.len() % 64 != 56 {
        buf.push(0);
    }
    let bit_len = (input.len() as u64).wrapping_mul(8);
    buf.extend_from_slice(&bit_len.to_be_bytes());

    for block in buf.chunks_exact(64) {
        let mut w = [0u32; 80];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([block[i * 4], block[i * 4 + 1], block[i * 4 + 2], block[i * 4 + 3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, &word) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A827999),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6),
            };
            let temp = a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(word);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
    }

    let mut out = [0u8; 20];
    for i in 0..5 {
        out[i * 4..i * 4 + 4].copy_from_slice(&h[i].to_be_bytes());
    }
    out
}

// Standard base64 with padding.
pub fn base64_encode(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for group in data.chunks(3) {
        let b0 = group[0] as u32;
        let b1 = *group.get(1).unwrap_or(&0) as u32;
        let b2 = *group.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(T[(n >> 18) as usize & 0x3f] as char);
        out.push(T[(n >> 12) as usize & 0x3f] as char);
        out.push(if group.len() > 1 { T[(n >> 6) as usize & 0x3f] as char } else { '=' });
        out.push(if group.len() > 2 { T[n as usize & 0x3f] as char } else { '=' });
    }
    out
}

// Standard base64 decode, None on any character outside the alphabet.
pub fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let val = |c: u8| -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some((c - b'A') as u32),
            b'a'..=b'z' => Some((c - b'a' + 26) as u32),
            b'0'..=b'9' => Some((c - b'0' + 52) as u32),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    };
    let bytes: Vec<u8> = text.bytes().filter(|&c| c != b'=' && !c.is_ascii_whitespace()).collect();
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for group in bytes.chunks(4) {
        let mut n = 0u32;
        for (i, &c) in group.iter().enumerate() {
            n |= val(c)? << (18 - 6 * i);
        }
        out.push((n >> 16) as u8);
        if group.len() > 2 { out.push((n >> 8) as u8); }
        if group.len() > 3 { out.push(n as u8); }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::format;

    #[test]
    fn sha1_matches_the_canonical_vector() {
        let hex: String = sha1(b"abc").iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, "a9993e364706816aba3e25717850c26c9cd0d89d");
    }

    #[test]
    fn accept_key_matches_rfc_6455() {
        assert_eq!(accept_key("dGhlIHNhbXBsZSBub25jZQ=="), "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=");
    }

    #[test]
    fn base64_pads_partial_groups() {
        assert_eq!(base64_encode(b"M"), "TQ==");
        assert_eq!(base64_encode(b"Ma"), "TWE=");
        assert_eq!(base64_encode(b"Man"), "TWFu");
    }

    #[test]
    fn base64_decode_reverses_encode() {
        for case in [b"".as_slice(), b"M", b"Ma", b"Man", b"\x00\xff\x10binary\n"] {
            assert_eq!(base64_decode(&base64_encode(case)).unwrap(), case);
        }
        assert!(base64_decode("not base64 !@#").is_none());
    }

    #[test]
    fn a_masked_client_frame_parses_back() {
        let framed = encode_frame(0x1, b"hello", Some([1, 2, 3, 4]));
        let (opcode, payload, consumed) = parse_frame(&framed).unwrap();
        assert_eq!(opcode, 0x1);
        assert_eq!(payload, b"hello");
        assert_eq!(consumed, framed.len());
    }

    #[test]
    fn an_unmasked_server_frame_parses_back() {
        let framed = encode_frame(0x1, b"world", None);
        assert_eq!(framed[1] & 0x80, 0);
        let (_, payload, _) = parse_frame(&framed).unwrap();
        assert_eq!(payload, b"world");
    }
}
