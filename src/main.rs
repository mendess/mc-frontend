mod favicon;
mod games;

use askama::Template;
use axum::{
    Router,
    response::{Html, Redirect},
    routing::get,
};
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

#[derive(askama::Template)]
#[template(path = "index.html")]
struct Index;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();
    let redirect = |s| axum::routing::any(Redirect::to(s));
    let router = Router::new()
        .route("/", get(async || Html(Index.render().unwrap())))
        .route("/favicon.ico", get(favicon::serve))
        .route("/games/minecraft", redirect("/games/minecraft/"))
        .nest("/games/minecraft/", games::minecraft()?)
        .nest("/games/bg3/", games::bg3()?);
    println!("serving at http://localhost:50002");
    axum::serve(
        tokio::net::TcpListener::bind("0.0.0.0:50002").await?,
        router,
    )
    .await?;
    Ok(())
}
