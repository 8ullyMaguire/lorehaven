//! HTTP rendering for domain errors, and request correlation.
//!
//! Spec §3.3 requires one JSON error envelope with a `request_id`, and spec
//! §5 requires request IDs in the logs. Both come from the same per-request
//! value, installed here as a task-local so that `into_response` can stamp it
//! without every handler threading it through by hand.

use std::collections::BTreeMap;
use std::future::Future;

use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use lorehaven_domain::error::AppError;
use lorehaven_domain::ids::RequestId;

tokio::task_local! {
    static CURRENT_REQUEST_ID: String;
}

/// Run `future` with `request_id` installed as the current correlation id.
pub async fn with_request_id<F, T>(request_id: String, future: F) -> T
where
    F: Future<Output = T>,
{
    CURRENT_REQUEST_ID.scope(request_id, future).await
}

/// The correlation id of the in-flight request, if we are inside one.
#[must_use]
pub fn current_request_id() -> Option<RequestId> {
    CURRENT_REQUEST_ID
        .try_with(|id| RequestId::sanitize(id))
        .ok()
        .flatten()
}

/// A [`AppError`] that knows how to render itself.
///
/// A newtype rather than an impl on `AppError` directly, because the orphan
/// rule forbids implementing a foreign trait (`IntoResponse`) for a foreign
/// type (`AppError` lives in the domain crate) — and keeping the domain free of
/// transport is worth more than saving a wrapper.
#[derive(Debug)]
pub struct ApiError(pub AppError);

/// Convenience alias for handler signatures.
pub type ApiResult<T> = Result<T, ApiError>;

impl From<AppError> for ApiError {
    fn from(error: AppError) -> Self {
        Self(error)
    }
}

impl From<lorehaven_db::content::ContentError> for ApiError {
    fn from(error: lorehaven_db::content::ContentError) -> Self {
        match error {
            lorehaven_db::content::ContentError::Refused(app) => Self(app),
            lorehaven_db::content::ContentError::Fault(cause) => Self(AppError::Internal(cause)),
        }
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(error: anyhow::Error) -> Self {
        Self(AppError::Internal(error))
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}

impl std::error::Error for ApiError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    code: &'static str,
    message: String,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    field_errors: BTreeMap<String, String>,
    request_id: String,
}

#[derive(Debug, Serialize)]
struct ErrorEnvelope {
    error: ErrorBody,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let error = self.0;
        let status =
            StatusCode::from_u16(error.status_code()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let code = error.code();

        // Faults are logged with their full chain; refusals are not noise.
        // `?error` rather than `%error`: the `Display` form of an internal
        // error is masked by design, so the diagnostic detail only appears in
        // the `Debug` rendering.
        if error.is_fault() {
            tracing::error!(error = ?error, code = code.as_str(), "request failed");
        } else {
            tracing::debug!(code = code.as_str(), "request rejected");
        }

        let request_id =
            current_request_id().map_or_else(|| "unavailable".to_owned(), |id| id.to_string());

        let body = ErrorEnvelope {
            error: ErrorBody {
                code: code.as_str(),
                message: error.public_message(),
                field_errors: error.field_errors(),
                request_id,
            },
        };

        let mut response = (status, axum::Json(body)).into_response();
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );

        if let Some(seconds) = error.retry_after_secs() {
            if let Ok(value) = HeaderValue::from_str(&seconds.to_string()) {
                response.headers_mut().insert(header::RETRY_AFTER, value);
            }
        }

        if status == StatusCode::UNAUTHORIZED {
            response.headers_mut().insert(
                header::WWW_AUTHENTICATE,
                HeaderValue::from_static("Session"),
            );
        }

        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_envelope_matches_the_documented_shape() {
        let response = with_request_id("req-42".to_owned(), async {
            ApiError(AppError::RevisionConflict {
                expected: 12,
                actual: 13,
            })
            .into_response()
        })
        .await;

        let (parts, body) = response.into_parts();
        assert_eq!(parts.status, StatusCode::CONFLICT);

        let bytes = axum::body::to_bytes(body, 64 * 1024).await.expect("body");
        let json: serde_json::Value = serde_json::from_slice(&bytes).expect("json");

        assert_eq!(json["error"]["code"], "REVISION_CONFLICT");
        assert_eq!(json["error"]["request_id"], "req-42");
        assert!(json["error"]["message"]
            .as_str()
            .is_some_and(|m| !m.is_empty()));
        // An empty field_errors map is omitted rather than serialised as {}.
        assert!(json["error"].get("field_errors").is_none());
    }

    #[tokio::test]
    async fn validation_field_errors_are_included() {
        let response = with_request_id("req-1".to_owned(), async {
            ApiError(AppError::field("title", "must not be empty")).into_response()
        })
        .await;

        let (parts, body) = response.into_parts();
        assert_eq!(parts.status, StatusCode::UNPROCESSABLE_ENTITY);

        let bytes = axum::body::to_bytes(body, 64 * 1024).await.expect("body");
        let json: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
        assert_eq!(json["error"]["field_errors"]["title"], "must not be empty");
    }

    #[tokio::test]
    async fn rate_limits_advise_a_retry_delay() {
        let response = with_request_id("req-2".to_owned(), async {
            ApiError(AppError::RateLimited {
                retry_after_secs: 45,
            })
            .into_response()
        })
        .await;

        let (parts, _body) = response.into_parts();
        assert_eq!(parts.status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            parts
                .headers
                .get(header::RETRY_AFTER)
                .and_then(|value| value.to_str().ok()),
            Some("45")
        );
    }

    #[tokio::test]
    async fn authentication_failures_advertise_the_scheme() {
        let response = with_request_id("req-3".to_owned(), async {
            ApiError(AppError::AuthRequired).into_response()
        })
        .await;
        let (parts, _body) = response.into_parts();
        assert_eq!(parts.status, StatusCode::UNAUTHORIZED);
        assert_eq!(
            parts
                .headers
                .get(header::WWW_AUTHENTICATE)
                .and_then(|value| value.to_str().ok()),
            Some("Session")
        );
    }

    #[tokio::test]
    async fn internal_faults_are_masked_but_still_correlated() {
        let response = with_request_id("req-4".to_owned(), async {
            ApiError(AppError::internal("db", anyhow::anyhow!("dsn=secret"))).into_response()
        })
        .await;

        let (parts, body) = response.into_parts();
        assert_eq!(parts.status, StatusCode::INTERNAL_SERVER_ERROR);
        let bytes = axum::body::to_bytes(body, 64 * 1024).await.expect("body");
        let text = String::from_utf8_lossy(&bytes);
        assert!(!text.contains("secret"));
        assert!(text.contains("req-4"));
    }

    #[tokio::test]
    async fn anyhow_errors_convert_into_api_errors() {
        fn fallible() -> Result<(), anyhow::Error> {
            Err(anyhow::anyhow!("boom"))
        }
        async fn handler() -> ApiResult<()> {
            fallible()?;
            Ok(())
        }

        // The `?` in `handler` converts an `anyhow::Error` into an `ApiError`,
        // which is the path every repository call takes.
        let error: ApiError = handler().await.expect_err("must fail");
        assert_eq!(error.0.code(), lorehaven_domain::ErrorCode::Internal);

        let response =
            with_request_id("req-5".to_owned(), async move { error.into_response() }).await;
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }
}
