use std::sync::Arc;

use axum::{
    routing::{delete, get},
    Router,
};
use sentry_tower::{NewSentryLayer, SentryHttpLayer};
use std::env;
use tower::ServiceBuilder;
use tower_http::cors::CorsLayer;
use yral_daily_streaks::api::handlers::*;
use yral_daily_streaks::config::AppConfig;
use yral_daily_streaks::middleware::create_before_send;
use yral_daily_streaks::state::AppState;
use yral_daily_streaks::utils::error::*;
use yral_daily_streaks::{get_swagger, get_swagger_root, openapi_spec};

fn setup_sentry_subscriber() {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    // Configure sentry_tracing to only capture errors as events
    // and only warnings as breadcrumbs (not debug/info)
    let sentry_layer = sentry_tracing::layer().event_filter(|metadata| {
        use sentry_tracing::EventFilter;
        match *metadata.level() {
            tracing::Level::ERROR => EventFilter::Event,
            tracing::Level::WARN => EventFilter::Breadcrumb,
            _ => EventFilter::Ignore, // Ignore DEBUG, INFO, TRACE
        }
    });

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,hyper=warn,reqwest=warn,tower_http=warn".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .with(sentry_layer)
        .init();
}

async fn main_impl() -> Result<()> {
    let conf = AppConfig::load()?;

    let state = Arc::new(AppState::new(&conf).await?);

    let sentry_tower_layer = ServiceBuilder::new()
        .layer(NewSentryLayer::new_from_top())
        .layer(SentryHttpLayer::with_transaction());

    // Build the application router with all routes defined here
    let app = Router::new()
        // API routes
        .route("/streak/{user_prinicipal}", get(checkin))
        .route("/streak/{user_prinicipal}", delete(delete_streak))
        // OpenAPI/Swagger UI routes
        .route("/explorer/{*tail}", get(get_swagger))
        .route("/explorer/", get(get_swagger_root))
        .route("/api-doc/openapi.json", get(openapi_spec))
        .route("/healthz", get(healthz))
        .layer(CorsLayer::permissive())
        .layer(sentry_tower_layer)
        // Add shared state
        .with_state(state.clone());

    let listener = tokio::net::TcpListener::bind(conf.bind_address)
        .await
        .map_err(Error::IO)?;

    log::info!("Server starting on {}", conf.bind_address);

    axum::serve(listener, app).await.map_err(Error::IO)?;

    Ok(())
}

fn main() {
    let _guard = sentry::init((
        "https://c7db7b42e715cae503160554ea5e0713@sentry.naitik.yral.com/5",
        sentry::ClientOptions {
            release: sentry::release_name!(),
            environment: Some(
                env::var("APP_ENV")
                    .unwrap_or_else(|_| "production".to_string())
                    .into(),
            ),
            server_name: Some(
                hostname::get()
                    .ok()
                    .and_then(|h| h.into_string().ok())
                    .unwrap_or_else(|| "unknown".to_string())
                    .into(),
            ),
            send_default_pii: true,
            traces_sample_rate: 0.01, //lower sampling for lower data accumulation.
            attach_stacktrace: true,
            auto_session_tracking: true,
            max_breadcrumbs: 100, // Store more breadcrumbs for better context
            before_send: Some(create_before_send()),
            ..Default::default()
        },
    ));

    setup_sentry_subscriber();

    log::info!("Sentry initialized successfully");

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            main_impl().await.unwrap();
        });
}
