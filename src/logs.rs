use crate::{Config, Error};
use chrono::NaiveDateTime;
use flate2::bufread::GzDecoder;
use futures::{Stream, StreamExt};
use serde::Deserialize;
use std::{
    collections::HashMap,
    fs::File,
    io::{self, Read},
    path::PathBuf,
    sync::{Arc, LazyLock},
};
use tokio::sync::Mutex;

#[derive(Debug, Deserialize, Clone)]
pub struct WhitelistEntry {
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct LogLine {
    pub player: String,
    pub timestamp: NaiveDateTime,
    pub message: String,
}

#[tracing::instrument(skip_all)]
fn parse_log(log: &str, whitelist: &[WhitelistEntry]) -> Vec<LogLine> {
    tracing::info!("parsing log");
    let mut death_records = Vec::new();
    for line in log.lines() {
        // Split by the standard Minecraft log separator "]: "
        let parts: Vec<&str> = line.splitn(2, "]: ").collect();
        if parts.len() == 2 {
            let meta_info = parts[0];
            let content = parts[1];

            // Extract Timestamp
            let timestamp_parts: Vec<&str> = meta_info.split_whitespace().collect();
            let timestamp = if timestamp_parts.len() >= 2 {
                format!("{} {}", timestamp_parts[0], timestamp_parts[1]).replace(['[', ']'], "")
            } else {
                "unknown".to_string()
            };
            let timestamp = match NaiveDateTime::parse_from_str(&timestamp, "%d%b%Y %H:%M:%S%.f") {
                Ok(d) => d,
                Err(e) => {
                    tracing::error!(error = ?e, timestamp, "failed to parse log timestamp");
                    continue;
                }
            };

            // Check against known players
            for WhitelistEntry { name } in whitelist {
                let player_prefix = format!("{name} ");
                if content.starts_with(&player_prefix) {
                    let message = content[name.len()..].trim().to_string();
                    death_records.push(LogLine {
                        timestamp,
                        player: name.clone(),
                        message,
                    });
                    break;
                }
            }
        }
    }
    death_records
}

/// The main parsing function
pub async fn parse_logs(config: &Config) -> Result<impl Stream<Item = LogLine>, Error> {
    static LOG_CACHE: LazyLock<Mutex<HashMap<PathBuf, Vec<LogLine>>>> =
        LazyLock::new(Default::default);

    let whitelist_path = config.server_dir.join("whitelist.json");
    tracing::debug!(?whitelist_path, "opening whitelist");
    let whitelist: Arc<Vec<WhitelistEntry>> =
        Arc::new(serde_json::from_reader(File::open(whitelist_path)?)?);

    let logs_dir = config.server_dir.join("logs");
    tracing::debug!(?logs_dir, "globing logs");

    // Collect and sort files similar to glob.glob() + sort()
    let mut files: Vec<std::path::PathBuf> = glob::glob(&format!("{}/*.gz", logs_dir.display()))
        .map_err(io::Error::other)?
        .collect::<Result<_, _>>()
        .map_err(io::Error::other)?;
    files.sort();
    files.pop(); // this one is the same as lattest.log so we don't want to cache it
    let death_record_futures = {
        let whitelist = whitelist.clone();
        futures::stream::iter(files)
            .filter(|p| std::future::ready(!p.to_string_lossy().contains("debug")))
            .map(move |file_path| {
                let whitelist = whitelist.clone();
                async move {
                    if let Some(cached) = LOG_CACHE.lock().await.get(&file_path) {
                        return cached.clone().into_iter();
                    };

                    let whitelist = whitelist.clone();
                    let read_result = tokio::task::spawn_blocking(move || {
                        tracing::error_span!("LOG PARSING", ?file_path).in_scope(|| {
                            let file = match File::open(&file_path) {
                                Ok(f) => f,
                                Err(e) => {
                                    tracing::error!(?file_path, error = ?e, "failed to read log");
                                    return None;
                                }
                            };
                            let mut gz = GzDecoder::new(std::io::BufReader::new(file));
                            let mut contents = String::new();

                            // Decompress and read to string (handles UTF-8)
                            match gz.read_to_string(&mut contents) {
                                Ok(_) => Some((file_path, parse_log(&contents, &whitelist))),
                                Err(e) => {
                                    tracing::error!(error = ?e, "failed to parse log");
                                    None
                                }
                            }
                        })
                    })
                    .await
                    .unwrap();
                    if let Some((file_path, records)) = read_result {
                        LOG_CACHE.lock().await.insert(file_path, records.clone());
                        records.into_iter()
                    } else {
                        vec![].into_iter()
                    }
                }
            })
            .buffered(usize::MAX)
    };

    Ok(death_record_futures
        .chain(futures::stream::iter([{
            let latest_log_path = logs_dir.join("latest.log");
            tracing::debug!(?latest_log_path, "reading log");
            match std::fs::read_to_string(latest_log_path) {
                Ok(contents) => parse_log(&contents, &whitelist).into_iter(),
                Err(e) => {
                    tracing::error!(error = ?e, "failed to read lattest log");
                    vec![].into_iter()
                }
            }
        }]))
        .flat_map(futures::stream::iter))
}
