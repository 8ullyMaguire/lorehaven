//! Lorehaven domain primitives.
//!
//! This crate deliberately holds no I/O and no transport. It defines the
//! vocabulary the rest of the workspace shares: identifiers, the error
//! taxonomy, and the resource policies that answer "may this actor do this
//! thing?".
//!
//! Architecture note (see `docs/adr/0001-stack.md`): the domain crate is the
//! only crate every other crate is allowed to depend on — and it depends on
//! none of them.

pub mod error;
pub mod ids;
pub mod policy;

pub use error::{AppError, ErrorCode, Result};
pub use ids::*;
