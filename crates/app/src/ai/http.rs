//! The HTTP boundary shared by every adapter.
//!
//! A provider's failure modes are the same regardless of which API it speaks — DNS, a
//! refused connection, a timeout, a 500 — and every adapter needs the same three answers
//! from them. Handling that once means an adapter is about *parsing*, which is the part
//! that actually differs between providers.

use std::time::Duration;

use lorehaven_domain::ai::{AiAbstain, AiOutcome};

/// How long a single provider call may take.
///
/// Generous for a local model on a slow machine and short enough that a hung provider
/// does not hold a worker slot. §23.7 requires cancellation, and a timeout is the only
/// cancellation a client can enforce on someone else's server.
const TIMEOUT: Duration = Duration::from_secs(60);

pub struct AiHttp {
    client: reqwest::Client,
    base_url: String,
    /// Whether the operator has consented to private text leaving the instance, for this
    /// provider. Set at construction rather than per call so it cannot be forgotten in a
    /// new adapter — the adapter reads it and cannot bypass it.
    pub private_text_allowed: bool,
}

impl AiHttp {
    pub fn new(base_url: &str) -> Self {
        AiHttp {
            client: reqwest::Client::builder()
                .timeout(TIMEOUT)
                .build()
                .expect("a client with a timeout builds"),
            base_url: base_url.trim_end_matches('/').to_string(),
            private_text_allowed: false,
        }
    }

    /// Opt this provider in to private text. §23.7's "explicit private-text consent",
    /// defaulting to *not* allowed — a provider that has not been told otherwise must not
    /// be sent a draft.
    pub fn allow_private_text(mut self, allowed: bool) -> Self {
        self.private_text_allowed = allowed;
        self
    }

    /// `GET /v1/models` — the cheapest call that proves a provider is reachable.
    ///
    /// Used by [`AiProvider::available`](lorehaven_domain::ai::AiProvider::available),
    /// which §23.7 makes the switch that keeps a deployment disabled without
    /// configuration. A short timeout: this is a liveness check on a page load path, not a
    /// task.
    pub async fn reachable(&self) -> bool {
        self.client
            .get(format!("{}/v1/models", self.base_url))
            .timeout(Duration::from_secs(2))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }

    /// `POST` a JSON body and parse a JSON reply.
    ///
    /// Every transport failure becomes an abstain, and each one keeps its own identity —
    /// a refused connection is not the same fact as a 500, and collapsing them is how a
    /// provider outage gets reported as "no results".
    pub async fn post_json<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> AiOutcome<T> {
        let response = self
            .client
            .post(format!("{}{}", self.base_url, path))
            .json(body)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    // Retryable: §23.7 requires cancellation, and a timeout is the
                    // provider being slow rather than wrong.
                    AiAbstain::InvalidOutput(format!("provider timed out: {e}"))
                } else {
                    // Not retryable. A refused connection or a DNS failure will still fail
                    // in an hour, and retrying is how a bad configuration becomes an
                    // outage.
                    AiAbstain::NotConfigured
                }
            })?;

        let status = response.status();
        let text = response.text().await.map_err(|e| {
            AiAbstain::InvalidOutput(format!("could not read the provider reply: {e}"))
        })?;
        if !status.is_success() {
            // The status is included because a 401 and a 500 need different operator
            // action, and the body is often the provider's own explanation.
            return Err(AiAbstain::InvalidOutput(format!(
                "provider returned {status}: {}",
                text.chars().take(200).collect::<String>()
            )));
        }
        serde_json::from_str(&text)
            .map_err(|e| AiAbstain::InvalidOutput(format!("reply was not the expected JSON: {e}")))
    }
}
