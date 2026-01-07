use crate::{Config, Error};
use askama::Template;
use axum::{
    extract::State,
    response::{Html, IntoResponse},
};
use chrono::{Days, NaiveDateTime};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    sync::Arc,
};

#[derive(Debug, Serialize, Deserialize)]
struct Deaths {
    timestamp: i64,
    #[serde(skip)]
    timestamp_chrono: NaiveDateTime,
    player: String,
    cause: String,
    date_key: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct Player {
    name: String,
    total_deaths: u64,
    unique_deaths: Vec<String>,
    exclusive_deaths: Vec<usize>,
}

#[derive(Debug, Template, Default)]
#[template(path = "deaths/index.html")]
struct DeathsTemplate {
    all_deaths: Vec<Deaths>,
    unique_deaths: usize,
    players: Vec<Player>,
    all_dates: Vec<String>,
}

pub async fn deaths(config: State<Arc<Config>>) -> Result<impl IntoResponse, Error> {
    fn date_key_format(d: &NaiveDateTime) -> String {
        d.format("%d %b %Y").to_string()
    }

    let raw_data = serde_json::from_reader::<_, Vec<(String, String, String)>>(File::open(
        config.backups_dir.join("deaths.json"),
    )?)?;

    if raw_data.is_empty() {
        return Ok(Html(DeathsTemplate::default().render()?));
    }

    let mut players = Vec::<Player>::new();
    let mut deaths = raw_data
        .clone()
        .into_iter()
        .rev()
        .map(|(date, player, cause)| {
            let player = match players.iter_mut().find(|p| p.name == player) {
                Some(p) => p,
                None => {
                    players.push(Player {
                        name: player,
                        total_deaths: 0,
                        unique_deaths: vec![],
                        exclusive_deaths: vec![],
                    });
                    players.last_mut().unwrap()
                }
            };
            if !player.unique_deaths.contains(&cause) {
                player.unique_deaths.push(cause.clone());
            }
            player.total_deaths += 1;
            let date = NaiveDateTime::parse_from_str(&date, "%d%b%Y %H:%M:%S%.f").unwrap();
            let timestamp = date.and_utc().timestamp_millis();
            let date_key = date_key_format(&date);
            Deaths {
                timestamp,
                timestamp_chrono: date,
                player: player.name.clone(),
                cause,
                date_key,
            }
        })
        .collect::<Vec<_>>();
    let mut map = HashMap::new();
    for (pi, p) in players.iter().enumerate() {
        for (di, death) in p.unique_deaths.iter().enumerate() {
            if players
                .iter()
                .flat_map(|p| &p.unique_deaths)
                .all(|d| *d != *death)
            {
                map.entry(pi).or_insert_with(Vec::new).push(di);
            }
        }
    }
    for (player, exclusive_deaths) in map {
        players[player].exclusive_deaths = exclusive_deaths;
    }
    deaths.sort_by_key(|d| d.timestamp);
    let mut all_dates = Vec::new();
    let min_date = deaths.first().unwrap();
    let max_date = deaths.last().unwrap();
    let mut current_date = min_date.timestamp_chrono;
    while current_date < max_date.timestamp_chrono {
        all_dates.push(date_key_format(&current_date));
        current_date = current_date.checked_add_days(Days::new(1)).unwrap();
    }

    Ok(Html(
        DeathsTemplate {
            unique_deaths: deaths
                .iter()
                .map(|d| &d.cause)
                .collect::<HashSet<_>>()
                .len(),
            all_deaths: deaths,
            players,
            all_dates,
        }
        .render()?,
    ))
}
