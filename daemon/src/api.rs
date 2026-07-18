use axum::{
    Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::{get, post},
};

use crate::meter::{MeterDescriptor, Meters};
use crate::sprinkler::{self, Sprinkler, ZoneStatus};

pub fn router(sprinkler: Sprinkler, meters: Meters) -> Router {
    let valve_routes = Router::new()
        .route("/api/zones", get(list_zones))
        .route("/api/zones/close-all", post(close_all))
        .route("/api/zones/{id}", get(get_zone))
        .route("/api/zones/{id}/open", post(open_zone))
        .route("/api/zones/{id}/close", post(close_zone))
        .with_state(sprinkler);

    let meter_routes = Router::new()
        .route("/api/meters", get(list_meters))
        .route("/api/meters/{id}", get(get_meter))
        .with_state(meters);

    Router::new()
        .route("/api/health", get(health))
        .merge(valve_routes)
        .merge(meter_routes)
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

// ── meters (discovery only — daemon is stateless) ──────────────────────────

async fn list_meters(State(m): State<Meters>) -> Json<Vec<MeterDescriptor>> {
    Json(m.all_descriptors())
}

async fn get_meter(
    State(m): State<Meters>,
    Path(id): Path<String>,
) -> Result<Json<MeterDescriptor>, impl IntoResponse> {
    m.get(&id)
        .map(Json)
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("Meter '{}' not found", id)))
}
