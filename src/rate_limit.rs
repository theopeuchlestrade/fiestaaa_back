use std::{
    collections::{HashMap, VecDeque},
    time::{Duration, Instant},
};

use redis::Client as RedisClient;
use tokio::sync::Mutex;

pub struct AuthRateLimiter {
    redis_client: Option<RedisClient>,
    attempts: Mutex<HashMap<String, VecDeque<Instant>>>,
    last_compaction: Mutex<Instant>,
    max_attempts: usize,
    window: Duration,
}

impl AuthRateLimiter {
    pub fn new(max_attempts: usize, window: Duration, redis_client: Option<RedisClient>) -> Self {
        Self {
            redis_client,
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

    async fn allow_local(&self, key: &str) -> bool {
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

    async fn allow_redis(&self, key: &str) -> Result<bool, redis::RedisError> {
        let Some(client) = self.redis_client.as_ref() else {
            return Ok(false);
        };

        let mut conn = client.get_multiplexed_async_connection().await?;
        let redis_key = format!("fiestaaa:rate_limit:{key}");
        let current_count: i64 = redis::cmd("INCR")
            .arg(&redis_key)
            .query_async(&mut conn)
            .await?;

        if current_count == 1 {
            let ttl = self.window.as_secs().max(1) as i64;
            let _: () = redis::cmd("EXPIRE")
                .arg(&redis_key)
                .arg(ttl)
                .query_async(&mut conn)
                .await?;
        }

        Ok(current_count <= self.max_attempts as i64)
    }

    pub async fn allow(&self, key: &str) -> bool {
        if self.redis_client.is_some()
            && let Ok(allowed) = self.allow_redis(key).await
        {
            return allowed;
        }

        self.allow_local(key).await
    }
}

#[cfg(test)]
mod tests {
    use super::AuthRateLimiter;
    use std::time::Duration;

    #[tokio::test]
    async fn allow_rejects_requests_over_the_limit() {
        let limiter = AuthRateLimiter::new(1, Duration::from_secs(60), None);

        assert!(limiter.allow("login:127.0.0.1").await);
        assert!(!limiter.allow("login:127.0.0.1").await);
    }

    #[tokio::test]
    async fn allow_compacts_expired_buckets() {
        let limiter = AuthRateLimiter::new(2, Duration::from_millis(5), None);

        assert!(limiter.allow("auth:first").await);
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(limiter.allow("auth:second").await);

        let attempts = limiter.attempts.lock().await;
        assert_eq!(attempts.len(), 1);
        assert!(attempts.contains_key("auth:second"));
        assert!(!attempts.contains_key("auth:first"));
    }
}
