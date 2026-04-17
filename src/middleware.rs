/// middleware.rs — Middleware di autenticazione JWT
///
/// In Axum i middleware sono funzioni che wrappano gli handler.
/// Questo middleware:
/// 1. Legge l'header `Authorization: Bearer <token>`
/// 2. Valida il JWT tramite auth_service::validate_jwt
/// 3. Se valido: inietta i Claims nell'handler via Extension
/// 4. Se non valido: blocca la request con 401 Unauthorized
///
/// Gli handler protetti ricevono i claims così:
///   `Extension(claims): Extension<Claims>`
/// e sanno chi è l'utente autenticato senza fare query al DB.

use axum::{
    extract::{Request, State},
    http::header::AUTHORIZATION,
    middleware::Next,
    response::Response,
};

use crate::errors::AppError;
use crate::services::auth_service;
use crate::AppState;

/// Middleware che verifica il JWT su ogni request alle route protette.
///
/// Axum lo registra con `.route_layer(axum::middleware::from_fn_with_state(...))`
/// applicandolo solo alle route che vogliamo proteggere.
pub async fn auth_middleware(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Result<Response, AppError> {
    // Legge l'header Authorization
    let auth_header = req
        .headers()
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::Unauthorized("Header Authorization mancante".to_string()))?;

    // Il formato atteso è "Bearer <token>"
    let token = auth_header
        .strip_prefix("Bearer ")
        .ok_or_else(|| AppError::Unauthorized("Formato token non valido. Usa: Bearer <token>".to_string()))?;

    // Valida il JWT — se scaduto o manomesso → AppError::Unauthorized
    let claims = auth_service::validate_jwt(token, &state.jwt_secret)?;

    // Inserisce i claims nella request come Extension
    // Gli handler possono estrarli con: Extension(claims): Extension<Claims>
    req.extensions_mut().insert(claims);

    // Passa la request all'handler successivo
    Ok(next.run(req).await)
}
