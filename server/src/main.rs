use eink_bridge::app::AppState;
use eink_bridge::config::AppConfig;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::from_default_env().add_directive("eink_bridge=info".parse().unwrap()),
        )
        .init();

    let config = AppConfig::load();
    let state = AppState::new(config.server.state_dir.clone());
    let app = eink_bridge::app::build_app(state);
    let addr = config.bind_addr();

    tracing::info!("eink-serve listening on {addr}");

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
