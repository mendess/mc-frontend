mod melee_honor_run;

use askama::Template as _;
use axum::{
    Router,
    response::{Html, IntoResponse},
    routing::get,
};

pub fn router() -> anyhow::Result<Router<()>> {
    Ok(Router::new()
        .route("/", get(index))
        .route("/melee-honor-run", get(melee_honor_run::index)))
}

#[derive(askama::Template)]
#[template(path = "bg3/index.html")]
struct Index;

async fn index() -> impl IntoResponse {
    Html(Index.render().unwrap())
}
