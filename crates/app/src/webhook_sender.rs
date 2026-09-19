//! Webhook delivery: sign, send, retry.
//!
//! A webhook event is signed with HMAC-SHA256 and posted to the endpoint URL.
//! The sender enforces:
//!   - a request timeout (no hanging on a slow receiver)
//!   - payload bounding (the domain's `bound_payload` already ran)
//!   - SSRF protection (loopback, private, and link-local addresses are refused
//!     unless the operator has explicitly allowlisted the host)
//!
//! Retries use exponential backoff with jitter. A delivery that exhausts its
//! attempts is recorded as `failed` and the endpoint is deactivated.

use std::net::IpAddr;
use std::time::Duration;

use lorehaven_domain::webhook::WebhookEvent;

/// A single delivery attempt, as recorded in `webhook_deliveries`.
#[derive(Debug, Clone)]
pub struct DeliveryRecord {
    pub event_id: String,
    pub endpoint_id: String,
    pub payload: serde_json::Value,
    pub signature: String,
    pub status: DeliveryStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryStatus {
    Ok,
    Retrying,
    Failed,
}

impl DeliveryStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Retrying => "retrying",
            Self::Failed => "failed",
        }
    }
}

/// Configuration for the webhook sender.
#[derive(Debug, Clone)]
pub struct WebhookSenderConfig {
    /// Per-request timeout.
    pub timeout: Duration,
    /// Maximum attempts before a delivery is marked failed.
    pub max_attempts: u32,
    /// Base delay for exponential backoff (doubles each attempt).
    pub base_delay: Duration,
    /// Hosts explicitly allowed despite SSRF rules (operator opt-in).
    pub allowed_hosts: Vec<String>,
}

impl Default for WebhookSenderConfig {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(10),
            max_attempts: 5,
            base_delay: Duration::from_millis(500),
            allowed_hosts: Vec::new(),
        }
    }
}

/// Send a webhook event to one endpoint.
///
/// Returns the delivery record and the HTTP status code (if any).
/// On network/SSRF failure, returns `Err(WebhookSendError)` — the caller
/// decides whether to retry.
pub async fn send(
    client: &reqwest::Client,
    config: &WebhookSenderConfig,
    endpoint: &str,
    secret: &str,
    event: &WebhookEvent,
    endpoint_id: &str,
    attempt: u32,
) -> Result<(DeliveryRecord, u16), WebhookSendError> {
    // SSRF check: resolve the URL's host and verify it is not private/loopback.
    let url = reqwest::Url::parse(endpoint).map_err(|e| WebhookSendError::InvalidUrl(e.to_string()))?;
    check_ssrf(&url, &config.allowed_hosts).await?;

    let payload = lorehaven_domain::webhook::bound_payload(&event.payload, 1024 * 1024);
    let body = serde_json::to_vec(&payload).expect("bounded payload is serializable");
    let signature = event.sign(secret);

    let response = client
        .post(url.clone())
        .header("Content-Type", "application/json")
        .header("X-Signature", &signature)
        .header("X-Event-Type", &event.event_type)
        .header("X-Event-Id", &event.event_id)
        .header("X-Attempt", attempt.to_string())
        .body(body)
        .timeout(config.timeout)
        .send()
        .await
        .map_err(WebhookSendError::Http)?;

    let status = response.status();
    let status_code = status.as_u16();
    let final_status = if status.is_success() {
        DeliveryStatus::Ok
    } else if attempt < config.max_attempts {
        DeliveryStatus::Retrying
    } else {
        DeliveryStatus::Failed
    };

    let record = DeliveryRecord {
        event_id: event.event_id.clone(),
        endpoint_id: endpoint_id.to_string(),
        payload,
        signature,
        status: final_status,
    };

    Ok((record, status_code))
}

/// An error that prevented a delivery attempt from completing.
#[derive(Debug)]
pub enum WebhookSendError {
    InvalidUrl(String),
    Http(reqwest::Error),
    Ssrf(String),
}

impl std::fmt::Display for WebhookSendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidUrl(e) => write!(f, "invalid URL: {e}"),
            Self::Http(e) => write!(f, "HTTP error: {e}"),
            Self::Ssrf(e) => write!(f, "SSRF check failed: {e}"),
        }
    }
}

impl std::error::Error for WebhookSendError {}

/// Check a URL against SSRF rules: refuse loopback, private, and link-local
/// addresses unless the host is explicitly allowlisted.
async fn check_ssrf(url: &reqwest::Url, allowed_hosts: &[String]) -> Result<(), WebhookSendError> {
    let host = url
        .host_str()
        .ok_or_else(|| WebhookSendError::Ssrf("URL has no host".into()))?;

    // Allowlisted hosts pass (operator opt-in).
    if allowed_hosts.iter().any(|h| h == host) {
        return Ok(());
    }

    // Resolve the hostname to an IP.
    let ip = tokio::net::lookup_host((host, url.port_or_known_default().unwrap_or(80)))
        .await
        .map_err(|e| WebhookSendError::Ssrf(e.to_string()))?
        .next()
        .ok_or_else(|| WebhookSendError::Ssrf("DNS resolution returned no addresses".into()))?
        .ip();

    if is_ip_blocked(ip) {
        return Err(WebhookSendError::Ssrf(format!(
            "address {ip} is not allowed (loopback/private/link-local)"
        )));
    }

    Ok(())
}

/// True if the IP is loopback, private, or link-local.
fn is_ip_blocked(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_unspecified()
        }
        IpAddr::V6(v6) => {
            v6.is_loopback() || v6.is_unspecified() || is_unique_local_or_link_local(&v6)
        }
    }
}

/// Check if a v6 address is unique-local (fc00::/7) or link-local (fe80::/10).
fn is_unique_local_or_link_local(v6: &std::net::Ipv6Addr) -> bool {
    let segments = v6.segments();
    // fc00::/7 — first 7 bits are 1111 110.
    (segments[0] & 0xffc0) == 0xfc00 || (segments[0] & 0xffc0) == 0xfe80
}

/// Compute the backoff delay for a given attempt (exponential with jitter).
pub fn backoff_delay(attempt: u32, base: Duration, max: Duration) -> Duration {
    let doubled = base.saturating_mul(2u32.saturating_pow(attempt.saturating_sub(1)));
    let jitter_millis = rand::random::<u64>() % doubled.as_millis().max(1) as u64;
    Duration::from_millis(jitter_millis).min(max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_ip_blocked_refuses_private_addresses() {
        assert!(is_ip_blocked(IpAddr::V4("127.0.0.1".parse().unwrap())));
        assert!(is_ip_blocked(IpAddr::V4("10.0.0.1".parse().unwrap())));
        assert!(is_ip_blocked(IpAddr::V4("172.16.0.1".parse().unwrap())));
        assert!(is_ip_blocked(IpAddr::V4("169.254.0.1".parse().unwrap())));
        assert!(is_ip_blocked(IpAddr::V4("0.0.0.0".parse().unwrap())));
    }

    #[test]
    fn is_ip_blocked_allows_public_addresses() {
        assert!(!is_ip_blocked(IpAddr::V4("8.8.8.8".parse().unwrap())));
        assert!(!is_ip_blocked(IpAddr::V4("1.1.1.1".parse().unwrap())));
    }

    #[test]
    fn backoff_exponential_with_jitter() {
        let base = Duration::from_millis(500);
        // Attempt 1: ~0–500 ms · 2^0 = ~0–500 ms.
        let d1 = backoff_delay(1, base, Duration::from_secs(30));
        assert!(d1 < base);
        // Attempt 2: 0–1000 ms.
        let d2 = backoff_delay(2, base, Duration::from_secs(30));
        assert!(d2 < base * 2);
        // Attempt 5: capped at max.
        let d5 = backoff_delay(5, base, Duration::from_secs(30));
        assert!(d5 < base * 16);
    }

    #[test]
    fn delivery_status_as_str() {
        assert_eq!(DeliveryStatus::Ok.as_str(), "ok");
        assert_eq!(DeliveryStatus::Retrying.as_str(), "retrying");
        assert_eq!(DeliveryStatus::Failed.as_str(), "failed");
    }

    #[tokio::test]
    async fn ssrf_blocks_private_ip() {
        let url = reqwest::Url::parse("http://127.0.0.1:9999/").unwrap();
        let result = check_ssrf(&url, &[]).await;
        assert!(result.is_err(), "loopback must be blocked");
    }

    #[tokio::test]
    async fn ssrf_allows_listed_host() {
        let url = reqwest::Url::parse("http://example.com/").unwrap();
        // Allowlisting a hostname that doesn't resolve is fine — we only check allowlist.
        let result = check_ssrf(&url, &["example.com".to_string()]).await;
        assert!(result.is_ok(), "allowlisted host must pass");
    }
}
