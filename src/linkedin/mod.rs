//! LinkedIn integration: URL parsing, session/auth, HTTP client, throttling,
//! and fetch orchestration. This layer knows nothing about the public schema.

pub mod auth;
pub mod client;
pub mod endpoints;
pub mod error;
pub mod repository;
pub mod session;
pub mod throttle;
pub mod url;
