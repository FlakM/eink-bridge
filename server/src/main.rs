use eink_bridge::app::AppState;
use eink_bridge::config::AppConfig;
use std::time::Duration;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::from_default_env().add_directive("eink_bridge=info".parse().unwrap()),
        )
        .init();

    eink_bridge::metrics::init();

    let config = AppConfig::load();
    tracing::info!(
        state_dir = %config.server.state_dir.display(),
        session_timeout_min = config.server.session_timeout_minutes,
        long_poll_sec = config.server.long_poll_seconds,
        "config loaded"
    );
    let state = AppState::new(config.server.state_dir.clone());
    let app = eink_bridge::app::build_app(state.clone());
    let addr = config.bind_addr();

    let timeout = Duration::from_secs(config.server.session_timeout_minutes * 60);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        loop {
            interval.tick().await;
            let expired = state.sessions.write().await.expire_stale(timeout);
            if expired > 0 {
                tracing::info!(count = expired, "expired stale sessions");
            }
        }
    });

    tracing::info!(%addr, "eink-serve listening");

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
