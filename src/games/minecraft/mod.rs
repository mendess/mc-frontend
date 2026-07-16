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

pub fn router() -> anyhow::Result<Router<()>> {
    let config = get_configuration()?;
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
            ("/maps/overworld/night/", "overworld-night"),
            ("/maps/nether/", "nether"),
            ("/super-secret-map-nether-mid/", "nether-mid"),
        ],
    );
    Ok(router.with_state(Arc::new(config)))
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
#[template(path = "minecraft/index.html")]
struct Index;

async fn index() -> Result<impl IntoResponse, Error> {
    Ok(Html(Index.render()?))
}

#[derive(Debug, Template)]
#[template(path = "minecraft/maps/index.html")]
struct Maps;

async fn maps() -> Result<impl IntoResponse, Error> {
    Ok(Html(Maps.render()?))
}
