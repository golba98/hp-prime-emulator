//! SHA-256 identities for firmware, storage and mapped-memory images.
//! Use the workspace's SHA-2 implementation so large RAM captures also use
//! its supported CPU acceleration. The existing digest format is unchanged.
use sha2::Digest;

#[derive(Default)]
pub struct Sha256(sha2::Sha256);
impl Sha256 {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn update(&mut self, data: &[u8]) {
        self.0.update(data);
    }
    pub fn finalize_hex(self) -> String {
        format!("{:x}", self.0.finalize())
    }
}
pub fn hex(data: &[u8]) -> String {
    format!("{:x}", sha2::Sha256::digest(data))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_vectors() {
        assert_eq!(
            hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let long = b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq";
        assert_eq!(
            hex(long),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    #[test]
    fn split_updates_match_one_shot() {
        let data: Vec<u8> = (0..1000u32).map(|i| (i * 7) as u8).collect();
        let mut h = Sha256::new();
        for part in data.chunks(37) {
            h.update(part);
        }
        assert_eq!(h.finalize_hex(), hex(&data));
    }
}
