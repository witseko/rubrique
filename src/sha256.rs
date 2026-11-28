//! SHA-256 (FIPS 180-4), to check a download against the digest built into
//! the app.

/// The round constants from FIPS 180-4, section 4.2.2.
const ROUND_CONSTANTS: [u32; 64] = [
    0x428a_2f98,
    0x7137_4491,
    0xb5c0_fbcf,
    0xe9b5_dba5,
    0x3956_c25b,
    0x59f1_11f1,
    0x923f_82a4,
    0xab1c_5ed5,
    0xd807_aa98,
    0x1283_5b01,
    0x2431_85be,
    0x550c_7dc3,
    0x72be_5d74,
    0x80de_b1fe,
    0x9bdc_06a7,
    0xc19b_f174,
    0xe49b_69c1,
    0xefbe_4786,
    0x0fc1_9dc6,
    0x240c_a1cc,
    0x2de9_2c6f,
    0x4a74_84aa,
    0x5cb0_a9dc,
    0x76f9_88da,
    0x983e_5152,
    0xa831_c66d,
    0xb003_27c8,
    0xbf59_7fc7,
    0xc6e0_0bf3,
    0xd5a7_9147,
    0x06ca_6351,
    0x1429_2967,
    0x27b7_0a85,
    0x2e1b_2138,
    0x4d2c_6dfc,
    0x5338_0d13,
    0x650a_7354,
    0x766a_0abb,
    0x81c2_c92e,
    0x9272_2c85,
    0xa2bf_e8a1,
    0xa81a_664b,
    0xc24b_8b70,
    0xc76c_51a3,
    0xd192_e819,
    0xd699_0624,
    0xf40e_3585,
    0x106a_a070,
    0x19a4_c116,
    0x1e37_6c08,
    0x2748_774c,
    0x34b0_bcb5,
    0x391c_0cb3,
    0x4ed8_aa4a,
    0x5b9c_ca4f,
    0x682e_6ff3,
    0x748f_82ee,
    0x78a5_636f,
    0x84c8_7814,
    0x8cc7_0208,
    0x90be_fffa,
    0xa450_6ceb,
    0xbef9_a3f7,
    0xc671_78f2,
];

/// The initial hash value from FIPS 180-4, section 5.3.3.
const INITIAL_STATE: [u32; 8] =
    [0x6a09_e667, 0xbb67_ae85, 0x3c6e_f372, 0xa54f_f53a, 0x510e_527f, 0x9b05_688c, 0x1f83_d9ab, 0x5be0_cd19];

/// Bytes in one message block.
const BLOCK_BYTES: usize = 64;

/// A SHA-256 hash that takes its input in parts.
pub struct Sha256 {
    state: [u32; 8],
    block: [u8; BLOCK_BYTES],
    /// Bytes of `block` filled so far.
    block_len: usize,
    /// Bytes of input so far.
    total_len: u64,
}

impl Default for Sha256 {
    fn default() -> Self {
        Self::new()
    }
}

impl Sha256 {
    /// A hash of no input yet.
    pub fn new() -> Self {
        Self { state: INITIAL_STATE, block: [0; BLOCK_BYTES], block_len: 0, total_len: 0 }
    }

    /// Add `data` to the input.
    pub fn update(&mut self, mut data: &[u8]) {
        self.total_len = self.total_len.wrapping_add(data.len() as u64);
        while !data.is_empty() {
            let take = (BLOCK_BYTES - self.block_len).min(data.len());
            self.block[self.block_len..self.block_len + take].copy_from_slice(&data[..take]);
            self.block_len += take;
            data = &data[take..];
            if self.block_len == BLOCK_BYTES {
                compress(&mut self.state, &self.block);
                self.block_len = 0;
            }
        }
    }

    /// The 32-byte digest of all input.
    pub fn finalize(mut self) -> [u8; 32] {
        let bit_len = self.total_len.wrapping_mul(8);
        self.update(&[0x80]);
        while self.block_len != BLOCK_BYTES - 8 {
            self.update(&[0]);
        }
        self.update(&bit_len.to_be_bytes());
        let mut digest = [0; 32];
        for (bytes, word) in digest.as_chunks_mut::<4>().0.iter_mut().zip(self.state) {
            bytes.copy_from_slice(&word.to_be_bytes());
        }
        digest
    }
}

/// Mix one 64-byte block into the state (FIPS 180-4, section 6.2.2).
#[expect(clippy::many_single_char_names, reason = "the working variables a to h as FIPS 180-4 names them")]
fn compress(state: &mut [u32; 8], block: &[u8; BLOCK_BYTES]) {
    let mut schedule = [0u32; 64];
    for (word, bytes) in schedule.iter_mut().zip(block.as_chunks::<4>().0) {
        *word = u32::from_be_bytes(*bytes);
    }
    for index in 16..64 {
        let older = schedule[index - 15];
        let newer = schedule[index - 2];
        let sigma0 = older.rotate_right(7) ^ older.rotate_right(18) ^ (older >> 3);
        let sigma1 = newer.rotate_right(17) ^ newer.rotate_right(19) ^ (newer >> 10);
        schedule[index] =
            schedule[index - 16].wrapping_add(sigma0).wrapping_add(schedule[index - 7]).wrapping_add(sigma1);
    }
    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = *state;
    for (constant, word) in ROUND_CONSTANTS.iter().zip(schedule) {
        let sum1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let choice = (e & f) ^ (!e & g);
        let temp1 = h.wrapping_add(sum1).wrapping_add(choice).wrapping_add(*constant).wrapping_add(word);
        let sum0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let majority = (a & b) ^ (a & c) ^ (b & c);
        let temp2 = sum0.wrapping_add(majority);
        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(temp1);
        d = c;
        c = b;
        b = a;
        a = temp1.wrapping_add(temp2);
    }
    for (word, value) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
        *word = word.wrapping_add(value);
    }
}

#[cfg(test)]
mod tests {
    use std::fmt::Write;

    use super::*;

    fn to_hex(digest: [u8; 32]) -> String {
        digest.iter().fold(String::new(), |mut text, byte| {
            let _ = write!(text, "{byte:02x}");
            text
        })
    }

    fn hex(input: &[u8]) -> String {
        let mut hash = Sha256::new();
        hash.update(input);
        to_hex(hash.finalize())
    }

    #[test]
    fn matches_the_fips_180_examples() {
        assert_eq!(hex(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(
            hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    #[test]
    fn gives_the_same_digest_for_input_in_parts() {
        let mut hash = Sha256::new();
        for _ in 0..1000 {
            hash.update(&[b'a'; 1000]);
        }
        assert_eq!(to_hex(hash.finalize()), "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0");
    }
}
