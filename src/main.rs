//! Process entry point: settings, logging, composition, and the HTTP server.
//!
//! Subcommands (keep tiny): `healthcheck` (used by the container health probe),
//! `--version`, `--help`.

use std::sync::Arc;
use std::time::Instant;

use tokio::signal;

use tross::app;
use tross::billing::BillingStore;
use tross::config::Settings;
use tross::linkedin::client::{SessionDiagnostics, VoyagerClient};
use tross::linkedin::repository::ProfileRepository;
use tross::linkedin::session::FileSessionStore;
use tross::service::cache::ProfileCache;
use tross::service::profile::ProfileService;
use tross::state::AppState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("--help") | Some("help") => {
            print_help();
            return Ok(());
        }
        Some("--version") => {
            println!("tross {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Some("healthcheck") => {
            return healthcheck().await;
        }
        _ => {}
    }

    // ponytail: auto-load .env so plain `cargo run` works; never overrides
    // vars already set in the environment.
    dotenvy::dotenv().ok();

    let settings = Arc::new(Settings::from_env()?);
    tross::telemetry::init_logging(&settings);
    tracing::info!(settings = ?settings, "startup.beginning");

    let billing = if settings.billing_enabled {
        let store = BillingStore::connect(&settings.redis_url).await?;
        let seeded = store.seed_from_file(&settings.api_key_seed_path).await?;
        tracing::info!(seeded, "billing.accounts_seeded");
        store
    } else {
        BillingStore::Disabled
    };

    let session_store = Arc::new(FileSessionStore::new(&settings.session_state_path));
    let voyager = VoyagerClient::new(
        settings.clone(),
        session_store as Arc<dyn tross::linkedin::session::SessionStore>,
    );
    // A cold start without a live session is not fatal: the first request
    // retries authentication and /readyz reports the reason meanwhile.
    if let Err(err) = voyager.ensure_session(false).await {
        tracing::warn!(code = err.code(), message = %err, "startup.session_unavailable");
    }

    let cache = Arc::new(ProfileCache::new(
        settings.cache_ttl_seconds,
        settings.cache_max_entries,
        Some(&settings.cache_dir),
        settings.cache_persist,
    ));

    let client_arc: Arc<VoyagerClient> = Arc::new(voyager);
    let repository = Arc::new(ProfileRepository::<VoyagerClient>::new(
        client_arc.clone(),
        settings.clone(),
    ));
    let service = Arc::new(ProfileService::new(repository, cache));

    let state = AppState {
        settings: settings.clone(),
        service,
        voyager: client_arc.clone() as Arc<dyn SessionDiagnostics>,
        billing,
        rate_limiter: tross::api::middleware::make_rate_limiter(),
        started_at: Instant::now(),
    };

    let address = format!("{}:{}", settings.host, settings.port);
    let listener = tokio::net::TcpListener::bind(&address).await?;
    tracing::info!(environment = settings.environment.as_str(), auth_required = settings.auth_required(), address = %address, "startup.complete");

    // Use the future directly with graceful shutdown; no tracing-wait layer
    // needed, so no extra dependency for the drain signal.
    let app = app::build_app(state).into_make_service_with_connect_info::<std::net::SocketAddr>();

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    tracing::info!("shutdown.complete");
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c().await.expect("ctrl-c handler installed");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("SIGTERM handler installed")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

async fn healthcheck() -> anyhow::Result<()> {
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8000);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()?;
    let response = client
        .get(format!("http://127.0.0.1:{port}/healthz"))
        .send()
        .await;
    match response {
        Ok(response) if response.status().is_success() => Ok(()),
        Ok(response) => anyhow::bail!("healthz returned HTTP {}", response.status()),
        Err(err) => anyhow::bail!("healthz unreachable: {err}"),
    }
}

fn print_help() {
    println!(
        "tross {} — read API for public LinkedIn member profiles\n\n\
         USAGE:\n    tross [SUBCOMMAND]\n\n\
         SUBCOMMANDS:\n    healthcheck    probe /healthz of a running instance\n\
         \nWithout a subcommand the API server starts.",
        env!("CARGO_PKG_VERSION")
    );
}
