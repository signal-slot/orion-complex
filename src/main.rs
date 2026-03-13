use std::sync::Arc;

use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use webauthn_rs::WebauthnBuilder;

use orion_complex::AppState;
use orion_complex::auth::AuthConfig;
use orion_complex::config;
use orion_complex::db;
use orion_complex::vm::VmProvider;
use orion_complex::vm::libvirt::LibvirtProvider;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "orion_complex=info,tower_http=info".parse().unwrap()),
        )
        .init();

    let config = config::Config::from_env();
    let auth_config = AuthConfig::from_env();

    tracing::info!("starting orion-complex on {}", config.listen_addr);

    let pool = db::init_pool(&config).await;

    let vm_provider: Arc<dyn VmProvider> = Arc::new(LibvirtProvider::new(&config.libvirt_uri, &config.data_dir));

    let rp_origin = url::Url::parse(&auth_config.webauthn_rp_origin)
        .expect("invalid WEBAUTHN_RP_ORIGIN");
    let webauthn = Arc::new(
        WebauthnBuilder::new(&auth_config.webauthn_rp_id, &rp_origin)
            .expect("failed to build webauthn")
            .rp_name(&auth_config.webauthn_rp_name)
            .build()
            .expect("failed to build webauthn"),
    );

    let state = AppState {
        db: pool,
        auth_config,
        http_client: reqwest::Client::new(),
        vm_provider,
        webauthn,
    };

    // Reconcile environments stuck in transient states from a previous crash
    orion_complex::background::reconcile_stuck_environments(&state.db).await;

    // Shutdown signal coordination
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    orion_complex::background::spawn_reaper(
        state.db.clone(),
        state.vm_provider.clone(),
        shutdown_rx.clone(),
        config.reaper_interval_secs,
    );
    orion_complex::background::spawn_heartbeat_checker(
        state.db.clone(),
        shutdown_rx,
        config.heartbeat_check_interval_secs,
        config.heartbeat_stale_threshold_secs,
    );

    // CORS configuration
    let cors = if let Some(ref origins) = config.cors_origins {
        let allowed: Vec<_> = origins
            .iter()
            .filter_map(|o| o.parse().ok())
            .collect();
        CorsLayer::new()
            .allow_origin(allowed)
            .allow_methods([
                axum::http::Method::GET,
                axum::http::Method::POST,
                axum::http::Method::PUT,
                axum::http::Method::DELETE,
            ])
            .allow_headers([axum::http::header::AUTHORIZATION, axum::http::header::CONTENT_TYPE])
    } else {
        CorsLayer::permissive()
    };

    let app = orion_complex::api::router()
        .with_state(state.clone())
        .layer(axum::Extension(state))
        .layer(cors)
        .layer(TraceLayer::new_for_http());

    let listener = tokio::net::TcpListener::bind(config.listen_addr)
        .await
        .expect("failed to bind listener");

    tracing::info!("listening on {}", config.listen_addr);

    let shutdown_signal = async move {
        tokio::signal::ctrl_c().await.ok();
        tracing::info!("received shutdown signal, draining connections...");
        let _ = shutdown_tx.send(true);
    };

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal)
        .await
        .expect("server error");

    tracing::info!("server shut down cleanly");
}
