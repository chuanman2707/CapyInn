//! How a PIN becomes the value stored in `users.pin_hash`.
//!
//! This lived in three places — `login`, `create_user`, and the setup
//! provisioner — each with its own copy of the same four lines. They agreed,
//! but nothing made them agree: any drift would have provisioned users who
//! could never log in, and the symptom would have looked like a wrong PIN.

use sha2::{Digest, Sha256};

/// The stored form of a PIN. Callers compare hashes, never the PIN itself.
pub fn pin_hash(pin: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(pin.as_bytes());
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::pin_hash;

    #[test]
    fn the_hash_is_lowercase_hex_sha256() {
        let hash = pin_hash("1234");
        assert_eq!(
            hash, "03ac674216f3e15c761ee1a5e255f067953623c8b388b4459e13f978d7c846f4",
            "the stored format is the plain lowercase-hex SHA-256 of the PIN; \
             changing it locks every existing user out"
        );
        assert_eq!(hash.len(), 64);
    }

    #[test]
    fn different_pins_hash_differently_and_the_same_pin_is_stable() {
        assert_eq!(pin_hash("0000"), pin_hash("0000"));
        assert_ne!(pin_hash("0000"), pin_hash("0001"));
        assert_ne!(pin_hash("1234"), pin_hash("12345"));
    }

    #[test]
    fn an_empty_pin_still_hashes_rather_than_panicking() {
        assert_eq!(pin_hash("").len(), 64);
        assert_ne!(pin_hash(""), pin_hash("0"));
    }
}
