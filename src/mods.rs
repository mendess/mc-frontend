use crate::{Config, Error};
use askama::Template;
use axum::{
    extract::State,
    response::{Html, IntoResponse},
};
use futures::TryStreamExt as _;
use regex::Regex;
use serde::Deserialize;
use std::sync::{Arc, LazyLock};
use tokio_stream::wrappers::ReadDirStream;

#[derive(Debug, Default, Template)]
#[template(path = "mods/index.html")]
pub struct Mods {
    neoforge_version: String,
    required: Vec<Mod>,
    recommended: Vec<Mod>,
    client_side: Vec<Mod>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct Mod {
    name: String,
    version: String,
}

pub async fn get_mods(config: State<Arc<Config>>) -> Result<impl IntoResponse, Error> {
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
    mods.neoforge_version = {
        static REGEX: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r#"libraries/net/neoforged/neoforge/(.*)/unix_args.txt"#).unwrap()
        });
        let x = std::fs::read_to_string(config.server_dir.join("run.sh"))?;
        let captures = REGEX.captures(&x).unwrap();
        captures.get(1).unwrap().as_str().to_string()
    };
    mods.required.sort();
    mods.recommended.sort();
    mods.client_side = CLIENT_SIDE_MODS.clone();
    Ok(Html(mods.render()?))
}

enum Either<T> {
    Left(T),
    Right(T),
}
