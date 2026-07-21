//! Throttling for the credential endpoints.
//!
//! This closes the last open item from the security review. It targets the
//! attack the app is actually exposed to in `jwt` mode: online password
//! guessing against `/api/auth/login`, and mass account probing through
//! register / forgot-password / magic-link.
//!
//! ## Scope and honest limitations
//!
//! State is process-local, deliberately. Castle deploys one pod per tenant with
//! a single replica, so a process is the whole instance and an in-memory
//! counter sees every attempt — no Redis, no shared store, no extra failure
//! domain. **If a tenant is ever scaled past one replica this becomes
//! per-replica**, and the effective limit multiplies by the replica count; at
//! that point this needs to move to a shared store or to the ingress.
//!
//! It is also not a defence against distributed flooding: an attacker with many
//! source addresses can still exhaust connections upstream of the app. That is
//! the ingress's job (`rateLimit` in the tenant chart sets nginx's
//! `limit-rps`/`limit-connections` per source address), and the two layers are
//! complementary — this one protects *accounts*, that one protects *capacity*.
//!
//! ## The lockout trade-off, stated plainly
//!
//! Login is keyed on the submitted email and checked *before* the password is
//! verified. That is what keeps an attacker from spending our CPU on argon2 for
//! every guess — but it means someone who knows a client's address can keep that
//! account throttled by guessing at it. The exposure is bounded: the window
//! never extends while blocked, so the account is usable again within
//! `LOGIN_WINDOW`, and a correct password clears the counter for good.
//!
//! Verifying the password first would remove the lockout entirely (the real user
//! always gets in) at the cost of unbounded hashing on demand. Between a
//! targeted 15-minute delay and a cheap CPU-exhaustion vector, this takes the
//! delay. If that ever proves wrong for a client, the fix is a two-tier limit —
//! verify beyond the soft cap, refuse outright at a hard one — not removing the
//! check.
//!
//! In `proxy` mode (production) Keycloak performs authentication and its own
//! brute-force detection applies; these limits then guard the endpoints that
//! remain reachable, including the honeypot's fake login.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::http::StatusCode;
use loco_rs::controller::ErrorDetail;
use loco_rs::prelude::*;

/// Failed logins allowed per key before the endpoint locks out.
const LOGIN_MAX_ATTEMPTS: u32 = 8;
/// Window over which those failures are counted.
const LOGIN_WINDOW: Duration = Duration::from_secs(15 * 60);
/// Requests allowed per key on the account-enumerating endpoints.
const SIGNUP_MAX_ATTEMPTS: u32 = 5;
const SIGNUP_WINDOW: Duration = Duration::from_secs(60 * 60);
/// Hard cap on tracked keys, so an attacker rotating keys cannot grow the map
/// without bound. Reaching it drops the oldest expired entries first.
const MAX_TRACKED_KEYS: usize = 10_000;

struct Bucket {
    count: u32,
    /// When the window closes and the count resets.
    expires_at: Instant,
}

/// A fixed-window counter keyed by an arbitrary string.
///
/// Fixed windows allow a burst at a window boundary (up to 2× the limit across
/// two adjacent windows). That is accepted: the goal is to make sustained
/// guessing impractical, not to smooth traffic, and the simpler algorithm is
/// easier to reason about than a leaky bucket.
pub struct RateLimiter {
    max: u32,
    window: Duration,
    buckets: Mutex<HashMap<String, Bucket>>,
}

impl RateLimiter {
    #[must_use]
    pub fn new(max: u32, window: Duration) -> Self {
        Self {
            max,
            window,
            buckets: Mutex::new(HashMap::new()),
        }
    }

    /// Records an attempt against `key`.
    ///
    /// Returns `Err(retry_after)` once the key has exceeded its allowance,
    /// without extending the window — an attacker cannot keep themselves locked
    /// out forever, and a legitimate user's wait only ever shrinks.
    pub fn check(&self, key: &str, now: Instant) -> std::result::Result<(), Duration> {
        let mut buckets = self.lock();
        Self::prune(&mut buckets, now);

        let bucket = buckets.entry(key.to_string()).or_insert(Bucket {
            count: 0,
            expires_at: now + self.window,
        });
        if bucket.expires_at <= now {
            bucket.count = 0;
            bucket.expires_at = now + self.window;
        }
        if bucket.count >= self.max {
            return Err(bucket.expires_at.saturating_duration_since(now));
        }
        bucket.count += 1;
        Ok(())
    }

    /// Clears a key's counter. Called after a *successful* login so that a user
    /// who mistyped their password a few times is not left near the limit.
    pub fn reset(&self, key: &str) {
        self.lock().remove(key);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Bucket>> {
        // A poisoned mutex only means some other request panicked while holding
        // it; the counters remain meaningful, so recover rather than propagate.
        self.buckets.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Drops expired entries. Runs on every check, which is O(n) only once the
    /// map is over the cap; below that it is a cheap no-op.
    fn prune(buckets: &mut HashMap<String, Bucket>, now: Instant) {
        if buckets.len() < MAX_TRACKED_KEYS {
            return;
        }
        buckets.retain(|_, b| b.expires_at > now);
        // Still full of live entries: the instance is under a rotating-key
        // flood. Drop everything rather than grow without bound — the limits
        // reset, but memory stays fixed and the ingress limiter still applies.
        if buckets.len() >= MAX_TRACKED_KEYS {
            buckets.clear();
        }
    }
}

fn login_limiter() -> &'static RateLimiter {
    static LIMITER: OnceLock<RateLimiter> = OnceLock::new();
    LIMITER.get_or_init(|| RateLimiter::new(LOGIN_MAX_ATTEMPTS, LOGIN_WINDOW))
}

fn signup_limiter() -> &'static RateLimiter {
    static LIMITER: OnceLock<RateLimiter> = OnceLock::new();
    LIMITER.get_or_init(|| RateLimiter::new(SIGNUP_MAX_ATTEMPTS, SIGNUP_WINDOW))
}

fn too_many_requests(retry_after: Duration) -> Error {
    // Deliberately vague: the response must not reveal whether the account
    // exists, only that this caller is going too fast.
    Error::CustomError(
        StatusCode::TOO_MANY_REQUESTS,
        ErrorDetail::new(
            "too_many_requests".to_string(),
            format!(
                "too many attempts, try again in {} seconds",
                retry_after.as_secs().max(1)
            ),
        ),
    )
}

/// Throttles a login attempt.
///
/// Keyed on the submitted email rather than the client address: the address is
/// either unavailable or proxy-supplied, and credential stuffing rotates
/// addresses while targeting one account. Keying on the account is what
/// actually protects it.
pub fn check_login(email: &str) -> Result<()> {
    login_limiter()
        .check(&normalize(email), Instant::now())
        .map_err(too_many_requests)
}

/// Clears a login counter after successful authentication.
pub fn clear_login(email: &str) {
    login_limiter().reset(&normalize(email));
}

/// Throttles the endpoints that reveal whether an address has an account
/// (register, forgot-password, magic-link).
pub fn check_signup(email: &str) -> Result<()> {
    signup_limiter()
        .check(&normalize(email), Instant::now())
        .map_err(too_many_requests)
}

fn normalize(email: &str) -> String {
    email.trim().to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_up_to_the_limit_then_blocks() {
        let limiter = RateLimiter::new(3, Duration::from_secs(60));
        let now = Instant::now();
        for _ in 0..3 {
            assert!(limiter.check("a@example.com", now).is_ok());
        }
        assert!(limiter.check("a@example.com", now).is_err());
    }

    #[test]
    fn keys_are_independent() {
        let limiter = RateLimiter::new(1, Duration::from_secs(60));
        let now = Instant::now();
        assert!(limiter.check("a@example.com", now).is_ok());
        assert!(limiter.check("a@example.com", now).is_err());
        // One account being attacked must not lock anyone else out.
        assert!(limiter.check("b@example.com", now).is_ok());
    }

    #[test]
    fn window_expiry_restores_access() {
        let window = Duration::from_secs(60);
        let limiter = RateLimiter::new(1, window);
        let now = Instant::now();
        assert!(limiter.check("a@example.com", now).is_ok());
        assert!(limiter.check("a@example.com", now).is_err());
        assert!(limiter.check("a@example.com", now + window).is_ok());
    }

    #[test]
    fn blocked_attempts_do_not_extend_the_window() {
        let window = Duration::from_secs(60);
        let limiter = RateLimiter::new(1, window);
        let start = Instant::now();
        assert!(limiter.check("a@example.com", start).is_ok());
        // Hammering while blocked must not push the unlock time out.
        for i in 1..30 {
            let _ = limiter.check("a@example.com", start + Duration::from_secs(i));
        }
        assert!(limiter.check("a@example.com", start + window).is_ok());
    }

    #[test]
    fn retry_after_shrinks_as_the_window_closes() {
        let window = Duration::from_secs(60);
        let limiter = RateLimiter::new(1, window);
        let start = Instant::now();
        assert!(limiter.check("a@example.com", start).is_ok());
        let first = limiter.check("a@example.com", start).unwrap_err();
        let later = limiter
            .check("a@example.com", start + Duration::from_secs(30))
            .unwrap_err();
        assert!(later < first, "{later:?} should be less than {first:?}");
    }

    #[test]
    fn reset_clears_the_counter() {
        let limiter = RateLimiter::new(1, Duration::from_secs(60));
        let now = Instant::now();
        assert!(limiter.check("a@example.com", now).is_ok());
        assert!(limiter.check("a@example.com", now).is_err());
        limiter.reset("a@example.com");
        assert!(limiter.check("a@example.com", now).is_ok());
    }

    #[test]
    fn keys_are_case_and_whitespace_insensitive() {
        // Otherwise "Alice@example.com" and "alice@example.com " would each get
        // their own allowance for the same account.
        assert_eq!(normalize("  Alice@Example.COM "), "alice@example.com");
    }

    #[test]
    fn tracked_keys_stay_bounded_under_a_rotating_key_flood() {
        let limiter = RateLimiter::new(1, Duration::from_secs(60));
        let now = Instant::now();
        for i in 0..(MAX_TRACKED_KEYS + 500) {
            let _ = limiter.check(&format!("{i}@example.com"), now);
        }
        assert!(limiter.lock().len() <= MAX_TRACKED_KEYS);
    }
}
