//! Logging configuration: readable console output in development, single-line
//! JSON in production. No log records ever contain cookie values, API keys,
//! or passwords.

use tracing_subscriber::EnvFilter;

use crate::config::{LogFormat, Settings};

pub fn init_logging(settings: &Settings) {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(settings.log_level.clone()));

    let builder = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false);

    match settings.log_format {
        LogFormat::Console => {
            builder.with_ansi(false).with_writer(std::io::stdout).init();
        }
        LogFormat::Json => {
            builder
                .json()
                .with_current_span(false)
                .with_span_list(false)
                .with_writer(std::io::stdout)
                .init();
        }
    }
}
