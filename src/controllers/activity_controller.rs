/// controllers/activity_controller.rs — Handler HTTP per le attività
///
/// Endpoint protetti da JWT:
///   GET    /activities      → elenco paginato con filtri (proprio o di un atleta collegato)
///   DELETE /activities/:id  → elimina una propria attività e il file originale (204)
use axum::{Json, extract::State, http::StatusCode};

use crate::AppState;
use crate::dto::activity::{ActivityDto, ActivityPageDto, ListActivitiesQuery};
use crate::errors::AppError;
use crate::extractors::{AppPath, AppQuery};
use crate::middleware::AuthUser;
use crate::parsing::Sport;
use crate::repositories::{activity_repository, user_repository};
use crate::services::access_service;
use crate::storage;

const DEFAULT_PER_PAGE: i64 = 20;
const MAX_PER_PAGE: i64 = 100;

fn validation_error(message: &str) -> AppError {
    AppError::coded(StatusCode::BAD_REQUEST, "VALIDATION_FAILED", message)
}

/// GET /activities?from=&to=&sport=&page=&per_page=&athlete_id=
pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
    AppQuery(query): AppQuery<ListActivitiesQuery>,
) -> Result<Json<ActivityPageDto>, AppError> {
    let page = query.page.unwrap_or(1);
    let per_page = query.per_page.unwrap_or(DEFAULT_PER_PAGE);
    if page < 1 {
        return Err(validation_error("page deve essere almeno 1"));
    }
    if !(1..=MAX_PER_PAGE).contains(&per_page) {
        return Err(validation_error(
            "per_page deve essere compreso tra 1 e 100",
        ));
    }
    if let (Some(from), Some(to)) = (query.from, query.to) {
        if from > to {
            return Err(validation_error("from non può essere successivo a to"));
        }
    }
    if let Some(sport) = query.sport.as_deref() {
        if Sport::from_str_opt(sport).is_none() {
            return Err(validation_error("sport non valido"));
        }
    }

    let target = access_service::resolve_target(&state.pool, &auth, query.athlete_id).await?;
    // Le date `from`/`to` sono giorni nel fuso dell'atleta di cui si leggono i dati
    let timezone = user_repository::timezone_of(&state.pool, target).await?;
    let filter = activity_repository::ActivityFilter {
        from: query.from,
        to: query.to,
        sport: query.sport.as_deref(),
        timezone: &timezone,
    };

    let offset = (page - 1).saturating_mul(per_page);
    let rows = activity_repository::list(&state.pool, target, &filter, per_page, offset).await?;
    let total = activity_repository::count(&state.pool, target, &filter).await?;

    Ok(Json(ActivityPageDto {
        items: rows.into_iter().map(ActivityDto::from).collect(),
        total,
        page,
        per_page,
    }))
}

/// DELETE /activities/:id
///
/// Solo il proprietario: per chiunque altro (coach compreso) l'attività "non esiste" (404).
/// Prima si elimina la riga, poi il file: un errore sul file viene solo loggato,
/// l'attività è comunque eliminata.
pub async fn delete(
    State(state): State<AppState>,
    auth: AuthUser,
    AppPath(id): AppPath<i64>,
) -> Result<StatusCode, AppError> {
    let file_path = activity_repository::delete_owned(&state.pool, id, auth.id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Attività {id} non trovata")))?;

    // Il file si rimuove solo se nessun'altra attività lo referenzia
    match activity_repository::file_path_in_use(&state.pool, &file_path).await {
        Ok(false) => {
            if let Err(e) = storage::remove(&state.upload_dir, &file_path).await {
                tracing::warn!("Rimozione del file {file_path} fallita: {e}");
            }
        }
        Ok(true) => {}
        Err(e) => tracing::warn!("Controllo riferimenti al file {file_path} fallito: {e}"),
    }

    Ok(StatusCode::NO_CONTENT)
}
