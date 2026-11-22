/// controllers/user_controller.rs — Handler HTTP per l'utente autenticato
///
/// Endpoint protetti da JWT: ogni handler riceve `AuthUser`, ricavato dai
/// claims già validati dal middleware.
///
/// Endpoint esposti:
///   GET    /users/me → account + profilo
///   PATCH  /users/me → aggiornamento parziale
///   DELETE /users/me → eliminazione dell'account
use axum::{Json, extract::State, http::StatusCode};

use crate::AppState;
use crate::dto::user::{MeResponse, UpdateMeRequest};
use crate::errors::AppError;
use crate::extractors::AppJson;
use crate::middleware::AuthUser;
use crate::services::user_service;

/// GET /users/me
///
/// Restituisce i dati dell'utente autenticato insieme al suo profilo.
pub async fn get_me(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<MeResponse>, AppError> {
    let me = user_service::get_me(&state.pool, auth.id).await?;
    Ok(Json(me))
}

/// PATCH /users/me
///
/// Aggiornamento parziale: un campo assente resta invariato, `null` azzera
/// i campi nullable del profilo.
/// Esempio: {"weight_kg": 70.5, "max_hr": null}
pub async fn update_me(
    State(state): State<AppState>,
    auth: AuthUser,
    AppJson(req): AppJson<UpdateMeRequest>,
) -> Result<Json<MeResponse>, AppError> {
    let me = user_service::update_me(&state.pool, auth.id, req).await?;
    Ok(Json(me))
}

/// DELETE /users/me
///
/// Elimina l'account e i file dell'utente. Restituisce 204 No Content.
pub async fn delete_me(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<StatusCode, AppError> {
    user_service::delete_me(&state.pool, &state.upload_dir, auth.id).await?;

    // 204 No Content — operazione riuscita, nessun body da restituire
    Ok(StatusCode::NO_CONTENT)
}
