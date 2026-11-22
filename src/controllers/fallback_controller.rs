/// controllers/fallback_controller.rs — Risposte per route e metodi inesistenti
///
/// Il router di axum risponde a una route sconosciuta (404) o a un metodo non
/// previsto (405) con un corpo vuoto: qui le due risposte vengono portate al
/// formato d'errore JSON dell'applicazione (`{"error","code","status"}`).
use axum::{
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};

use crate::errors::AppError;

/// Fallback del router: qualunque route non registrata → `404 NOT_FOUND`.
pub async fn not_found() -> AppError {
    AppError::NotFound("Risorsa non trovata".to_string())
}

/// Sostituisce le risposte 405 del router con `405 METHOD_NOT_ALLOWED` JSON,
/// mantenendo l'header `Allow` con i metodi ammessi.
pub async fn method_not_allowed(response: Response) -> Response {
    if response.status() != StatusCode::METHOD_NOT_ALLOWED {
        return response;
    }
    let mut json = AppError::coded(
        StatusCode::METHOD_NOT_ALLOWED,
        "METHOD_NOT_ALLOWED",
        "Metodo non consentito per questa risorsa",
    )
    .into_response();
    if let Some(allow) = response.headers().get(header::ALLOW) {
        json.headers_mut().insert(header::ALLOW, allow.clone());
    }
    json
}
