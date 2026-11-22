/// extractors.rs — Estrattori axum personalizzati
///
/// `AppJson<T>` sostituisce `axum::Json<T>` in ingresso: quando il body
/// non è deserializzabile (JSON malformato, campo mancante, ruolo
/// sconosciuto...) risponde con il nostro formato d'errore
/// `400 VALIDATION_FAILED` invece del 422 testuale di default di axum.
///
/// `AppPath<T>` e `AppQuery<T>` fanno lo stesso per i parametri di path e di
/// query string (es. `/coach-links/abc/accept`, `?status=a&status=b`): il
/// rifiuto di default di axum è un 400 in testo semplice, non JSON.
use axum::{
    Json, async_trait,
    extract::rejection::JsonRejection,
    extract::{FromRequest, FromRequestParts, Path, Query, Request},
    http::{StatusCode, request::Parts},
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

/// Parametri di path deserializzati, con errori nel formato applicativo.
pub struct AppPath<T>(pub T);

#[async_trait]
impl<T, S> FromRequestParts<S> for AppPath<T>
where
    T: DeserializeOwned + Send,
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        Path::<T>::from_request_parts(parts, state)
            .await
            .map(|Path(value)| AppPath(value))
            .map_err(|rejection| {
                AppError::coded(
                    StatusCode::BAD_REQUEST,
                    "VALIDATION_FAILED",
                    rejection.body_text(),
                )
            })
    }
}

/// Query string deserializzata, con errori nel formato applicativo.
pub struct AppQuery<T>(pub T);

#[async_trait]
impl<T, S> FromRequestParts<S> for AppQuery<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        Query::<T>::from_request_parts(parts, state)
            .await
            .map(|Query(value)| AppQuery(value))
            .map_err(|rejection| {
                AppError::coded(
                    StatusCode::BAD_REQUEST,
                    "VALIDATION_FAILED",
                    rejection.body_text(),
                )
            })
    }
}
