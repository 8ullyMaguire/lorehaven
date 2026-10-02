//! §23.7's provider adapters.
//!
//! `openai` covers the OpenAI-compatible API, which is also what Ollama and most local
//! runners expose; `http` holds the transport that is identical across providers, so an
//! adapter is only ever about parsing.
//!
//! Every adapter has three obligations and they are all negative: do not return
//! unvalidated model output, do not send private text without consent, and do not report
//! a transport failure as an empty result.

pub mod http;
pub mod openai;

pub use http::AiHttp;
pub use openai::OpenAiCompatible;
