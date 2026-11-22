/// extractors.rs — Estrattori axum personalizzati
///
/// `AppJson<T>` sostituisce `axum::Json<T>` in ingresso: quando il body
/// non è deserializzabile (JSON malformato, campo mancante, ruolo
/// sconosciuto...) risponde con il nostro formato d'errore
/// `400 VALIDATION_FAILED` invece del 422 testuale di default di axum.

use axum::{
    async_trait,
    extract::{rejection::JsonRejection, FromRequest, Request},
    http::StatusCode,
    Json,
};
use serde::de::DeserializeOwned;

use crate::errors::AppError;

/// Body JSON deserializzato, con errori nel formato applicativo.
pub struct AppJson<T>(pub T);

#[async_trait]
impl<T, S> FromRequest<S> for AppJson<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        match Json::<T>::from_request(req, state).await {
            Ok(Json(value)) => Ok(AppJson(value)),
            Err(rejection) => {
                // Errori di lettura del body (es. troppo grande) mantengono il loro status;
                // tutto il resto è un problema di contenuto → 400.
                let status = match rejection {
                    JsonRejection::BytesRejection(_) => rejection.status(),
                    _ => StatusCode::BAD_REQUEST,
                };
                Err(AppError::coded(
                    status,
                    "VALIDATION_FAILED",
                    rejection.body_text(),
                ))
            }
        }
    }
}
