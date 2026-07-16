use axum::{
    extract::{Query, Request},
    response::IntoResponse,
};
use serde::Deserialize;
use tower::ServiceExt;
use tower_http::services::ServeFile;

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum VersionKind {
    Bg3,
    Mc,
}

#[derive(Deserialize)]
pub struct Version {
    #[serde(default)]
    v: Option<VersionKind>,
}

pub async fn serve(Query(Version { v }): Query<Version>, request: Request) -> impl IntoResponse {
    let ico = match v {
        Some(VersionKind::Bg3) => ServeFile::new("./assets/bg3-favicon.ico"),
        Some(VersionKind::Mc) => ServeFile::new("./assets/minecraft-favicon.ico"),
        _ => ServeFile::new("./assets/favicon.ico"),
    };

    ico.oneshot(request).await
}
