//! Shared HTTP retry helper used by both provider implementations.
//!
//! Spec §8 retry policy (resolved per the implementation design doc):
//!   - 5xx → retry. Sleep 1s before retry #1, 2s before retry #2.
//!   - 4xx → fail immediately.
//!   - 429 with Retry-After → wait and retry once.
//!   - Network/timeout → fail immediately (no retry on transport errors in M1).

use reqwest::header::HeaderValue;
use std::time::Duration;

use crate::error::TranslateError;

/// Build the credential-bearing HTTP client shared by provider adapters.
/// Redirects are disabled so authorization headers and request bodies are
/// never replayed to a different origin.
pub fn provider_http_client(timeout: Duration) -> Result<reqwest::Client, TranslateError> {
    reqwest::Client::builder()
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("clipt9n/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| TranslateError::Network(format!("building HTTP client: {e}")))
}

const PROVIDER_ERROR_BODY_BUDGET_BYTES: usize = 8 * 1024;
const PROVIDER_ERROR_MESSAGE_MAX_CHARS: usize = 2_000;

/// Read a bounded portion of a provider error response and sanitize it before
/// storing it in a structured error. Newline and tab remain available for
/// readable provider diagnostics; all other control characters are stripped.
pub async fn bounded_provider_error(mut response: reqwest::Response) -> String {
    let mut body = Vec::with_capacity(PROVIDER_ERROR_BODY_BUDGET_BYTES);
    while body.len() < PROVIDER_ERROR_BODY_BUDGET_BYTES {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                let remaining = PROVIDER_ERROR_BODY_BUDGET_BYTES - body.len();
                let retained = remaining.min(chunk.len());
                body.extend_from_slice(&chunk[..retained]);
                if retained < chunk.len() {
                    break;
                }
            }
            Ok(None) => break,
            Err(error) => {
                let detail = format!("reading provider error response: {error}");
                let remaining = PROVIDER_ERROR_BODY_BUDGET_BYTES - body.len();
                body.extend_from_slice(&detail.as_bytes()[..detail.len().min(remaining)]);
                break;
            }
        }
    }

    let sanitized: String = String::from_utf8_lossy(&body)
        .chars()
        .filter(|character| !character.is_control() || *character == '\n' || *character == '\t')
        .collect();
    if sanitized.chars().count() <= PROVIDER_ERROR_MESSAGE_MAX_CHARS {
        sanitized
    } else {
        let mut bounded: String = sanitized
            .chars()
            .take(PROVIDER_ERROR_MESSAGE_MAX_CHARS - 1)
            .collect();
        bounded.push('…');
        bounded
    }
}

/// Outcome of a single retryable attempt.
pub enum AttemptOutcome<T, E> {
    /// Operation succeeded; return the value.
    Done(T),
    /// Transient failure; sleep and retry if budget remaining.
    Retry(E),
    /// Transient failure with a server-provided delay; retry once if budget remains.
    RetryAfter(Duration, E),
    /// Permanent failure; return the error immediately.
    Fatal(E),
}

/// Run `op` with retries on `Retry` outcomes.
///
/// `backoffs[i]` is the sleep duration before attempt `i+1` (0-indexed in
/// terms of retries, not total attempts). Number of attempts =
/// `backoffs.len() + 1`.
///
/// Returns the first `Done` value, or the last `Retry`/`Fatal` error if all
/// attempts fail.
pub async fn with_retry<T, E, F, Fut>(backoffs: &[Duration], mut op: F) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = AttemptOutcome<T, E>>,
{
    let mut last_err: Option<E> = None;
    let mut next_delay: Option<Duration> = None;
    let mut retry_after_used = false;
    let total_attempts = backoffs.len() + 1;
    for attempt in 0..total_attempts {
        if attempt > 0 {
            tokio::time::sleep(next_delay.take().unwrap_or(backoffs[attempt - 1])).await;
        }
        match op().await {
            AttemptOutcome::Done(v) => return Ok(v),
            AttemptOutcome::Fatal(e) => return Err(e),
            AttemptOutcome::Retry(e) => {
                last_err = Some(e);
            }
            AttemptOutcome::RetryAfter(delay, e) => {
                if retry_after_used || attempt + 1 >= total_attempts {
                    return Err(e);
                }
                retry_after_used = true;
                last_err = Some(e);
                next_delay = Some(delay);
            }
        }
    }
    Err(last_err.expect("with_retry called with empty backoffs and op() returned Retry"))
}

/// Parse a `Retry-After` header containing integer seconds.
pub fn parse_retry_after(value: Option<&HeaderValue>) -> Option<Duration> {
    let seconds = value?.to_str().ok()?.trim().parse::<u64>().ok()?;
    Some(Duration::from_secs(seconds.min(30)))
}

/// The default backoff schedule used by both providers in production.
/// `[1s, 2s]` → 3 total attempts on 5xx (initial + retry #1 after 1s + retry
/// #2 after 2s).
pub fn default_backoffs() -> Vec<Duration> {
    vec![Duration::from_secs(1), Duration::from_secs(2)]
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    use super::*;

    fn fast_backoffs() -> Vec<Duration> {
        // Use millisecond-scale sleeps in tests so the suite doesn't take
        // 3+ seconds for retry assertions.
        vec![Duration::from_millis(1), Duration::from_millis(2)]
    }

    #[tokio::test]
    async fn succeeds_on_first_attempt() {
        let count = Arc::new(AtomicU32::new(0));
        let c = count.clone();
        let result: Result<u32, &str> = with_retry(&fast_backoffs(), || {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                AttemptOutcome::Done(42u32)
            }
        })
        .await;
        assert_eq!(result.unwrap(), 42);
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn retries_then_succeeds() {
        let count = Arc::new(AtomicU32::new(0));
        let c = count.clone();
        let result: Result<u32, &str> = with_retry(&fast_backoffs(), || {
            let c = c.clone();
            async move {
                let n = c.fetch_add(1, Ordering::SeqCst);
                if n < 2 {
                    AttemptOutcome::Retry("transient")
                } else {
                    AttemptOutcome::Done(99u32)
                }
            }
        })
        .await;
        assert_eq!(result.unwrap(), 99);
        assert_eq!(count.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn gives_up_after_all_attempts_exhausted() {
        let count = Arc::new(AtomicU32::new(0));
        let c = count.clone();
        let result: Result<u32, &str> = with_retry(&fast_backoffs(), || {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                AttemptOutcome::Retry("still failing")
            }
        })
        .await;
        assert_eq!(result.unwrap_err(), "still failing");
        assert_eq!(count.load(Ordering::SeqCst), 3); // 1 initial + 2 retries
    }

    #[tokio::test]
    async fn fatal_returns_immediately() {
        let count = Arc::new(AtomicU32::new(0));
        let c = count.clone();
        let result: Result<u32, &str> = with_retry(&fast_backoffs(), || {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                AttemptOutcome::Fatal("4xx")
            }
        })
        .await;
        assert_eq!(result.unwrap_err(), "4xx");
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }
}
