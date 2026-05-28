use sha2::{Digest, Sha256};
use std::fmt::Write as _;

pub fn normalize_email(input: &str) -> String {
    input.trim().to_lowercase()
}

pub fn sha256_hex(input: &str) -> String {
    let digest = Sha256::digest(input.as_bytes());
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest.as_slice() {
        write!(&mut output, "{byte:02x}").expect("writing to a string should not fail");
    }
    output
}

#[cfg(test)]
mod tests {
    use super::{normalize_email, sha256_hex};

    #[test]
    fn normalize_email_trims_and_lowercases() {
        assert_eq!(normalize_email("  USER@Example.COM "), "user@example.com");
    }

    #[test]
    fn sha256_hex_is_deterministic() {
        assert_eq!(sha256_hex("abc"), sha256_hex("abc"));
        assert_ne!(sha256_hex("abc"), sha256_hex("abcd"));
    }
}
