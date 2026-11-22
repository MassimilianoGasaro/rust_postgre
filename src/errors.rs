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

    #[error("Accesso negato: {0}")]
    Forbidden(String),

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

    /// Errore con codice applicativo specifico (es. `EMAIL_TAKEN`).
    /// Il client usa `code` per distinguere i casi, `message` è solo descrittivo.
    #[error("{message}")]
    Coded {
        status: StatusCode,
        code: &'static str,
        message: String,
    },
}

impl AppError {
    /// Costruisce un errore con codice applicativo specifico.
    pub fn coded(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        AppError::Coded {
            status,
            code,
            message: message.into(),
        }
    }
}

/// Implementando IntoResponse, axum può convertire automaticamente
/// AppError in una risposta HTTP quando viene restituito da un handler.
/// Questo ci permette di scrivere: `return Err(AppError::NotFound(...))`
/// negli handler e ottenere automaticamente un 404 JSON.
impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, code, message) = match &self {
            AppError::Database(e) => {
                tracing::error!("Errore DB: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "INTERNAL",
                    "Errore interno del server".to_string(),
                )
            }
            AppError::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, "UNAUTHORIZED", msg.clone()),
            AppError::Forbidden(msg) => (StatusCode::FORBIDDEN, "FORBIDDEN", msg.clone()),
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, "NOT_FOUND", msg.clone()),
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, "BAD_REQUEST", msg.clone()),
            AppError::Conflict(msg) => (StatusCode::CONFLICT, "CONFLICT", msg.clone()),
            AppError::Internal(msg) => {
                tracing::error!("Errore interno: {}", msg);
                (StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL", msg.clone())
            }
            AppError::Jwt(e) => {
                tracing::warn!("JWT non valido: {}", e);
                (
                    StatusCode::UNAUTHORIZED,
                    "UNAUTHORIZED",
                    "Token non valido o scaduto".to_string(),
                )
            }
            AppError::Bcrypt(e) => {
                tracing::error!("Errore bcrypt: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "INTERNAL",
                    "Errore interno del server".to_string(),
                )
            }
            AppError::Coded {
                status,
                code,
                message,
            } => (*status, *code, message.clone()),
        };

        // Risposta JSON standardizzata per tutti gli errori:
        // {"error": "<messaggio>", "code": "<CODICE>", "status": <n>}
        let body = Json(json!({
            "error": message,
            "code": code,
            "status": status.as_u16()
        }));

        (status, body).into_response()
    }
}

/// Alias per Result con AppError — evita di scrivere Result<T, AppError> ovunque
pub type AppResult<T> = Result<T, AppError>;

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    /// Converte un errore in (status, body JSON).
    async fn render(err: AppError) -> (StatusCode, serde_json::Value) {
        let response = err.into_response();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    #[tokio::test]
    async fn coded_error_has_custom_code() {
        let (status, body) =
            render(AppError::coded(StatusCode::CONFLICT, "EMAIL_TAKEN", "x")).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(
            body,
            json!({"error": "x", "code": "EMAIL_TAKEN", "status": 409})
        );
    }

    #[tokio::test]
    async fn generic_errors_have_generic_codes() {
        let (status, body) = render(AppError::NotFound("y".into())).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["code"], "NOT_FOUND");

        let (status, body) = render(AppError::Forbidden("f".into())).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["code"], "FORBIDDEN");

        let (_, body) = render(AppError::Unauthorized("u".into())).await;
        assert_eq!(body["code"], "UNAUTHORIZED");
        let (_, body) = render(AppError::BadRequest("b".into())).await;
        assert_eq!(body["code"], "BAD_REQUEST");
        let (_, body) = render(AppError::Conflict("c".into())).await;
        assert_eq!(body["code"], "CONFLICT");
        let (status, body) = render(AppError::Internal("i".into())).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["code"], "INTERNAL");
    }
}
