/// controllers/user_controller.rs — Handler HTTP per la gestione utenti
///
/// Endpoint protetti da JWT: ogni handler riceve `Extension(claims)`
/// con le info dell'utente autenticato già validate dal middleware.
///
/// Endpoint esposti:
///   GET    /users              → lista paginata
///   GET    /users/:id          → singolo utente
///   PATCH  /users/:id          → aggiornamento parziale
///   DELETE /users/:id          → eliminazione

use axum::{
    extract::{Extension, Path, Query, State},
    http::StatusCode,
    Json,
};

use crate::dto::auth::Claims;
use crate::dto::user::{PaginationQuery, UpdateUserRequest};
use crate::errors::AppError;
use crate::services::user_service;
use crate::AppState;

/// GET /users?page=1&per_page=20
///
/// Restituisce la lista paginata degli utenti.
/// - `Query(pagination)`: axum legge automaticamente i query params
/// - `Extension(claims)`: iniettato dal middleware JWT — sappiamo chi chiede
pub async fn list(
    State(state): State<AppState>,
    Extension(_claims): Extension<Claims>, // _ perché non lo usiamo qui
    Query(pagination): Query<PaginationQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    let result = user_service::get_all_users(&state.pool, pagination).await?;
    Ok(Json(serde_json::json!(result)))
}

/// GET /users/:id
///
/// Restituisce i dati di un singolo utente.
/// - `Path(id)`: axum estrae l'UUID dal path `/users/:id`
pub async fn get_by_id(
    State(state): State<AppState>,
    Extension(_claims): Extension<Claims>,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, AppError> {
    let user = user_service::get_user_by_id(&state.pool, id).await?;
    Ok(Json(serde_json::json!(user)))
}

/// PATCH /users/:id
///
/// Aggiornamento parziale di un utente.
/// Il body JSON può contenere uno o più campi (email, username, password).
/// Esempio: {"username": "nuovo_nome"} — aggiorna solo username.
pub async fn update(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<i64>,
    Json(req): Json<UpdateUserRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Opzionale: verifica che l'utente stia modificando solo sé stesso
    // (rimuovi questo check se gli admin devono poter modificare tutti)
    if claims.sub != id {
        return Err(AppError::Unauthorized(
            "Non puoi modificare un altro utente".to_string(),
        ));
    }

    let updated = user_service::update_user(&state.pool, id, req).await?;
    Ok(Json(serde_json::json!(updated)))
}

/// DELETE /users/:id
///
/// Elimina un utente. Restituisce 204 No Content (nessun body).
pub async fn delete(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<i64>,
) -> Result<StatusCode, AppError> {
    // Anche qui: solo l'utente stesso può eliminarsi
    if claims.sub != id {
        return Err(AppError::Unauthorized(
            "Non puoi eliminare un altro utente".to_string(),
        ));
    }

    user_service::delete_user(&state.pool, id).await?;

    // 204 No Content — operazione riuscita, nessun body da restituire
    Ok(StatusCode::NO_CONTENT)
}
