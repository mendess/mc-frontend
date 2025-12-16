use askama::Template;
use axum::{
    Router,
    extract::State,
    http::StatusCode,
    response::{Html, IntoResponse, Redirect},
    routing::get,
};
use serde::Deserialize;
use std::{fs::File, io, path::PathBuf, sync::Arc};
use tower_http::services::ServeDir;

#[derive(Deserialize)]
struct Config {
    backups_dir: PathBuf,
}

fn get_configuration() -> Result<Config, config::ConfigError> {
    config::Config::builder()
        .add_source(config::File::with_name("config/conf").required(false))
        .build()
        .and_then(config::Config::try_deserialize)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = get_configuration()?;
    let router = Router::new()
        .route("/", get(index))
        .route("/deaths", get(deaths))
        .route("/super-secret-map", get(Redirect::to("/super-secret-map/")))
        .nest_service(
            "/super-secret-map/",
            ServeDir::new(config.backups_dir.join("map/web-export"))
                .append_index_html_on_directories(true),
        )
        .with_state(Arc::new(config));

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
#[template(path = "deaths/index.html")]
struct Deaths {
    deaths: Vec<(String, String, String)>,
}

#[derive(Debug, Template)]
#[template(path = "index.html")]
struct Index;

async fn index() -> Result<impl IntoResponse, Error> {
    Ok(Html(Index.render()?))
}

async fn deaths(config: State<Arc<Config>>) -> Result<impl IntoResponse, Error> {
    let deaths = serde_json::from_reader(File::open(config.backups_dir.join("deaths.json"))?)?;
    Ok(Html(Deaths { deaths }.render()?))
}
