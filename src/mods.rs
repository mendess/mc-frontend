use crate::{Config, Error};
use askama::Template;
use axum::{
    extract::State,
    response::{AppendHeaders, Html, IntoResponse},
};
use regex::Regex;
use reqwest::{StatusCode, header::CONTENT_TYPE};
use serde::Deserialize;
use std::{
    io::{self, Cursor, Write},
    sync::{Arc, LazyLock},
};
use tokio_stream::{StreamExt as _, wrappers::ReadDirStream};
use zip::write::SimpleFileOptions;

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
    slug: String,
    version: String,
    mandatory: bool,
    client_side_only: bool,
}

const LATEST: &str = "latest";

mod mod_pack {
    use crate::{Error, mods::Mod};
    use futures::{StreamExt, TryStreamExt, io};
    use serde::{Deserialize, Serialize};
    use std::{
        collections::HashMap,
        sync::{LazyLock, Mutex},
        time::{Duration, SystemTime},
    };

    static MOD_INFO_CACHE: LazyLock<Mutex<HashMap<String, (SystemTime, Project)>>> =
        LazyLock::new(Default::default);

    #[derive(Debug, Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct ModPack {
        pub game: &'static str,
        pub format_version: u8,
        pub version_id: String,
        pub name: &'static str,
        pub summary: &'static str,
        pub files: Vec<Project>,
        pub dependencies: Dependencies,
    }

    impl ModPack {
        pub async fn new(
            mods: impl Iterator<Item = Mod>,
            neoforge_version: String,
        ) -> Result<Self, Error> {
            let client = &reqwest::Client::new();
            Ok(Self {
                game: "minecraft",
                format_version: 1,
                version_id: chrono::Utc::now()
                    .date_naive()
                    .format("%Y.%m.%d")
                    .to_string(),
                name: "large biomes pack",
                summary: "the modpack for the large biomes server",
                files: futures::stream::iter(mods)
                    .map(|m| async move {
                        let up_to_date = |ts: SystemTime, version: &str| match version {
                            super::LATEST => match SystemTime::now().duration_since(ts) {
                                Ok(d) => d < Duration::from_hours(72),
                                Err(_) => false,
                            },
                            _ => m.version == version,
                        };
                        if let Some((ts, project)) = MOD_INFO_CACHE.lock().unwrap().get(&m.slug)
                            && up_to_date(*ts, &project.version)
                        {
                            return Ok(project.clone());
                        }
                        let versions = async {
                            tracing::info!(mod = ?m, "getting versions");
                            client
                                .get(format!(
                                    "https://api.modrinth.com/v2/project/{}/version",
                                    m.slug
                                ))
                                .send()
                                .await?
                                .error_for_status()?
                                .json::<Vec<Version>>()
                                .await
                        }
                        .await
                        .map_err(io::Error::other)?;

                        #[derive(Deserialize)]
                        struct Version {
                            game_versions: Vec<String>,
                            loaders: Vec<String>,
                            version_number: String,
                            files: Vec<VersionFile>,
                        }

                        #[derive(Deserialize)]
                        struct VersionFile {
                            hashes: Hashes,
                            url: String,
                            filename: String,
                            size: usize,
                            primary: bool,
                        }

                        let Some(version) = versions.into_iter().find(|v| {
                            v.loaders.iter().any(|l| l == "neoforge")
                                && v.game_versions.iter().any(|l| l == "1.21.1")
                                && (m.client_side_only || v.version_number.contains(&m.version))
                        }) else {
                            tracing::error!(mod = ?m, "failed to find suitable version");
                            return Err(Error::Io(io::Error::other(format!(
                                "failed to find suitable version for mod: {}",
                                m.name
                            ))));
                        };

                        let file_idx = version
                            .files
                            .iter()
                            .position(|f| f.primary)
                            .unwrap_or_default();

                        let Some(file) = version.files.into_iter().nth(file_idx) else {
                            tracing::error!(mod = ?m, "failed to find suitable file");
                            return Err(Error::Io(io::Error::other(format!(
                                "failed to find suitable file for mod: {}",
                                m.name
                            ))));
                        };

                        let project = Project {
                            path: format!("mods/{}", file.filename),
                            hashes: file.hashes,
                            env: Env {
                                client: if m.mandatory { "required" } else { "optional" },
                            },
                            downloads: vec![file.url],
                            file_size: file.size,
                            version: m.version,
                        };
                        MOD_INFO_CACHE
                            .lock()
                            .unwrap()
                            .insert(m.slug.clone(), (SystemTime::now(), project.clone()));
                        Ok(project)
                    })
                    .buffered(usize::MAX)
                    .try_collect()
                    .await?,
                dependencies: Dependencies {
                    minecraft: "1.21.1".to_owned(),
                    neoforge: neoforge_version,
                },
            })
        }
    }

    #[derive(Debug, Clone, Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Project {
        path: String,
        hashes: Hashes,
        env: Env,
        downloads: Vec<String>,
        file_size: usize,
        #[serde(skip)]
        version: String,
    }

    #[derive(Debug, Serialize, Deserialize, Clone)]
    pub struct Hashes {
        sha512: String,
        sha1: String,
    }

    #[derive(Debug, Clone, Serialize)]
    pub struct Env {
        client: &'static str,
    }

    #[derive(Debug, Clone, Serialize)]
    pub struct Dependencies {
        minecraft: String,
        neoforge: String,
    }
}

pub async fn generate_mod_pack(config: State<Arc<Config>>) -> Result<impl IntoResponse, Error> {
    let server_mods = server_mods(&config).await?;
    let recommended_mods = recommended_mods().await?;
    let neoforge_version = neoforge_version(&config).await?;
    let modpack = mod_pack::ModPack::new(
        server_mods.into_iter().chain(recommended_mods),
        neoforge_version,
    )
    .await?;
    let json_data = serde_json::to_vec_pretty(&modpack).unwrap();

    // 2. Create a buffer in memory
    let mut buffer = Vec::new();

    // 3. Scope the ZipWriter so it returns ownership of the buffer when dropped/finished
    {
        let mut zip = zip::ZipWriter::new(Cursor::new(&mut buffer));

        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);

        // Create the file entry
        zip.start_file("modrinth.index.json", options)
            .map_err(io::Error::other)?;
        zip.write_all(&json_data)?;

        zip.start_file("overrides/servers.dat", options)
            .map_err(io::Error::other)?;
        zip.write_all(
            tokio::fs::read_to_string("./assets/servers.dat")
                .await?
                .as_bytes(),
        )?;

        // Explicitly finish to write the central directory to the buffer
        zip.finish().map_err(io::Error::other)?;
    }

    tracing::info!(len = buffer.len(), "serving modpack");

    Ok((
        StatusCode::OK,
        AppendHeaders([(CONTENT_TYPE, "application/x-modrith-modpack+zip")]),
        buffer,
    ))
}

pub async fn server_mods(config: &Config) -> Result<Vec<Mod>, Error> {
    const MANDATORY_MODS: &[&str] = &["create", "copycats", "voicechat"];
    const SERVER_SUPPORTED_MODS: &[&str] = &["DistantHorizons", "jei", "no-chat-reports"];

    Ok(
        ReadDirStream::new(tokio::fs::read_dir(config.server_dir.join("mods")).await?)
            .filter_map(|s| {
                let p = s.ok()?.path();
                let p = p.file_name().unwrap();
                let m = p.to_str().unwrap();
                fn parse_version(sufix: &str) -> Option<&str> {
                    sufix.strip_suffix(".jar")?.strip_prefix("-")
                }
                let (name, version, mandatory): (&str, &str, bool) = MANDATORY_MODS
                    .iter()
                    .find_map(|mod_name| {
                        m.strip_prefix(*mod_name)
                            .and_then(|sufix| Some((*mod_name, parse_version(sufix)?, true)))
                    })
                    .or_else(|| {
                        SERVER_SUPPORTED_MODS.iter().find_map(|mod_name| {
                            m.strip_prefix(*mod_name)
                                .and_then(|sufix| Some((*mod_name, parse_version(sufix)?, false)))
                        })
                    })?;
                let name = match name {
                    "voicechat" => "simple-voice-chat",
                    n => n,
                };
                Some(Mod {
                    name: name.to_owned(),
                    slug: name.to_owned(),
                    version: version.to_owned(),
                    mandatory,
                    client_side_only: false,
                })
            })
            .collect()
            .await,
    )
}

pub async fn recommended_mods() -> Result<Vec<Mod>, Error> {
    static CLIENT_SIDE_MODS: LazyLock<Vec<Mod>> = LazyLock::new(|| {
        [
            "sodium",
            "ferrite-core",
            "entityculling",
            "lithium",
            "immediatelyfast",
            "sodium-extra",
            "dynamic-fps",
            "modernfix",
            "mouse-tweaks",
        ]
        .map(|name| Mod {
            name: name.into(),
            slug: name.into(),
            version: LATEST.into(),
            mandatory: false,
            client_side_only: true,
        })
        .to_vec()
    });

    Ok(CLIENT_SIDE_MODS.clone())
}

async fn neoforge_version(config: &Config) -> Result<String, Error> {
    static REGEX: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"libraries/net/neoforged/neoforge/(.*)/unix_args.txt"#).unwrap()
    });
    let x = tokio::fs::read_to_string(config.server_dir.join("run.sh")).await?;
    let captures = REGEX.captures(&x).unwrap();
    Ok(captures.get(1).unwrap().as_str().to_string())
}

pub async fn get_mods(config: State<Arc<Config>>) -> Result<impl IntoResponse, Error> {
    let mut server_mods = server_mods(&config).await?;
    let recommended_mods = recommended_mods().await?;
    let mut mods = Mods {
        neoforge_version: neoforge_version(&config).await?,
        required: server_mods.extract_if(.., |m| m.mandatory).collect(),
        recommended: server_mods,
        client_side: recommended_mods,
    };
    mods.required.sort();
    mods.recommended.sort();
    Ok(Html(mods.render()?))
}
