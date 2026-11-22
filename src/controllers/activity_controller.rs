/// controllers/activity_controller.rs — Handler HTTP per le attività
///
/// Endpoint protetti da JWT:
///   GET    /activities      → elenco paginato con filtri (proprio o di un atleta collegato)
///   DELETE /activities/:id  → elimina una propria attività e il file originale (204)
use axum::{Json, extract::State, http::StatusCode};

use crate::AppState;
use crate::dto::activity::{ActivityPageDto, ListActivitiesQuery};
use crate::errors::AppError;
use crate::extractors::{AppPath, AppQuery};
use crate::middleware::AuthUser;
use crate::services::activity_service;

/// GET /activities?from=&to=&sport=&page=&per_page=&athlete_id=
pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
    AppQuery(query): AppQuery<ListActivitiesQuery>,
) -> Result<Json<ActivityPageDto>, AppError> {
    Ok(Json(activity_service::list(&state.pool, &auth, query).await?))
}

/// DELETE /activities/:id
///
/// Solo il proprietario: per chiunque altro l'attività "non esiste" (404).
pub async fn delete(
    State(state): State<AppState>,
    auth: AuthUser,
    AppPath(id): AppPath<i64>,
) -> Result<StatusCode, AppError> {
    activity_service::delete(&state.pool, &state.upload_dir, &auth, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
