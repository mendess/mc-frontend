use crate::{Config, Error, logs};
use askama::Template;
use axum::{
    extract::{Query, State},
    response::{Html, IntoResponse},
};
use chrono::{Datelike, Days, NaiveDate, NaiveDateTime, NaiveTime};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, future::ready, sync::Arc};

macro_rules! ts {
    ($d:literal Jan $y:literal $h:literal : $mm:literal : $s:literal : $ms:literal) => {
        ts!($d 1 $y $h : $mm : $s : $ms)
    };
    ($d:literal Jun $y:literal $h:literal : $mm:literal : $s:literal : $ms:literal) => {
        ts!($d 6 $y $h : $mm : $s : $ms)
    };
    ($d:literal $m:literal $y:literal $h:literal : $mm:literal : $s:literal : $ms:literal) => {
        #[allow(clippy::zero_prefixed_literal)]
        NaiveDateTime::new(
            NaiveDate::from_ymd_opt($y, $m, $d).unwrap(),
            NaiveTime::from_hms_milli_opt($h, $mm, $s, $ms).unwrap(),
        )
    };
}

const IGNORED_TIMESTAMPS: &[NaiveDateTime] = &[
    ts!(6 Jun 2025 15:42:05:682),
    ts!(8 Jun 2025 18:40:17:329),
    ts!(5 Jan 2026 01:49:16:370),
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
    let deaths = logs::parse_logs(&config)
        .await?
        .filter(|line| {
            ready(
                !IGNORED_MESSAGES
                    .iter()
                    .any(|&msg| line.message.contains(msg)),
            )
        })
        .inspect(|l| {
            if l.message.contains("Husk") {
                println!("{l:?}")
            }
        })
        .filter(|line| ready(!IGNORED_TIMESTAMPS.contains(&line.timestamp)))
        .collect::<Vec<_>>()
        .await;

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

    let unique_deaths = death_pie_chart(deaths.iter().map(|d| &d.message));
    for p in &mut players {
        p.unique_deaths = death_pie_chart(
            deaths
                .iter()
                .filter(|d| d.player == p.name)
                .map(|d| &d.message),
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
                    .filter(|d| d.message == **pd)
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
