mod deaths;
mod logs;
mod mods;

use askama::Template;
use axum::{
    Router,
    http::StatusCode,
    response::{Html, IntoResponse, Redirect},
    routing::get,
};
use serde::Deserialize;
use std::{
    io,
    path::{Path, PathBuf},
    sync::Arc,
};
use tower_http::services::ServeDir;
use tracing::level_filters::LevelFilter;
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt as _, util::SubscriberInitExt as _};

#[derive(Deserialize)]
struct Config {
    backups_dir: PathBuf,
    server_dir: PathBuf,
}

fn get_configuration() -> Result<Config, config::ConfigError> {
    config::Config::builder()
        .add_source(config::File::with_name("config/conf").required(false))
        .build()
        .and_then(config::Config::try_deserialize)
}

fn add_map_routes(
    mut router: Router<Arc<Config>>,
    config: &Config,
    maps: &[(&str, &str)],
) -> Router<Arc<Config>> {
    let base = Path::new("map/web-export");
    for (map, dir) in maps {
        router = router
            .route(map.trim_end_matches("/"), get(Redirect::to(map)))
            .nest_service(
                map,
                ServeDir::new(config.backups_dir.join(base).join(dir))
                    .append_index_html_on_directories(true),
            );
    }
    router
}

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
    let config = get_configuration()?;
    init_tracing();
    let router = Router::new()
        .route("/", get(index))
        .route("/deaths", get(deaths::deaths))
        .route("/mods", get(mods::get_mods))
        .route("/mods/large-biomes.mrpack", get(mods::generate_mod_pack))
        .route("/maps", get(maps))
        .route("/super-secret-map/", get(Redirect::to("/maps/overworld/")))
        .route("/super-secret-map", get(Redirect::to("/maps/overworld/")))
        .route(
            "/super-secret-map-nether/",
            get(Redirect::to("/maps/nether/")),
        )
        .route(
            "/super-secret-map-nether",
            get(Redirect::to("/maps/nether/")),
        );
    let router = add_map_routes(
        router,
        &config,
        &[
            ("/maps/overworld/", "overworld-day"),
            ("/maps/nether/", "nether"),
            ("/super-secret-map-nether-mid/", "nether-mid"),
        ],
    );
    let router = router.with_state(Arc::new(config));

    println!("serving at http://localhost:50002");
    axum::serve(
        tokio::net::TcpListener::bind("0.0.0.0:50002").await?,
        router,
    )
    .await?;
    Ok(())
}

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error("io: {0}")]
    Io(#[from] io::Error),
    #[error("serde: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("rendering: {0}")]
    Rendering(#[from] askama::Error),
}

impl IntoResponse for Error {
    fn into_response(self) -> axum::response::Response {
        (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()).into_response()
    }
}

#[derive(Debug, Template)]
#[template(path = "index.html")]
struct Index;

async fn index() -> Result<impl IntoResponse, Error> {
    Ok(Html(Index.render()?))
}

#[derive(Debug, Template)]
#[template(path = "maps/index.html")]
struct Maps;

async fn maps() -> Result<impl IntoResponse, Error> {
    Ok(Html(Maps.render()?))
}
