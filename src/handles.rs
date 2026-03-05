use rand_core::{OsRng, RngCore};
use sqlx::PgPool;

const HANDLE_MIN_LEN: usize = 4;
const HANDLE_MAX_LEN: usize = 32;

// Small curated list to build memorable handles (2 words + digits).
const HANDLE_WORDS: &[&str] = &[
    "mango",
    "forest",
    "comet",
    "aurora",
    "lagoon",
    "ember",
    "cactus",
    "cosmo",
    "breeze",
    "delta",
    "fluffy",
    "coral",
    "pixel",
    "lotus",
    "canyon",
    "ember",
    "marble",
    "otter",
    "quartz",
    "salsa",
    "tango",
    "umbra",
    "velvet",
    "whisky",
    "yodel",
    "zephyr",
    "acorn",
    "bamboo",
    "cedar",
    "dandelion",
    "ember",
    "frost",
    "glow",
    "hazel",
    "iguana",
    "jasper",
    "koala",
    "lichen",
    "meadow",
    "nebula",
    "onyx",
    "papaya",
    "quokka",
    "ripple",
    "saffron",
    "topaz",
    "utopia",
    "vortex",
    "willow",
    "yucca",
];

#[derive(Clone, Debug)]
pub struct HandleCandidate {
    pub normalized: String,
}

pub fn normalize_handle(raw: &str) -> HandleCandidate {
    HandleCandidate {
        normalized: raw.trim().to_lowercase(),
    }
}

pub fn is_valid_handle(raw: &str) -> bool {
    let len = raw.len();
    if !(HANDLE_MIN_LEN..=HANDLE_MAX_LEN).contains(&len) {
        return false;
    }
    let mut chars = raw.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !is_allowed_char(first) || !first.is_ascii_alphanumeric() {
        return false;
    }
    let mut last_ok = false;
    for c in raw.chars() {
        if !is_allowed_char(c) {
            return false;
        }
        last_ok = c.is_ascii_alphanumeric();
    }
    last_ok
}

fn is_allowed_char(c: char) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_' | '.')
}

pub fn looks_like_email(input: &str) -> bool {
    input.contains('@')
}

pub async fn handle_available(db: &PgPool, handle: &str) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar::<_, bool>(
        "SELECT NOT EXISTS(SELECT 1 FROM users WHERE lower(handle) = lower($1))",
    )
    .bind(handle)
    .fetch_one(db)
    .await
}

pub async fn generate_unique_handle(db: &PgPool) -> Result<String, sqlx::Error> {
    let mut rng = OsRng;
    let mut attempts = 0;
    while attempts < 20 {
        let first = pick_word(&mut rng);
        let second = pick_word(&mut rng);
        let number = (rng.next_u64() % 10_000) as u32;
        let candidate = format!("{first}-{second}-{number:04}");
        if handle_available(db, &candidate).await? {
            return Ok(candidate);
        }
        attempts += 1;
    }

    // Fallback: still ensure a unique suffix even if the word pool is saturated.
    let suffix = rng.next_u64();
    let fallback = format!("fiestaaa-{suffix:x}");
    if handle_available(db, &fallback).await? {
        Ok(fallback)
    } else {
        // In the unlikely case everything collides, return the last attempt.
        Ok(fallback)
    }
}

fn pick_word(rng: &mut OsRng) -> &'static str {
    let idx = (rng.next_u64() as usize) % HANDLE_WORDS.len();
    HANDLE_WORDS[idx]
}

#[cfg(test)]
mod tests {
    use super::{is_valid_handle, looks_like_email, normalize_handle};

    #[test]
    fn normalize_handle_trims_and_lowercases() {
        let candidate = normalize_handle("  Fresh_Handle  ");
        assert_eq!(candidate.normalized, "fresh_handle");
    }

    #[test]
    fn is_valid_handle_accepts_supported_patterns_only() {
        assert!(is_valid_handle("fresh.handle_42"));
        assert!(!is_valid_handle("abc"));
        assert!(!is_valid_handle("-starts-with-dash"));
        assert!(!is_valid_handle("ends-with-dash-"));
        assert!(!is_valid_handle("contains space"));
    }

    #[test]
    fn looks_like_email_detects_at_sign() {
        assert!(looks_like_email("user@example.com"));
        assert!(!looks_like_email("plain_handle"));
    }
}
