use askama::Template;
use axum::{
    extract::Query,
    response::{Html, IntoResponse},
};
use serde::{Deserialize, Serialize};

const SPELLS: &str = include_str!("./spell-list.json");

#[derive(Serialize, Deserialize, Clone)]
struct Spell {
    name: String,
    url: String,
    icon_url: String,
    level: u8,
    #[serde(default)]
    range: f32,
    damage: Option<String>,
    damage_type: Option<String>,
    #[serde(default)]
    banned: bool,
}

#[derive(askama::Template)]
#[template(path = "bg3/melee_honor_run.html")]
struct SpellTemplate {
    spells: Vec<Vec<Spell>>,
    banned: bool,
}

#[derive(Deserialize)]
pub struct BannedQuery {
    #[serde(default)]
    banned: bool,
}

pub async fn index(Query(BannedQuery { banned }): Query<BannedQuery>) -> impl IntoResponse {
    Html(
        SpellTemplate {
            spells: serde_json::from_str::<Vec<Spell>>(SPELLS)
                .unwrap()
                .into_iter()
                .map(|mut s| {
                    s.banned = !((s.range < 10.0 || s.damage.is_none())
                        && s.name != "Lightning Bolt"
                        && s.name != "Produce Flame"
                        && s.name != "Sunbeam");
                    s
                })
                .filter(|s| banned || !s.banned)
                .fold(vec![vec![]; 7], |mut map, s| {
                    map[s.level as usize].push(s);
                    map
                }),
            banned: !banned,
        }
        .render()
        .unwrap(),
    )
}
