//! Calibrated decision models for Lorehaven.
//!
//! A **decision model** maps text to a probability over a declared question
//! set, rather than generating text. The distinction matters for what this
//! crate is allowed to do: a generator's answer is prose to be read, and a
//! decision model's answer is a number to be *thresholded*, and a threshold is
//! where every design decision about this crate lives.
//!
//! Two halves, and the split is deliberate:
//!
//! * [`Client`] — speaks to a decision model (Laya, served by Unsloth at
//!   `POST /v1/systemone`). It transports numbers and refuses nonsense ones.
//! * [`reconcile`] — the policy deciding what a number is *allowed to do*.
//!
//! The second half is the reason this crate exists rather than a `reqwest`
//! call at the call site. `reconcile` is small, total, and testable without a
//! network, and it is the only place where a model's output turns into a
//! decision. A caller that skips it and thresholds directly has reimplemented
//! the policy with whatever defaults it happened to have.
//!
//! **The asymmetry in `reconcile` is the feature.** A model may narrow an
//! acceptance to a hold, and may do nothing else. It cannot reject, and it
//! cannot accept. That is what makes it safe to point at an instance holding
//! other people's work: the model's ceiling is a hold, which a person reviews.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod reconcile;

use std::collections::BTreeMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// How many questions one request may carry, and how wide each may be.
///
/// These are the model's limits, not ours, and exceeding them does not error
/// in a way we could detect — the model answers with its best effort on a
/// truncated question set, which is a *silently wrong* answer rather than a
/// refusal. So the limits are enforced here, before the request, where a
/// refusal is cheap and honest.
pub mod limits {
    /// Questions per request.
    pub const MAX_QUESTIONS: usize = 64;
    /// Options in one `choice`.
    pub const MAX_CHOICE_OPTIONS: usize = 255;
    /// Levels in one `score`.
    pub const MAX_SCORE_LEVELS: usize = 10;
}

/// What to ask, and how to score the answer.
///
/// The three shapes are separate variants rather than one configurable
/// question because they carry different guarantees. A `YesNo` returns a
/// probability of yes. A `Choice` returns a distribution. A `Score` returns a
/// position on a scale plus the distribution over the levels. Code that
/// treats them interchangeably has to erase information, and the information
/// is the point of using a calibrated model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Question {
    /// Yes or no. The model returns the probability of yes.
    YesNo {
        /// The key this answer comes back under.
        key: String,
        /// What to decide, in the model's own words.
        instructions: String,
    },
    /// One of several options, with a probability for each.
    Choice {
        /// The key this answer comes back under.
        key: String,
        /// What to decide, in the model's own words.
        instructions: String,
        /// Option label to the description that identifies it.
        ///
        /// A `BTreeMap` rather than a `Vec` so the request is byte-identical
        /// for identical input: a model whose own sampling is seeded by the
        /// order it sees options would otherwise make the same question
        /// non-reproducible depending on how a caller happened to build a map.
        criteria: BTreeMap<String, String>,
    },
    /// A position on an ordered scale.
    Score {
        /// The key this answer comes back under.
        key: String,
        /// What to decide, in the model's own words.
        instructions: String,
        /// The scale, in order. More than [`limits::MAX_SCORE_LEVELS`] is refused.
        criteria: Vec<String>,
    },
}

impl Question {
    /// The key this question's answer comes back under.
    #[must_use]
    pub fn key(&self) -> &str {
        match self {
            Self::YesNo { key, .. } | Self::Choice { key, .. } | Self::Score { key, .. } => key,
        }
    }

    /// Is this question within the model's limits?
    ///
    /// Checked before the request so an over-wide question is a refusal the
    /// caller can act on, rather than a model answering about a question that
    /// was quietly cut in half.
    #[must_use]
    pub fn within_limits(&self) -> bool {
        match self {
            Self::YesNo { .. } => true,
            Self::Choice { criteria, .. } => {
                !criteria.is_empty() && criteria.len() <= limits::MAX_CHOICE_OPTIONS
            }
            Self::Score { criteria, .. } => {
                !criteria.is_empty() && criteria.len() <= limits::MAX_SCORE_LEVELS
            }
        }
    }
}

/// One answer, in the shape the question asked for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Answer {
    /// The probability of yes, in `0.0..=1.0`.
    YesNo {
        /// Probability that the answer is yes.
        noul: f64,
    },
    /// The most likely option, and the distribution over all of them.
    Choice {
        /// The winning option.
        choice: String,
        /// How far one option stands out from the rest, in `0.0..=1.0`.
        confidence: f64,
        /// Every option's probability.
        probabilities: BTreeMap<String, f64>,
    },
    /// A position on the scale, counting from zero.
    Score {
        /// The position, where `0.0` is the first level.
        score: f64,
        /// How far one level stands out from the rest, in `0.0..=1.0`.
        confidence: f64,
    },
}

impl Answer {
    /// The probability of yes, for a yes/no question.
    ///
    /// A convenience for the common case, and a **refusal** for the other two
    /// rather than a conversion: a `choice` has no single probability of yes
    /// that a caller could have meant, and picking the winner's probability
    /// would answer a question nobody asked.
    #[must_use]
    pub fn probability_of_yes(&self) -> Option<f64> {
        match self {
            Self::YesNo { noul } => Some(*noul),
            Self::Choice { .. } | Self::Score { .. } => None,
        }
    }
}

/// A key and its answer, paired because the API returns them separately.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Decision {
    /// Which question this answers.
    pub key: String,
    /// What the model said.
    pub answer: Answer,
}

/// Where the model is, and how to talk to it.
#[derive(Debug, Clone)]
pub struct ClientConfig {
    /// The base URL of the serving process, without a trailing slash.
    pub base_url: String,
    /// Which model to use. `laya` names whichever the server has configured;
    /// `laya-multilingual`, `laya-english` and `laya-typed-decisions` name one.
    pub model: String,
    /// The bearer token, if the server wants one.
    pub api_key: Option<String>,
    /// How long to wait before giving up.
    pub timeout_ms: u64,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            base_url: "http://127.0.0.1:8888".to_owned(),
            model: "laya".to_owned(),
            api_key: None,
            // Long enough for a warm model, short enough that a cold one
            // cannot hold a request open indefinitely. The first request
            // against a cold model takes 10-20s and is expected to fail this
            // timeout; that is correct, and the caller falls back rather than
            // waiting out a model load inside a request handler.
            timeout_ms: 5_000,
        }
    }
}

/// Why a decision could not be obtained.
#[derive(Debug, thiserror::Error)]
pub enum DecisionError {
    /// The model could not be reached: refused the connection, timed out, or
    /// the host does not exist.
    #[error("the decision model at {url} did not answer: {source}")]
    Unreachable {
        /// Where the client tried.
        url: String,
        /// The underlying transport error.
        #[source]
        source: reqwest::Error,
    },
    /// The model answered with a non-success status.
    #[error("the decision model answered {status}")]
    Status {
        /// The status code.
        status: u16,
    },
    /// The model answered, but without the key that was asked for.
    #[error("the decision model answered without {key}")]
    MissingKey {
        /// The key that was asked for and not returned.
        key: String,
    },
    /// The model returned something that is not a probability.
    #[error("the decision model returned {value}, which is not a probability in 0.0..=1.0")]
    NotAProbability {
        /// The value as it came back.
        value: f64,
    },
    /// The request would have exceeded one of the model's own limits.
    #[error("{0}")]
    TooLarge(String),
}

impl DecisionError {
    /// Could this failure be a transient outage rather than a wrong answer?
    ///
    /// The distinction decides whether a caller may retry, and it is why
    /// `Unreachable` and `Status` are separate variants: the first is the
    /// model being down, the second is the model refusing to answer, and
    /// conflating them would make a 401 look like a network blip worth
    /// retrying forever.
    #[must_use]
    pub fn is_transient(&self) -> bool {
        match self {
            Self::Unreachable { .. } => true,
            // 5xx is the model failing; 4xx is the model saying no, and
            // repeating a refused request unchanged will be refused again.
            Self::Status { status } => *status >= 500,
            Self::MissingKey { .. } | Self::NotAProbability { .. } | Self::TooLarge(_) => false,
        }
    }
}

/// A client for a decision model.
#[derive(Debug, Clone)]
pub struct Client {
    http: reqwest::Client,
    base_url: String,
    model: String,
    api_key: Option<String>,
}

impl Client {
    /// Build a client. This does no I/O: a client that cannot reach its model
    /// is still a usable value, and the failure belongs to [`Client::ask`]
    /// where a caller can handle it.
    #[must_use]
    pub fn new(config: ClientConfig) -> Self {
        let timeout = Duration::from_millis(config.timeout_ms);
        Self {
            // A timeout on the client as well as the request, so a server that
            // accepts the connection and then stalls cannot hold a worker
            // forever. The two are set to the same value deliberately: a
            // request-level timeout that fires first would leave the
            // connection poisoned.
            http: reqwest::Client::builder()
                .timeout(timeout)
                .build()
                .unwrap_or_default(),
            base_url: config.base_url.trim_end_matches('/').to_owned(),
            model: config.model,
            api_key: config.api_key,
        }
    }

    /// The base URL this client talks to, for logging and for the audit record.
    #[must_use]
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Ask the model.
    ///
    /// `state` is any JSON value. The API accepts a bare string too, and which
    /// one is used must not change the answer, so this always sends a JSON
    /// value and lets the server do the unwrapping.
    ///
    /// # Errors
    ///
    /// Returns [`DecisionError::TooLarge`] before any I/O when a question
    /// exceeds the model's limits, and the transport/status/parse errors
    /// otherwise. Every one of them is a *refusal*, never a partial answer:
    /// a caller that gets an error has no answers, not some of them.
    pub async fn ask(
        &self,
        state: &serde_json::Value,
        questions: &[Question],
    ) -> Result<Vec<Decision>, DecisionError> {
        if questions.is_empty() {
            return Err(DecisionError::TooLarge(
                "a decision request with no questions asks nothing".to_owned(),
            ));
        }
        if questions.len() > limits::MAX_QUESTIONS {
            return Err(DecisionError::TooLarge(format!(
                "{} questions exceeds the model's limit of {}",
                questions.len(),
                limits::MAX_QUESTIONS
            )));
        }
        for question in questions {
            if !question.within_limits() {
                return Err(DecisionError::TooLarge(format!(
                    "the question `{}` exceeds the model's limits",
                    question.key()
                )));
            }
        }

        // Built explicitly rather than with `json!`: a question's key is
        // *dynamic*, and `json!` takes literal object keys, so the only way to
        // write it as a macro is to stringify the key, which produces the key
        // `"q"` rather than the question named `q`. A `Map` built in a loop
        // says what it means.
        let mut asked = serde_json::Map::new();
        for question in questions {
            asked.insert(
                question.key().to_owned(),
                serde_json::to_value(question).expect("a question is always serialisable"),
            );
        }
        let payload = serde_json::json!({
            "model": self.model,
            "state": state,
            "questions": asked,
        });

        let url = format!("{}/v1/systemone", self.base_url);
        let mut request = self.http.post(&url).json(&payload);
        if let Some(key) = &self.api_key {
            request = request.bearer_auth(key);
        }

        let response = request
            .send()
            .await
            .map_err(|source| DecisionError::Unreachable {
                url: url.clone(),
                source,
            })?;
        let status = response.status();
        if !status.is_success() {
            return Err(DecisionError::Status {
                status: status.as_u16(),
            });
        }
        let body: AnswersEnvelope = response
            .json()
            .await
            .map_err(|source| DecisionError::Unreachable { url, source })?;

        let mut decisions = Vec::with_capacity(questions.len());
        for question in questions {
            let answer = body.answers.get(question.key()).cloned().ok_or_else(|| {
                DecisionError::MissingKey {
                    key: question.key().to_owned(),
                }
            })?;
            answer.check_probabilities()?;
            decisions.push(Decision {
                key: question.key().to_owned(),
                answer,
            });
        }
        Ok(decisions)
    }
}

/// The response envelope. Only the field that is used is modelled; the API
/// also returns `model` and `usage`, and ignoring them is deliberate — they
/// are not needed to act on the answer, and modelling them would be a promise
/// to keep them accurate.
#[derive(Debug, Deserialize)]
struct AnswersEnvelope {
    answers: BTreeMap<String, Answer>,
}

impl Answer {
    /// Refuse an answer whose numbers are not probabilities.
    ///
    /// A `NaN` is the case that makes this worth doing at all. Every
    /// comparison against `NaN` is false, so a `NaN` posterior silently takes
    /// the *accept* branch of a `p < threshold` test — the model's least
    /// meaningful output would become the most permissive one. Clamping is
    /// worse still: a `1.9` clamped to `1.0` is the strongest possible
    /// confidence out of garbage. So both are refused.
    fn check_probabilities(&self) -> Result<(), DecisionError> {
        let check = |value: f64| {
            if value.is_finite() && (0.0..=1.0).contains(&value) {
                Ok(())
            } else {
                Err(DecisionError::NotAProbability { value })
            }
        };
        match self {
            Self::YesNo { noul } => check(*noul),
            Self::Choice {
                confidence,
                probabilities,
                ..
            } => {
                check(*confidence)?;
                for probability in probabilities.values() {
                    if !probability.is_finite() || *probability < 0.0 {
                        return Err(DecisionError::NotAProbability {
                            value: *probability,
                        });
                    }
                }
                // The distribution's total is NOT required to be 1.0. A model
                // that reports 0.999 is not lying and renormalising it would
                // invent a precision the model did not claim. Only a total
                // above 1.0 is refused, because that cannot be a distribution.
                let total: f64 = probabilities.values().sum();
                if total > 1.0 + f64::EPSILON {
                    return Err(DecisionError::NotAProbability { value: total });
                }
                Ok(())
            }
            Self::Score {
                score, confidence, ..
            } => {
                check(*confidence)?;
                // The score is a POSITION, not a probability: it indexes the
                // scale, so it may be fractional and may exceed 1.0 on a
                // ten-level scale. It is checked for finiteness only, and
                // range-checking it against 1.0 would refuse every score
                // model that ever answered.
                if score.is_finite() {
                    Ok(())
                } else {
                    Err(DecisionError::NotAProbability { value: *score })
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn yes_no(key: &str) -> Question {
        Question::YesNo {
            key: key.to_owned(),
            instructions: "Does the customer ask for a refund?".to_owned(),
        }
    }

    #[test]
    fn a_probability_is_a_probability() {
        let answer = Answer::YesNo { noul: 0.98 };
        assert!(answer.check_probabilities().is_ok());
        assert_eq!(answer.probability_of_yes(), Some(0.98));
    }

    #[test]
    fn a_nan_is_refused_rather_than_clamped() {
        // The reason this function exists. `NaN < threshold` is false, so an
        // unclamped NaN takes the accept branch of every threshold test: the
        // model's least meaningful output would become the most permissive one.
        let answer = Answer::YesNo { noul: f64::NAN };
        assert!(matches!(
            answer.check_probabilities(),
            Err(DecisionError::NotAProbability { .. })
        ));
    }

    #[test]
    fn an_infinite_probability_is_refused() {
        let answer = Answer::YesNo {
            noul: f64::INFINITY,
        };
        assert!(answer.check_probabilities().is_err());
    }

    #[test]
    fn a_probability_above_one_is_refused() {
        let answer = Answer::YesNo { noul: 1.4 };
        assert!(answer.check_probabilities().is_err());
    }

    #[test]
    fn a_distribution_totalling_above_one_is_refused() {
        let answer = Answer::Choice {
            choice: "billing".to_owned(),
            confidence: 0.6,
            probabilities: BTreeMap::from([
                ("billing".to_owned(), 0.7),
                ("technical".to_owned(), 0.7),
            ]),
        };
        assert!(answer.check_probabilities().is_err());
    }

    #[test]
    fn a_distribution_totalling_just_under_one_is_kept_as_reported() {
        // Not renormalised. The model said 0.999 and that is what it said.
        let answer = Answer::Choice {
            choice: "billing".to_owned(),
            confidence: 0.6,
            probabilities: BTreeMap::from([("billing".to_owned(), 0.999)]),
        };
        assert!(answer.check_probabilities().is_ok());
    }

    #[test]
    fn a_score_may_exceed_one_because_it_is_a_position_not_a_probability() {
        // A ten-level scale runs 0..9. Refusing 1.9 here would refuse every
        // score model that ever answered.
        let answer = Answer::Score {
            score: 1.9,
            confidence: 0.7,
        };
        assert!(answer.check_probabilities().is_ok());
    }

    #[test]
    fn a_non_finite_score_is_refused() {
        let answer = Answer::Score {
            score: f64::NAN,
            confidence: 0.7,
        };
        assert!(answer.check_probabilities().is_err());
    }

    #[test]
    fn a_choice_has_no_probability_of_yes() {
        // Rather than a conversion nobody asked for.
        let answer = Answer::Choice {
            choice: "billing".to_owned(),
            confidence: 1.0,
            probabilities: BTreeMap::from([("billing".to_owned(), 1.0)]),
        };
        assert_eq!(answer.probability_of_yes(), None);
    }

    #[test]
    fn a_question_with_no_options_is_refused() {
        let question = Question::Choice {
            key: "team".to_owned(),
            instructions: "Which team?".to_owned(),
            criteria: BTreeMap::new(),
        };
        assert!(!question.within_limits());
    }

    #[test]
    fn a_score_with_more_levels_than_the_model_allows_is_refused() {
        let question = Question::Score {
            key: "urgency".to_owned(),
            instructions: "How urgent?".to_owned(),
            criteria: (0..11).map(|i| format!("level {i}")).collect(),
        };
        assert!(!question.within_limits());
    }

    #[test]
    fn a_choice_with_the_maximum_options_is_accepted() {
        let question = Question::Choice {
            key: "team".to_owned(),
            instructions: "Which team?".to_owned(),
            criteria: (0..limits::MAX_CHOICE_OPTIONS)
                .map(|i| (format!("team-{i}"), format!("description {i}")))
                .collect(),
        };
        assert!(question.within_limits());
    }

    #[tokio::test]
    async fn a_request_with_no_questions_is_refused_before_any_io() {
        // The client is never given a URL that could be dialled, so a refusal
        // here cannot have been preceded by a request.
        let client = Client::new(ClientConfig {
            base_url: "http://127.0.0.1:1".to_owned(),
            ..ClientConfig::default()
        });
        let error = client
            .ask(&serde_json::json!("text"), &[])
            .await
            .expect_err("an empty question set asks nothing");
        assert!(matches!(error, DecisionError::TooLarge(_)));
    }

    #[tokio::test]
    async fn too_many_questions_is_refused_before_any_io() {
        let client = Client::new(ClientConfig {
            base_url: "http://127.0.0.1:1".to_owned(),
            ..ClientConfig::default()
        });
        let questions: Vec<Question> = (0..=limits::MAX_QUESTIONS)
            .map(|i| yes_no(&format!("q{i}")))
            .collect();
        let error = client
            .ask(&serde_json::json!("text"), &questions)
            .await
            .expect_err("65 questions is one too many");
        assert!(matches!(error, DecisionError::TooLarge(_)));
    }

    #[tokio::test]
    async fn an_unreachable_model_is_transient_and_a_refusal_is_not() {
        // The distinction decides whether a caller may retry.
        let client = Client::new(ClientConfig {
            base_url: "http://127.0.0.1:1".to_owned(),
            timeout_ms: 250,
            ..ClientConfig::default()
        });
        let error = client
            .ask(&serde_json::json!("text"), &[yes_no("refund")])
            .await
            .expect_err("nothing is listening on port 1");
        assert!(matches!(error, DecisionError::Unreachable { .. }));
        assert!(error.is_transient(), "a model that is down may be retried");

        assert!(!DecisionError::Status { status: 401 }.is_transient());
        assert!(!DecisionError::Status { status: 422 }.is_transient());
        assert!(DecisionError::Status { status: 503 }.is_transient());
        assert!(!DecisionError::MissingKey {
            key: "x".to_owned()
        }
        .is_transient());
    }
}
