//! Public domain types (response schema) and API envelopes.
//!
//! `domain` must never depend on Axum, Reqwest, or filesystem code.

pub mod profile;
pub mod response;
