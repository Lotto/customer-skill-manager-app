use sha2::{Digest, Sha256};

/// Compute the lowercase hex SHA-256 of a byte slice.
///
/// This is the canonical content hash the app uses to decide whether a skill
/// archive on the backend differs from what is installed locally.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vector() {
        // SHA-256 of the empty input.
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn abc_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn is_stable() {
        assert_eq!(sha256_hex(b"skill-payload"), sha256_hex(b"skill-payload"));
        assert_ne!(sha256_hex(b"a"), sha256_hex(b"b"));
    }
}
