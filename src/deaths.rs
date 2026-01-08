use crate::{Config, Error};
use askama::Template;
use axum::{
    extract::{Query, State},
    response::{Html, IntoResponse},
};
use chrono::{Datelike, Days, NaiveDateTime};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs::File, sync::Arc};

#[derive(Debug, Serialize, Deserialize)]
struct Deaths {
    #[serde(skip)]
    timestamp: NaiveDateTime,
    player: String,
    cause: String,
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

#[derive(Debug, Template, Default)]
#[template(path = "deaths/index.html")]
struct DeathsTemplate {
    years: Vec<i32>,
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
        .map(|(date, player, cause)| Deaths {
            timestamp: NaiveDateTime::parse_from_str(&date, "%d%b%Y %H:%M:%S%.f").unwrap(),
            player,
            cause,
        })
        .filter(|d| year.is_none_or(|y| d.timestamp.year() == y))
        .inspect(|d| {
            let player = match players.iter_mut().find(|p| p.name == d.player) {
                Some(p) => p,
                None => {
                    players.push(Player::new(d.player.clone()));
                    players.last_mut().unwrap()
                }
            };
            player.total_deaths += 1;
        })
        .collect::<Vec<_>>();
    deaths.sort_by_key(|d| d.timestamp);

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
    let mut years = deaths
        .iter()
        .map(|d| d.timestamp.year())
        .collect::<Vec<_>>();

    years.sort();
    years.dedup();

    Ok(Html(
        DeathsTemplate {
            years,
            total_deaths: deaths.len(),
            players,
            deaths_over_time,
            unique_deaths,
        }
        .render()?,
    ))
}
