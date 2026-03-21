use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::{get, post},
    Router,
};

use crate::sprinkler::{self, Sprinkler, ZoneStatus};

pub fn router(sprinkler: Sprinkler) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/zones", get(list_zones))
        .route("/api/zones/close-all", post(close_all))
        .route("/api/zones/{id}", get(get_zone))
        .route("/api/zones/{id}/open", post(open_zone))
        .route("/api/zones/{id}/close", post(close_zone))
        .with_state(sprinkler)
}

// ---------------------------------------------------------------------------

async fn health() -> &'static str {
    "ok"
}

async fn list_zones(State(s): State<Sprinkler>) -> Json<Vec<ZoneStatus>> {
    Json(sprinkler::get_all(&s).await)
}

async fn get_zone(
    State(s): State<Sprinkler>,
    Path(id): Path<String>,
) -> Result<Json<ZoneStatus>, impl IntoResponse> {
    sprinkler::get_zone(&s, &id)
        .await
        .map(Json)
        .map_err(|e| (StatusCode::NOT_FOUND, e))
}

async fn open_zone(
    State(s): State<Sprinkler>,
    Path(id): Path<String>,
) -> Result<Json<ZoneStatus>, impl IntoResponse> {
    sprinkler::open_zone(&s, &id)
        .await
        .map(Json)
        .map_err(|e| (StatusCode::BAD_REQUEST, e))
}

async fn close_zone(
    State(s): State<Sprinkler>,
    Path(id): Path<String>,
) -> Result<Json<ZoneStatus>, impl IntoResponse> {
    sprinkler::close_zone(&s, &id)
        .await
        .map(Json)
        .map_err(|e| (StatusCode::BAD_REQUEST, e))
}

async fn close_all(State(s): State<Sprinkler>) -> Json<Vec<ZoneStatus>> {
    Json(sprinkler::close_all(&s).await)
}
