//! Lorehaven domain primitives.
//!
//! This crate deliberately holds no I/O and no transport. It defines the
//! vocabulary the rest of the workspace shares: identifiers, the error
//! taxonomy, the resource policies that answer "may this actor do this thing?",
//! and the restricted editor document format.
//!
//! Architecture note (see `docs/adr/0001-stack.md`): the domain crate is the
//! only crate every other crate is allowed to depend on — and it depends on
//! none of them.

pub mod content;
pub mod document;
pub mod error;
pub mod ids;
pub mod policy;
pub mod reading;

pub use error::{AppError, ErrorCode, Result};
pub use ids::*;
