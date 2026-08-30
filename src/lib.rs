//! Tross: a Rust reimplementation of the LinkedIn profile API service.
//!
//! Layering (one-way only):
//!
//! ```text
//! api -> service -> linkedin repository -> linkedin client/auth
//!                -> parser -> domain
//! cache -> domain
//! config/state -> construction only
//! ```

pub mod api;
pub mod app;
pub mod billing;
pub mod config;
pub mod domain;
pub mod error;
pub mod linkedin;
pub mod parser;
pub mod service;
pub mod state;
pub mod telemetry;
