use sha2::{Digest, Sha256};

pub fn normalize_email(input: &str) -> String {
    input.trim().to_lowercase()
}

pub fn sha256_hex(input: &str) -> String {
    format!("{:x}", Sha256::digest(input.as_bytes()))
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
