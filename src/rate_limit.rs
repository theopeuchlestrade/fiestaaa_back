use std::{
    collections::{HashMap, VecDeque},
    time::{Duration, Instant},
};

use tokio::sync::Mutex;

pub struct AuthRateLimiter {
    attempts: Mutex<HashMap<String, VecDeque<Instant>>>,
    last_compaction: Mutex<Instant>,
    max_attempts: usize,
    window: Duration,
}

impl AuthRateLimiter {
    pub fn new(max_attempts: usize, window: Duration) -> Self {
        Self {
            attempts: Mutex::new(HashMap::new()),
            last_compaction: Mutex::new(Instant::now()),
            max_attempts,
            window,
        }
    }

    fn prune_bucket(entry: &mut VecDeque<Instant>, now: Instant, window: Duration) {
        while let Some(oldest) = entry.front() {
            if now.duration_since(*oldest) > window {
                entry.pop_front();
            } else {
                break;
            }
        }
    }

    pub async fn allow(&self, key: &str) -> bool {
        let now = Instant::now();
        let should_compact = {
            let mut last_compaction = self.last_compaction.lock().await;
            if now.duration_since(*last_compaction) >= self.window {
                *last_compaction = now;
                true
            } else {
                false
            }
        };

        let mut attempts = self.attempts.lock().await;
        if should_compact {
            attempts.retain(|_, entry| {
                Self::prune_bucket(entry, now, self.window);
                !entry.is_empty()
            });
        }

        let entry = attempts.entry(key.to_string()).or_default();
        Self::prune_bucket(entry, now, self.window);

        if entry.len() >= self.max_attempts {
            return false;
        }

        entry.push_back(now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::AuthRateLimiter;
    use std::time::Duration;

    #[tokio::test]
    async fn allow_rejects_requests_over_the_limit() {
        let limiter = AuthRateLimiter::new(1, Duration::from_secs(60));

        assert!(limiter.allow("login:127.0.0.1").await);
        assert!(!limiter.allow("login:127.0.0.1").await);
    }

    #[tokio::test]
    async fn allow_compacts_expired_buckets() {
        let limiter = AuthRateLimiter::new(2, Duration::from_millis(5));

        assert!(limiter.allow("auth:first").await);
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(limiter.allow("auth:second").await);

        let attempts = limiter.attempts.lock().await;
        assert_eq!(attempts.len(), 1);
        assert!(attempts.contains_key("auth:second"));
        assert!(!attempts.contains_key("auth:first"));
    }
}
