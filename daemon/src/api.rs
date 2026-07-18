use axum::{
    Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::{get, post},
};

use crate::sprinkler::{self, Sprinkler, ZoneStatus};
use crate::switch::{self, Switches, SwitchStatus};

pub fn router(sprinkler: Sprinkler, switches: Switches) -> Router {
    // Sub-routers because the two states have different types.
    let zone_routes = Router::new()
        .route("/api/zones", get(list_zones))
        .route("/api/zones/close-all", post(close_all))
        .route("/api/zones/{id}", get(get_zone))
        .route("/api/zones/{id}/open", post(open_zone))
        .route("/api/zones/{id}/close", post(close_zone))
        .with_state(sprinkler);

    let switch_routes = Router::new()
        .route("/api/switches", get(list_switches))
        .route("/api/switches/{id}", get(get_switch))
        .with_state(switches);

    Router::new()
        .route("/api/health", get(health))
        .merge(zone_routes)
        .merge(switch_routes)
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

// ── switches (read-only inputs) ─────────────────────────────────────────────

async fn list_switches(State(s): State<Switches>) -> Json<Vec<SwitchStatus>> {
    Json(switch::get_all(&s))
}

async fn get_switch(
    State(s): State<Switches>,
    Path(id): Path<String>,
) -> Result<Json<SwitchStatus>, impl IntoResponse> {
    switch::get_switch(&s, &id)
        .map(Json)
        .map_err(|e| (StatusCode::NOT_FOUND, e))
}
