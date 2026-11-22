/// controllers/auth_controller.rs — Handler HTTP per l'autenticazione
///
/// I controller sono il "confine" tra HTTP e la logica di business.
/// Le loro responsabilità sono SOLO:
/// 1. Estrarre i dati dalla request (body JSON, path params, headers...)
/// 2. Chiamare il service appropriato
/// 3. Restituire la risposta HTTP corretta
///
/// NON contengono logica di business — quella sta nei services.
/// NON fanno query al DB — le fa il repository.

use axum::{extract::State, http::StatusCode, Json};

use crate::dto::auth::{LoginRequest, RegisterRequest};
use crate::errors::AppError;
use crate::extractors::AppJson;
use crate::services::auth_service;

/// AppState condiviso tra tutti gli handler tramite Arc.
/// Axum lo inietta automaticamente con `State(state)`.
use crate::AppState;

/// POST /api/auth/register
///
/// Registra un nuovo utente.
/// - `State(state)`: axum inietta lo stato condiviso (pool, config)
/// - `AppJson(req)`: deserializza il body JSON in `RegisterRequest`; se non è valido → 400 VALIDATION_FAILED
///
/// Restituisce 201 Created con il token JWT e le info utente.
pub async fn register(
    State(state): State<AppState>,
    AppJson(req): AppJson<RegisterRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), AppError> {
    // Delega tutto al service — il controller non sa nulla di bcrypt o JWT
    let response = auth_service::register(
        &state.pool,
        &state.jwt_secret,
        state.jwt_expiration_hours,
        req,
    )
    .await?;

    // 201 Created è più corretto di 200 OK per una nuova risorsa
    Ok((StatusCode::CREATED, Json(serde_json::json!(response))))
}

/// POST /api/auth/login
///
/// Autentica un utente esistente.
/// Restituisce 200 OK con il token JWT.
pub async fn login(
    State(state): State<AppState>,
    AppJson(req): AppJson<LoginRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let response = auth_service::login(
        &state.pool,
        &state.jwt_secret,
        state.jwt_expiration_hours,
        req,
    )
    .await?;

    Ok(Json(serde_json::json!(response)))
}
