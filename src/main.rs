mod games;

use axum::{Router, response::Redirect};
use tracing::level_filters::LevelFilter;
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt as _, util::SubscriberInitExt as _};

fn init_tracing() {
    tracing_subscriber::registry()
        .with(
            EnvFilter::builder()
                .with_default_directive(LevelFilter::INFO.into())
                .from_env_lossy(),
        )
        .with(fmt::layer().pretty())
        .init();
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();
    let redirect = |s| axum::routing::any(Redirect::to(s));
    let router = Router::new()
        .route("/", redirect("/games/minecraft/"))
        .route("/games/minecraft", redirect("/games/minecraft/"))
        .nest("/games/minecraft/", games::minecraft()?);
    println!("serving at http://localhost:50002");
    axum::serve(
        tokio::net::TcpListener::bind("0.0.0.0:50002").await?,
        router,
    )
    .await?;
    Ok(())
}
