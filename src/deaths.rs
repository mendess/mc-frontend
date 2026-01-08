use crate::{Config, Error};
use askama::Template;
use axum::{
    extract::{Query, State},
    response::{Html, IntoResponse},
};
use chrono::{Datelike, Days, NaiveDateTime};
use flate2::read::GzDecoder;
use futures::{StreamExt, stream::FuturesOrdered};
use glob::glob;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::File,
    sync::{Arc, LazyLock},
};
use std::{
    io::{self, Read},
    path::PathBuf,
};
use tokio::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeathRecord {
    #[serde(skip)]
    pub timestamp: NaiveDateTime,
    pub player: String,
    pub cause: String,
}

// Constants equivalent to your Python configuration
const IGNORED_TIMESTAMPS: &[&str] = &[
    "06Jun2025 15:42:05.682",
    "08Jun2025 18:40:17.329",
    "05Jan2026 01:49:16.370",
];

const IGNORED_MESSAGES: &[&str] = &[
    "has the following entity data",
    "joined the game",
    "left the game",
    "lost connection",
    "has made the advancement",
    "has reached the goal",
    "has completed the challenge",
    "[Server]",
    "<",
    "moved too quickly!",
    "moved wrongly!",
    "logged in with entity id",
    "UUID of player",
    "displaying particle",
    "issued server command",
    "teleported to",
];

/// Checks if the message content contains any of the ignored patterns
fn is_ignored_message(content: &str) -> bool {
    IGNORED_MESSAGES.iter().any(|&msg| content.contains(msg))
}

#[derive(Debug, Deserialize, Clone)]
struct WhitelistEntry {
    name: String,
}

#[tracing::instrument(skip_all)]
fn parse_log(log: &str, whitelist: &[WhitelistEntry]) -> Vec<DeathRecord> {
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

            if IGNORED_TIMESTAMPS.contains(&timestamp.as_str()) {
                continue;
            }
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
                if content.starts_with(&player_prefix) && !is_ignored_message(content) {
                    let cause = content[name.len()..].trim().to_string();
                    death_records.push(DeathRecord {
                        timestamp,
                        player: name.clone(),
                        cause,
                    });
                    break;
                }
            }
        }
    }
    death_records
}

/// The main parsing function
pub async fn parse_logs(config: &Config) -> Result<Vec<DeathRecord>, Error> {
    static LOG_CACHE: LazyLock<Mutex<HashMap<PathBuf, Vec<DeathRecord>>>> =
        LazyLock::new(Default::default);

    let whitelist_path = config.server_dir.join("whitelist.json");
    tracing::debug!(?whitelist_path, "opening whitelist");
    let whitelist: Vec<WhitelistEntry> = serde_json::from_reader(File::open(whitelist_path)?)?;

    let mut death_records: Vec<DeathRecord> = Vec::new();

    let logs_dir = config.server_dir.join("logs");
    tracing::debug!(?logs_dir, "globing logs");

    // Collect and sort files similar to glob.glob() + sort()
    let mut files: Vec<std::path::PathBuf> = glob(&format!("{}/*.gz", logs_dir.display()))
        .map_err(io::Error::other)?
        .collect::<Result<_, _>>()
        .map_err(io::Error::other)?;
    files.sort();
    let mut death_record_futures = files
        .into_iter()
        .filter(|p| !p.to_string_lossy().contains("debug"))
        .map(|file_path| async {
            if let Some(cached) = LOG_CACHE.lock().await.get(&file_path) {
                return cached.clone();
            };

            let whitelist = whitelist.clone();
            let (file_path, records) = tokio::task::spawn_blocking(move || {
                tracing::error_span!("parse log", ?file_path).in_scope(|| {
                    tracing::debug!("reading log");
                    let file = match File::open(&file_path) {
                        Ok(f) => f,
                        Err(e) => {
                            tracing::error!(?file_path, error = ?e, "failed to read log");
                            return (file_path, vec![]);
                        }
                    };
                    let mut gz = GzDecoder::new(file);
                    let mut contents = String::new();

                    // Decompress and read to string (handles UTF-8)
                    if gz.read_to_string(&mut contents).is_ok() {
                        (file_path, parse_log(&contents, &whitelist))
                    } else {
                        (file_path, vec![])
                    }
                })
            })
            .await
            .unwrap();
            LOG_CACHE.lock().await.insert(file_path, records.clone());
            records
        })
        .collect::<FuturesOrdered<_>>();

    while let Some(deaths) = death_record_futures.next().await {
        death_records.extend(deaths)
    }

    let latest_log_path = logs_dir.join("latest.log");
    tracing::debug!(?latest_log_path, "reading log");
    death_records.extend(parse_log(
        &std::fs::read_to_string(latest_log_path)?,
        &whitelist,
    ));

    Ok(death_records)
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct Chart {
    labels: Vec<String>,
    values: Vec<u64>,
}

impl Chart {
    fn new(data: Vec<(String, u64)>) -> Self {
        let (labels, values) = data.into_iter().collect();
        Self { labels, values }
    }

    fn inc(&mut self, s: String) {
        self.inc_by(s, 1);
    }

    fn inc_by(&mut self, s: String, amount: u64) {
        match self.labels.iter().position(|l| *l == s) {
            Some(i) => self.values[i] += amount,
            None => {
                self.labels.push(s);
                self.values.push(amount);
            }
        }
    }

    fn add_0(&mut self, s: String) {
        self.labels.push(s);
        self.values.push(0);
    }

    fn len(&self) -> usize {
        self.labels.len()
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Player {
    name: String,
    total_deaths: u64,
    exclusive_deaths: Vec<String>,
    unique_deaths: Chart,
    deaths_over_time: Chart,
}

impl Player {
    fn new(name: String) -> Self {
        Self {
            name,
            total_deaths: 0,
            exclusive_deaths: vec![],
            unique_deaths: Default::default(),
            deaths_over_time: Default::default(),
        }
    }
}

#[derive(Debug, Serialize)]
struct Year {
    number: i32,
    enabled: bool,
}

#[derive(Debug, Template, Default)]
#[template(path = "deaths/index.html")]
struct DeathsTemplate {
    years: Vec<Year>,
    no_year_enabled: bool,
    total_deaths: usize,
    players: Vec<Player>,
    unique_deaths: Chart,
    deaths_over_time: Chart,
}

#[derive(Debug, Deserialize)]
pub struct DeathQuery {
    year: Option<i32>,
}

pub async fn deaths(
    config: State<Arc<Config>>,
    Query(DeathQuery { year }): Query<DeathQuery>,
) -> Result<impl IntoResponse, Error> {
    let deaths = parse_logs(&config).await?;

    if deaths.is_empty() {
        return Ok(Html(DeathsTemplate::default().render()?));
    }

    let mut years = Vec::<Year>::new();
    let mut players = Vec::<Player>::new();

    let deaths = deaths
        .iter()
        .inspect(|d| {
            let d_year = d.timestamp.year();
            match years.binary_search_by_key(&d_year, |y| y.number) {
                Ok(_) => {}
                Err(i) => {
                    years.insert(
                        i,
                        Year {
                            number: d_year,
                            enabled: year.is_some_and(|y| y == d_year),
                        },
                    );
                }
            }
        })
        .filter(|d| year.is_none_or(|y| d.timestamp.year() == y))
        .collect::<Vec<_>>();

    for d in deaths.iter().rev() {
        let player = match players.iter_mut().find(|p| p.name == d.player) {
            Some(p) => p,
            None => {
                players.push(Player::new(d.player.clone()));
                players.last_mut().unwrap()
            }
        };
        player.total_deaths += 1;
    }

    let deaths_over_time_map = deaths.iter().fold(HashMap::new(), |mut acc, d| {
        let (count, players): &mut (u64, Vec<String>) = acc.entry(d.timestamp.date()).or_default();
        *count += 1;
        players.push(d.player.clone());
        acc
    });

    let deaths_over_time = {
        let max_date = deaths.last().unwrap().timestamp.date();
        let mut current_date = deaths.first().unwrap().timestamp.date();
        let mut deaths_over_time = Chart::default();
        while current_date <= max_date {
            let date_key = current_date.format("%d %b %Y").to_string();
            if let Some((dot, dead_players)) = deaths_over_time_map.get(&current_date) {
                for dp in dead_players {
                    players
                        .iter_mut()
                        .filter(|p| p.name == *dp)
                        .for_each(|p| p.deaths_over_time.inc(date_key.clone()));
                }
                deaths_over_time.inc_by(date_key, *dot);
            } else {
                for p in &mut players {
                    p.deaths_over_time.add_0(date_key.clone());
                }
                deaths_over_time.add_0(date_key);
            }
            current_date = current_date.checked_add_days(Days::new(1)).unwrap();
        }
        deaths_over_time
    };

    fn death_pie_chart<I>(i: I) -> Chart
    where
        I: Iterator,
        I::Item: AsRef<str>,
    {
        let mut unique_deaths = i
            .map(|d| d.as_ref().to_owned())
            .fold(HashMap::<String, u64>::new(), |mut acc, c| {
                *acc.entry(c).or_default() += 1;
                acc
            })
            .into_iter()
            .collect::<Vec<(_, _)>>();

        unique_deaths.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        Chart::new(unique_deaths)
    }

    let unique_deaths = death_pie_chart(deaths.iter().map(|d| &d.cause));
    for p in &mut players {
        p.unique_deaths = death_pie_chart(
            deaths
                .iter()
                .filter(|d| d.player == p.name)
                .map(|d| &d.cause),
        );
    }
    for p in &mut players {
        p.exclusive_deaths = p
            .unique_deaths
            .labels
            .iter()
            .filter(|pd| {
                deaths
                    .iter()
                    .filter(|d| d.cause == **pd)
                    .all(|d| d.player == p.name)
            })
            .cloned()
            .collect();
    }

    Ok(Html(
        DeathsTemplate {
            no_year_enabled: years.iter().all(|y| !y.enabled),
            years,
            total_deaths: deaths.len(),
            players,
            deaths_over_time,
            unique_deaths,
        }
        .render()?,
    ))
}
