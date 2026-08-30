//! Pure mappers from Voyager payloads to the public schema.
//!
//! `parser` performs no HTTP, no logging-heavy orchestration, and no disk
//! access; it is fully testable against recorded fixtures.

pub mod assembler;
pub mod common;
pub mod contact;
pub mod dash;
pub mod draft;
pub mod legacy;
pub mod normalized;
