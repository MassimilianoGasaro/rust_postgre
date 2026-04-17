/// errors.rs — Gestione centralizzata degli errori
///
/// In Rust non esistono eccezioni: gli errori sono valori.
/// Questo file definisce UN tipo di errore per tutta l'app.
/// Grazie a `thiserror` possiamo derivare automaticamente Display e Error.
/// Grazie all'implementazione di `IntoResponse` di axum, possiamo
/// restituire questi errori direttamente dagli handler HTTP e axum
/// li convertirà in risposte JSON con il codice HTTP corretto.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use thiserror::Error;

/// Tutti gli errori possibili dell'applicazione.
/// `#[derive(Error)]` genera automaticamente l'implementazione di std::error::Error.
/// `#[error("...")]` definisce il messaggio dell'errore.
#[derive(Debug, Error)]
pub enum AppError {
    #[error("Errore del database: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Non autorizzato: {0}")]
    Unauthorized(String),

    #[error("Non trovato: {0}")]
    NotFound(String),

    #[error("Dati non validi: {0}")]
    BadRequest(String),

    #[error("Conflitto: {0}")]
    Conflict(String),

    #[error("Errore interno: {0}")]
    Internal(String),

    #[error("Errore JWT: {0}")]
    Jwt(#[from] jsonwebtoken::errors::Error),

    #[error("Errore hash password: {0}")]
    Bcrypt(#[from] bcrypt::BcryptError),
}

/// Implementando IntoResponse, axum può convertire automaticamente
/// AppError in una risposta HTTP quando viene restituito da un handler.
/// Questo ci permette di scrivere: `return Err(AppError::NotFound(...))`
/// negli handler e ottenere automaticamente un 404 JSON.
impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            AppError::Database(e) => {
                tracing::error!("Errore DB: {}", e);
                (StatusCode::INTERNAL_SERVER_ERROR, "Errore interno del server".to_string())
            }
            AppError::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, msg.clone()),
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, msg.clone()),
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            AppError::Conflict(msg) => (StatusCode::CONFLICT, msg.clone()),
            AppError::Internal(msg) => {
                tracing::error!("Errore interno: {}", msg);
                (StatusCode::INTERNAL_SERVER_ERROR, msg.clone())
            }
            AppError::Jwt(e) => {
                tracing::warn!("JWT non valido: {}", e);
                (StatusCode::UNAUTHORIZED, "Token non valido o scaduto".to_string())
            }
            AppError::Bcrypt(e) => {
                tracing::error!("Errore bcrypt: {}", e);
                (StatusCode::INTERNAL_SERVER_ERROR, "Errore interno del server".to_string())
            }
        };

        // Risposta JSON standardizzata per tutti gli errori
        let body = Json(json!({
            "error": message,
            "status": status.as_u16()
        }));

        (status, body).into_response()
    }
}

/// Alias per Result con AppError — evita di scrivere Result<T, AppError> ovunque
pub type AppResult<T> = Result<T, AppError>;
