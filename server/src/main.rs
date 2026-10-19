use eink_bridge::app::AppState;
use eink_bridge::config::AppConfig;
use std::net::SocketAddr;
use std::time::Duration;
use tracing_subscriber::EnvFilter;

/// How long open connections get to finish once the stop signal arrives before the
/// server stops regardless. The write queue is drained either way.
const STOP_GRACE: Duration = Duration::from_secs(5);

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
    let state = AppState::with_config(
        config.server.state_dir.clone(),
        config.server.long_poll_seconds,
    );
    let app = eink_bridge::app::build_app(state.clone());
    let addr = config.bind_addr();

    // Retention is independent of expiry: expiry ends a review, retention eventually
    // removes the directory of a finished one. Run once at startup so a machine that
    // was off for a week catches up, then daily.
    let sweeper = state.clone();
    tokio::spawn(async move {
        loop {
            sweeper.sweep_retention().await;
            tokio::time::sleep(Duration::from_secs(24 * 60 * 60)).await;
        }
    });
    state.refresh_dir_gauge().await;

    let timeout = Duration::from_secs(config.server.session_timeout_minutes * 60);
    let expirer = state.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        loop {
            interval.tick().await;
            let expired = expirer.expire_stale(timeout).await;
            if expired > 0 {
                tracing::info!(count = expired, "expired stale sessions");
            }
        }
    });

    // A port already in use is the first thing that goes wrong after a reboot, and a
    // panic trace in the journal is no way to learn it. Say what happened and stop.
    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("eink-serve: cannot listen on {addr}: {error}");
            tracing::error!(%addr, %error, "cannot listen");
            std::process::exit(1);
        }
    };

    // SIGTERM is how systemd stops the service. Dropping the process on it used to drop
    // whatever the write queue still held, which was the last change a client had just
    // been told was safe.
    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    tokio::spawn(async move {
        stop_signal().await;
        tracing::info!("stop requested; finishing open requests");
        let _ = stop_tx.send(true);
    });
    let mut graceful = stop_rx.clone();
    let server = axum::serve(
        listener,
        // Each request learns its peer address, which is what tells a CLI on this machine
        // from a client on the LAN.
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        let _ = graceful.wait_for(|stop| *stop).await;
    });

    tracing::info!(%addr, "eink-serve listening");
    let mut deadline = stop_rx;
    tokio::select! {
        result = server => {
            if let Err(error) = result {
                eprintln!("eink-serve: server stopped: {error}");
                tracing::error!(%error, "server stopped");
                std::process::exit(1);
            }
        }
        _ = async {
            let _ = deadline.wait_for(|stop| *stop).await;
            tokio::time::sleep(STOP_GRACE).await;
        } => {
            tracing::warn!("connections still open after {STOP_GRACE:?}; stopping anyway");
        }
    }
    match state.flush_writes().await {
        Ok(()) => tracing::info!("eink-serve stopped; every queued write is on disk"),
        Err(error) => {
            eprintln!("eink-serve: a session write failed while stopping: {error}");
            tracing::error!(error, "a session write failed while stopping");
            std::process::exit(1);
        }
    }
}

/// Resolves on ctrl-c or SIGTERM.
async fn stop_signal() {
    let ctrl_c = tokio::signal::ctrl_c();
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("a SIGTERM handler can be installed");
        tokio::select! {
            _ = ctrl_c => {}
            _ = terminate.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = ctrl_c.await;
    }
}
