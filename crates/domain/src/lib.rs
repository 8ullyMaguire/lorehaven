//! Lorehaven domain primitives.

pub mod blocking;
pub mod caps;
pub mod charging;
pub mod community;
pub mod content;
pub mod discovery;
pub mod document;
pub mod economy;
pub mod error;
pub mod events;
pub mod exports;
pub mod extension;
pub mod fairqueue;
pub mod governance;
pub mod ids;
pub mod imports;
pub mod jobs;
pub mod library;
pub mod marketplace;
pub mod policy;
pub mod positivity;
pub mod query;
pub mod query_sql;
pub mod reading;
pub mod taxonomy;
pub mod webhook;

pub use error::{AppError, ErrorCode, Result};
pub use ids::*;
