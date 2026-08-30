//! Minimal parser benchmark (no criterion): timings are printed by main()
//! with std::time::Instant, and the bench harness is disabled in Cargo.toml
//! (`[[bench]] name = "parser", harness = false`).
//!
//! Run with: `cargo bench --bench parser` (or `cargo run --release --bench parser`).
//!
//! Reads CARGO_MANIFEST_DIR/fixtures/dash_normalized.json — a golden fixture
//! of LinkedIn's normalised Dash envelope. If the fixture is missing the bench
//! prints a note and exits 0, so `cargo bench` never fails on a bare checkout.

use std::time::Instant;

const ITERATIONS: usize = 200;

fn main() {
    let fixture = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/dash_normalized.json");
    let raw = match std::fs::read_to_string(fixture) {
        Ok(raw) => raw,
        Err(err) => {
            println!("bench: fixture not readable ({fixture}): {err}; skipping");
            return;
        }
    };

    let input: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(value) => value,
        Err(err) => {
            eprintln!("bench: fixture is not valid JSON: {err}");
            return;
        }
    };

    let build_profile = |document: tross::parser::dash::DashProfileDocument| {
        let now = chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
            .expect("canned timestamp")
            .with_timezone(&chrono::Utc);
        let draft = tross::parser::draft::ProfileDraft {
            identity: document.identity,
            sections: document.sections,
            network: document.network,
            contact: document.contact,
            strategy: "bench".to_string(),
        };
        tross::parser::assembler::build_profile(
            "bench",
            "vanity",
            "https://www.linkedin.com/in/bench/",
            &draft,
            Vec::new(),
            Vec::new(),
            now,
        )
    };

    // One untimed iteration so one-time warm-up (allocators, lazy statics)
    // does not inflate the average.
    let _ = tross::parser::dash::parse_dash_profile(&input, (2026, 8));

    let mut bytes: f64 = raw.len() as f64;
    let mut parsed = 0usize;
    let start = Instant::now();
    for _ in 0..ITERATIONS {
        if let Some(document) = tross::parser::dash::parse_dash_profile(&input, (2026, 8))
            && let Ok(rendered) = serde_json::to_string(&build_profile(document))
        {
            bytes += rendered.len() as f64;
            parsed += 1;
        }
    }
    let elapsed = start.elapsed();
    let seconds = elapsed.as_secs_f64();

    if parsed == 0 {
        eprintln!("bench: fixture produced no profile; nothing to measure");
        return;
    }

    println!(
        "parsed {parsed}/{ITERATIONS} profiles in {elapsed:.2?}: {:.3} ms/iter, {:.2} MB/s (input + serialized output)",
        elapsed.as_secs_f64() * 1_000.0 / ITERATIONS as f64,
        bytes / 1_000_000.0 / seconds,
    );
}
