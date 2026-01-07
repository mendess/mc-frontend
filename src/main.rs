use askama::Template;
use axum::{
    Router,
    extract::State,
    http::StatusCode,
    response::{Html, IntoResponse, Redirect},
    routing::get,
};
use futures::TryStreamExt;
use regex::Regex;
use serde::Deserialize;
use std::{
    fs::File,
    io,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock},
};
use tokio_stream::wrappers::ReadDirStream;
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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = get_configuration()?;
    let router = Router::new()
        .route("/", get(index))
        .route("/deaths", get(deaths))
        .route("/mods", get(get_mods))
        .route("/maps", get(maps));
    let router = add_map_routes(
        router,
        &config,
        &[
            ("/super-secret-map/", "overworld-day"),
            ("/super-secret-map-nether/", "nether"),
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

#[derive(Debug, Template)]
#[template(path = "maps/index.html")]
struct Maps;

async fn maps() -> Result<impl IntoResponse, Error> {
    Ok(Html(Maps.render()?))
}

async fn deaths(config: State<Arc<Config>>) -> Result<impl IntoResponse, Error> {
    let deaths = serde_json::from_reader(File::open(config.backups_dir.join("deaths.json"))?)?;
    Ok(Html(Deaths { deaths }.render()?))
}

#[derive(Debug, Default, Template)]
#[template(path = "mods/index.html")]
struct Mods {
    required: Vec<Mod>,
    recommended: Vec<Mod>,
    client_side: Vec<Mod>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
struct Mod {
    name: String,
    version: String,
}

async fn get_mods(config: State<Arc<Config>>) -> Result<impl IntoResponse, Error> {
    static MOD_PARSER: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?<name>[a-zA-Z\-]+)-(?<version>.*)\.jar").unwrap());
    let mods_dir = ReadDirStream::new(tokio::fs::read_dir(config.server_dir.join("mods")).await?)
        .try_collect::<Vec<_>>()
        .await?;

    const SERVER_ONLY_MODS: &[&str] = &["Prometheus-Exporter", "servercore"];

    const MANDATORY_MODS: &[&str] = &["create", "copycats", "simple-voice-chat"];

    static CLIENT_SIDE_MODS: LazyLock<Vec<Mod>> = LazyLock::new(|| {
        vec![
            Mod {
                name: "mouse-tweaks".into(),
                version: "any".into(),
            },
            Mod {
                name: "sodium".into(),
                version: "any".into(),
            },
            Mod {
                name: "lithium".into(),
                version: "any".into(),
            },
        ]
    });

    let mut mods = mods_dir
        .into_iter()
        .filter_map(|s| {
            let p = s.path();
            let p = p.file_name().unwrap();
            let m = p.to_str().unwrap();
            let captures = MOD_PARSER.captures(m).unwrap();
            let name = captures.name("name").unwrap();
            let version = captures.name("version").unwrap();
            if SERVER_ONLY_MODS.contains(&name.as_str()) {
                return None;
            }
            let m = Mod {
                name: match name.as_str() {
                    "voicechat-neoforge" => "simple-voice-chat",
                    "no-chat-reports-NeoForge" => "no-chat-reports",
                    n => n,
                }
                .to_owned(),
                version: version.as_str().to_owned(),
            };
            if MANDATORY_MODS.contains(&m.name.as_str()) {
                Some(Either::Left(m))
            } else {
                Some(Either::Right(m))
            }
        })
        .fold(Mods::default(), |mut acc, m| {
            match m {
                Either::Left(m) => acc.required.push(m),
                Either::Right(m) => acc.recommended.push(m),
            }
            acc
        });
    mods.required.sort();
    mods.recommended.sort();
    mods.client_side = CLIENT_SIDE_MODS.clone();
    Ok(Html(mods.render()?))
}

enum Either<T> {
    Left(T),
    Right(T),
}
