/// controllers/health_controller.rs — Endpoint di health check
///
/// Pubblico (nessun JWT): serve a load balancer, orchestratori e al client
/// per verificare che il server risponda.
use axum::Json;
use serde_json::{Value, json};

/// GET /api/health
///
/// Restituisce 200 OK con `{"status": "ok", "version": "X.Y.Z"}`: la versione è quella di
/// `Cargo.toml`, così il client può verificare di parlare con un'API abbastanza recente.
pub async fn health() -> Json<Value> {
    Json(json!({"status": "ok", "version": env!("CARGO_PKG_VERSION")}))
}
