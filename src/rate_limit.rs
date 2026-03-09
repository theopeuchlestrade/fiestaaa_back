use std::{
    collections::{HashMap, VecDeque},
    time::{Duration, Instant},
};

use tokio::sync::Mutex;

pub struct AuthRateLimiter {
    attempts: Mutex<HashMap<String, VecDeque<Instant>>>,
    max_attempts: usize,
    window: Duration,
}

impl AuthRateLimiter {
    pub fn new(max_attempts: usize, window: Duration) -> Self {
        Self {
            attempts: Mutex::new(HashMap::new()),
            max_attempts,
            window,
        }
    }

    pub async fn allow(&self, key: &str) -> bool {
        let now = Instant::now();
        let mut attempts = self.attempts.lock().await;
        let entry = attempts.entry(key.to_string()).or_default();

        while let Some(oldest) = entry.front() {
            if now.duration_since(*oldest) > self.window {
                entry.pop_front();
            } else {
                break;
            }
        }

        if entry.len() >= self.max_attempts {
            return false;
        }

        entry.push_back(now);
        true
    }
}
