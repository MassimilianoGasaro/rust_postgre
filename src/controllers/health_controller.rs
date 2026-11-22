/// controllers/health_controller.rs — Endpoint di health check
///
/// Pubblico (nessun JWT): serve a load balancer, orchestratori e al client
/// per verificare che il server risponda.
use axum::Json;
use serde_json::{Value, json};

/// GET /api/health
///
/// Restituisce 200 OK con `{"status": "ok"}`.
pub async fn health() -> Json<Value> {
    Json(json!({"status": "ok"}))
}
